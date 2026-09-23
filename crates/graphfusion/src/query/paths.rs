//! Path state, native DataFusion recursion, and endpoint-partitioned selection.
use super::{
    execution,
    graph::{self, Bindings, ElementKind},
};
use crate::{
    catalog::ObjectId,
    gql as ast,
    graph::{GraphData, FROM, ID, TO},
    Error, Result, SessionState, Value,
};
use datafusion::{
    arrow::datatypes::DataType,
    common::{Column, ScalarValue},
    datasource::{cte_worktable::CteWorkTable, provider_as_source},
    functions::core::expr_fn::{get_field, named_struct},
    functions_nested::expr_fn::{
        array_append, array_distinct, array_element, array_has, array_length, array_slice,
        make_array,
    },
    logical_expr::{cast, lit, Expr, ExprFunctionExt, ExprSchemable, JoinType, LogicalPlanBuilder},
};
use std::sync::Arc;

const NODES: &str = "__gql_path_nodes";
const EDGES: &str = "__gql_path_edges";
const GRAPH: &str = "__gql_path_graph";

#[derive(Clone)]
pub(super) struct State {
    pub nodes: Expr,
    pub edges: Expr,
}
impl State {
    pub fn new(node: Expr) -> Self {
        Self {
            nodes: make_array(vec![node]),
            edges: lit(ScalarValue::List(ScalarValue::new_list(
                &[],
                &DataType::UInt64,
                true,
            ))),
        }
    }
    pub fn append(&self, edge: Expr, node: Expr) -> Self {
        Self {
            nodes: array_append(self.nodes.clone(), node),
            edges: array_append(self.edges.clone(), edge),
        }
    }
    pub fn start(&self) -> Expr {
        array_element(self.nodes.clone(), lit(1_i64))
    }
    pub fn end(&self) -> Expr {
        array_element(self.nodes.clone(), lit(-1_i64))
    }
    pub fn length(&self) -> Expr {
        cast(array_length(self.edges.clone()), DataType::Int64)
    }
    pub fn value(&self, graph: ObjectId) -> Expr {
        named_struct(vec![
            lit(GRAPH),
            lit(graph),
            lit(NODES),
            self.nodes.clone(),
            lit(EDGES),
            self.edges.clone(),
        ])
    }
    pub fn valid(&self, mode: ast::PathMode, different: bool) -> Expr {
        let mut result = if different || mode == ast::PathMode::Trail {
            unique(self.edges.clone())
        } else {
            lit(true)
        };
        result = result.and(match mode {
            ast::PathMode::Acyclic => unique(self.nodes.clone()),
            ast::PathMode::Simple => {
                unique(self.nodes.clone()).or(self.start().eq(self.end()).and(unique(array_slice(
                    self.nodes.clone(),
                    lit(1_i64),
                    cast(array_length(self.nodes.clone()), DataType::Int64) - lit(1_i64),
                    None,
                ))))
            }
            _ => lit(true),
        });
        result
    }
}
fn unique(values: Expr) -> Expr {
    array_length(array_distinct(values.clone())).eq(array_length(values))
}

pub(super) fn length(value: Expr, schema: &datafusion::common::DFSchema) -> Result<Expr> {
    if value.get_type(schema)? == DataType::Null {
        return Ok(lit(ScalarValue::Int64(None)));
    }
    let DataType::Struct(fields) = value.get_type(schema)? else {
        return Err(Error::InvalidQuery(
            "PATH_LENGTH requires a path value".into(),
        ));
    };
    if fields.len() != 3
        || fields[0].name() != GRAPH
        || fields[1].name() != NODES
        || fields[2].name() != EDGES
    {
        return Err(Error::InvalidQuery(
            "PATH_LENGTH requires a path value".into(),
        ));
    }
    Ok(cast(array_length(get_field(value, EDGES)), DataType::Int64))
}
pub(super) fn elements(value: Expr, schema: &datafusion::common::DFSchema) -> Result<Expr> {
    if value.get_type(schema)? == DataType::Null {
        return Ok(super::path_values::null_list());
    }
    length(value.clone(), schema)?;
    Ok(super::path_values::references(
        get_field(value.clone(), GRAPH),
        Some(get_field(value.clone(), NODES)),
        Some(get_field(value, EDGES)),
    ))
}

#[derive(Clone, Copy)]
pub(super) enum Selection {
    All,
    Any(u64),
    Shortest(u64),
    Groups(u64),
}
pub(super) fn prefix(
    prefix: Option<&ast::PathPatternPrefix>,
    session: &SessionState,
) -> Result<(ast::PathMode, Selection)> {
    use ast::PathSearchPrefix as S;
    let count = |n: Option<&ast::UnsignedIntegerSpecification>| -> Result<u64> {
        let n = match n {
            None => 1,
            Some(ast::UnsignedIntegerSpecification::Literal(n)) => *n,
            Some(ast::UnsignedIntegerSpecification::Parameter(name)) => {
                match session.parameters.get(name).map(|p| &p.value) {
                    Some(Value::Integer(n)) if *n > 0 => *n as u64,
                    _ => {
                        return Err(Error::InvalidQuery(format!(
                            "path count parameter ${name} must be a positive integer"
                        )))
                    }
                }
            }
        };
        if n == 0 || n > i64::MAX as u64 {
            return Err(Error::InvalidQuery(
                "path count must be a positive signed integer".into(),
            ));
        }
        Ok(n)
    };
    Ok(match prefix {
        None => (ast::PathMode::Walk, Selection::All),
        Some(ast::PathPatternPrefix::Mode { mode, .. }) => (*mode, Selection::All),
        Some(ast::PathPatternPrefix::Search(search)) => {
            let (mode, selection) = match search {
                S::All { mode, .. } => (mode, Selection::All),
                S::Any { mode, count: n, .. } => (mode, Selection::Any(count(n.as_ref())?)),
                S::Shortest { mode, .. } | S::AnyShortest { mode, .. } => {
                    (mode, Selection::Shortest(1))
                }
                S::AllShortest { mode, .. } => (mode, Selection::Groups(1)),
                S::CountedShortest { mode, count: n, .. } => {
                    (mode, Selection::Shortest(count(Some(n))?))
                }
                S::CountedShortestGroup { mode, count: n, .. } => {
                    (mode, Selection::Groups(count(n.as_ref())?))
                }
            };
            (mode.unwrap_or(ast::PathMode::Walk), selection)
        }
    })
}

pub(super) fn select(
    plan: LogicalPlanBuilder,
    scope: &mut Bindings,
    state: &State,
    selection: Selection,
    input_row: Option<Expr>,
) -> Result<LogicalPlanBuilder> {
    use datafusion::functions_window::{rank::dense_rank, row_number::row_number};
    let (window, count) = match selection {
        Selection::All => return Ok(plan),
        Selection::Any(n) => (row_number(), n),
        Selection::Shortest(n) => (row_number(), n),
        Selection::Groups(n) => (dense_rank(), n),
    };
    let mut keys = vec![state.start(), state.end()];
    keys.extend(input_row);
    let name = scope.fresh("path_rank");
    let rank = window
        .partition_by(keys)
        .order_by(vec![state.length().sort(true, false)])
        .build()?
        .alias(&name);
    Ok(plan
        .window([rank])?
        .filter(Expr::Column(Column::new_unqualified(name)).lt_eq(lit(count)))?)
}

pub(super) fn bounds(
    q: &ast::PathPatternQuantifier,
    mode: ast::PathMode,
    different: bool,
    data: &GraphData,
    limit: u64,
) -> Result<(u64, u64)> {
    use ast::PathPatternQuantifier as Q;
    let (min, max) = match q {
        Q::ZeroOrMore => (0, None),
        Q::OneOrMore => (1, None),
        Q::Optional => (0, Some(1)),
        Q::Fixed(n) => (*n, Some(*n)),
        Q::Range { min, max } => (min.unwrap_or(0), *max),
    };
    if max == Some(0) || max.is_some_and(|max| min > max) {
        return Err(Error::InvalidQuery(
            "path quantifier requires a positive upper bound and lower <= upper".into(),
        ));
    }
    let natural = if different || mode == ast::PathMode::Trail {
        Some(data.edge_count() as u64)
    } else if mode == ast::PathMode::Acyclic {
        Some((data.node_count() as u64).saturating_sub(1))
    } else if mode == ast::PathMode::Simple {
        Some(data.node_count() as u64)
    } else {
        None
    };
    let max = match (max, natural) {
        (Some(a), Some(b)) => a.min(b),
        (Some(a), None) | (None, Some(a)) => a,
        (None, None) => {
            return Err(super::unsupported(
                "unbounded repeatable WALK; specify an upper bound or a finite path mode",
            ))
        }
    };
    if max > limit || min > limit {
        return Err(Error::InvalidQuery(format!(
            "path expansion exceeds the {limit}-hop implementation limit"
        )));
    }
    Ok((min, max))
}

/// Carry the incoming binding table through DataFusion's work table. The recursive
/// term joins one oriented edge; it never traverses graph data in Rust.
#[allow(clippy::too_many_arguments)]
pub(super) fn expand(
    plan: LogicalPlanBuilder,
    scope: &mut Bindings,
    session: &SessionState,
    graph_id: ObjectId,
    data: &GraphData,
    edge: &ast::RelationshipPattern,
    state: &State,
    mode: ast::PathMode,
    different: bool,
) -> Result<(LogicalPlanBuilder, State)> {
    let (min, max) = bounds(
        edge.quantifier.as_ref().expect("quantified edge"),
        mode,
        different,
        data,
        session.query_limits.max_path_hops,
    )?;
    if edge
        .variable
        .as_ref()
        .is_some_and(|name| scope.contains(&name.value))
    {
        return Err(Error::InvalidQuery(
            "quantified group variable is already bound".into(),
        ));
    }
    let incoming = plan.schema().columns();
    let work_name = scope.fresh("path_work");
    let names: Vec<_> = incoming.iter().map(|_| scope.fresh("path_input")).collect();
    let node_name = scope.fresh("path_nodes");
    let edge_name = scope.fresh("path_edges");
    let depth_name = scope.fresh("path_depth");
    let mut projections: Vec<_> = incoming
        .iter()
        .zip(&names)
        .map(|(c, n)| Expr::Column(c.clone()).alias(n))
        .collect();
    projections.extend([
        state.nodes.clone().alias(&node_name),
        state.edges.clone().alias(&edge_name),
        lit(0_u64).alias(&depth_name),
    ]);
    let anchor = plan.project(projections)?;
    let work = LogicalPlanBuilder::scan(
        &work_name,
        provider_as_source(Arc::new(CteWorkTable::new(
            &work_name,
            Arc::new(anchor.schema().as_arrow().clone()),
        ))),
        None,
    )?;
    let mut restored: Vec<_> = incoming
        .iter()
        .zip(&names)
        .map(|(c, n)| execution::col(&work_name, n).alias_qualified(c.relation.clone(), &c.name))
        .collect();
    restored.extend(
        [&node_name, &edge_name, &depth_name].map(|n| execution::col(&work_name, n).alias(n)),
    );
    let mut recursive = work.project(restored)?;
    let col = |n: &str| Expr::Column(Column::new_unqualified(n));
    let current = State {
        nodes: col(&node_name),
        edges: col(&edge_name),
    };
    recursive = recursive.filter(col(&depth_name).lt(lit(max)))?;
    let (edge_scan, binding) = graph::scan(
        scope,
        graph_id,
        data,
        ElementKind::Edge,
        Some(edge.direction),
    )?;
    let mut local = scope.clone();
    if let Some(name) = &edge.variable {
        graph::declare(&mut local, &name.value, &binding)?;
    }
    recursive = recursive.join_on(
        edge_scan,
        JoinType::Inner,
        [current.end().eq(binding.column(FROM))],
    )?;
    recursive = graph::predicates_for(
        recursive,
        &local,
        session,
        &binding,
        &edge.labels,
        edge.label_expression.as_ref(),
        edge.properties.as_ref(),
        edge.where_clause.as_ref(),
    )?;
    if different || mode == ast::PathMode::Trail {
        recursive = recursive.filter(Expr::Not(Box::new(array_has(
            current.edges.clone(),
            binding.column(ID),
        ))))?;
    }
    let next = current.append(binding.column(ID), binding.column(TO));
    recursive = recursive.filter(next.valid(mode, different))?;
    let mut output: Vec<_> = incoming
        .iter()
        .zip(&names)
        .map(|(c, n)| Expr::Column(c.clone()).alias(n))
        .collect();
    output.extend([
        next.nodes.alias(&node_name),
        next.edges.alias(&edge_name),
        (col(&depth_name) + lit(1_u64)).alias(&depth_name),
    ]);
    let result =
        anchor.to_recursive_query(work_name, recursive.project(output)?.build()?, false)?;
    let result = result.filter(col(&depth_name).gt_eq(lit(min)))?;
    let mut output: Vec<_> = incoming
        .iter()
        .zip(&names)
        .map(|(c, n)| col(n).alias_qualified(c.relation.clone(), &c.name))
        .collect();
    output.extend([col(&node_name), col(&edge_name)]);
    let mut result = result.project(output)?;
    if let Some(name) = &edge.variable {
        let ids = array_slice(
            current.edges.clone(),
            cast(array_length(state.edges.clone()), DataType::Int64) + lit(1_i64),
            current.length(),
            None,
        );
        let value = super::path_values::references(lit(graph_id), None, Some(ids));
        let column = scope.scalar(&name.value);
        let mut output: Vec<_> = result
            .schema()
            .columns()
            .into_iter()
            .map(Expr::Column)
            .collect();
        output.push(value.alias(column.name));
        result = result.project(output)?;
    }
    Ok((result, current))
}
