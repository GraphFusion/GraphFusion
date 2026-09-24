use super::expressions::Binder;
use crate::{
    catalog::ObjectId,
    gql as ast,
    graph::{self, GraphData, Table},
    Error, Result, SessionState,
};
use datafusion::{
    arrow::datatypes::DataType,
    common::{Column, ScalarValue, TableReference},
    datasource::provider_as_source,
    logical_expr::{cast, lit, Expr, JoinType, LogicalPlan, LogicalPlanBuilder},
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ElementKind {
    Node,
    Edge,
}

#[derive(Clone, Debug)]
pub(super) struct ElementBinding {
    pub alias: String,
    pub graph: ObjectId,
    pub kind: ElementKind,
    pub properties: BTreeMap<String, DataType>,
    pub labels: BTreeMap<String, String>,
    pub present: BTreeMap<String, String>,
}
impl ElementBinding {
    pub fn column(&self, name: &str) -> Expr {
        Expr::Column(Column::new(
            Some(TableReference::bare(self.alias.clone())),
            name,
        ))
    }
    pub fn property(&self, name: &str) -> Expr {
        if self.properties.contains_key(name) {
            self.column(name)
        } else {
            lit(ScalarValue::Null)
        }
    }
    pub fn property_exists(&self, name: &str) -> Expr {
        self.nullable_predicate(self.present.get(name).map_or_else(
            || lit(false),
            |column| self.column(column).and(self.property(name).is_not_null()),
        ))
    }
    pub fn label(&self, label: &ast::LabelExpression) -> Expr {
        self.nullable_predicate(match label {
            ast::LabelExpression::Label(name) => self
                .labels
                .get(&name.value)
                .map_or_else(|| lit(false), |column| self.column(column)),
            ast::LabelExpression::Wildcard => self
                .labels
                .values()
                .map(|column| self.column(column))
                .reduce(Expr::or)
                .unwrap_or_else(|| lit(false)),
            ast::LabelExpression::And(a, b) => self.label(a).and(self.label(b)),
            ast::LabelExpression::Or(a, b) => self.label(a).or(self.label(b)),
            ast::LabelExpression::Not(a) => Expr::Not(Box::new(self.label(a))),
            ast::LabelExpression::Parenthesized(a) => self.label(a),
        })
    }
    pub fn nullable_predicate(&self, value: Expr) -> Expr {
        Expr::Case(datafusion::logical_expr::expr::Case::new(
            None,
            vec![(
                Box::new(self.column(graph::ID).is_null()),
                Box::new(lit(ScalarValue::Boolean(None))),
            )],
            Some(Box::new(value)),
        ))
    }
    pub fn identity(&self) -> Expr {
        let kind = if self.kind == ElementKind::Node {
            "n"
        } else {
            "e"
        };
        let value = datafusion::functions::string::expr_fn::concat(vec![
            lit(format!("g{}:{kind}:", self.graph)),
            cast(self.column(graph::ID), DataType::Utf8),
        ]);
        Expr::Case(datafusion::logical_expr::expr::Case::new(
            None,
            vec![(
                Box::new(self.column(graph::ID).is_null()),
                Box::new(lit(ScalarValue::Utf8(None))),
            )],
            Some(Box::new(value)),
        ))
    }
}

#[derive(Clone, Default)]
pub(super) struct Bindings {
    pub scalars: BTreeMap<String, Column>,
    pub elements: BTreeMap<String, ElementBinding>,
    pub order: Vec<String>,
    pub conditional: BTreeSet<String>,
    pub groups: BTreeSet<String>,
    next: usize,
}
impl Bindings {
    pub fn advance(&mut self, other: &Self) {
        self.next = self.next.max(other.next);
    }
    pub fn fresh(&mut self, prefix: &str) -> String {
        let name = format!("__gf_{prefix}_{}", self.next);
        self.next += 1;
        name
    }
    pub fn contains(&self, name: &str) -> bool {
        self.scalars.contains_key(name) || self.elements.contains_key(name)
    }
    pub fn scalar(&mut self, name: &str) -> Column {
        let column = Column::new_unqualified(format!("__gf_scalar_{}", self.next));
        self.next += 1;
        self.scalars.insert(name.into(), column.clone());
        self.order.push(name.into());
        column
    }
    pub fn element(&self, name: &str) -> Result<&ElementBinding> {
        self.elements
            .get(name)
            .ok_or_else(|| Error::InvalidQuery(format!("unbound element variable {name}")))
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn matches(
    mut plan: LogicalPlanBuilder,
    scope: &mut Bindings,
    session: &SessionState,
    graph_id: ObjectId,
    graph: &GraphData,
    clause: &ast::MatchClause,
    ctx: &datafusion::execution::context::SessionContext,
    trace: &mut super::execution::Trace,
) -> Result<LogicalPlanBuilder> {
    if clause.optional || clause.keep.is_some() || clause.yield_clause.is_some() {
        return Err(super::unsupported("OPTIONAL MATCH, KEEP, or graph YIELD"));
    }
    scope.conditional.clear();
    let different = !matches!(clause.mode, Some(ast::MatchMode::RepeatableElements { .. }));
    if different && clause.patterns.len() > 1 {
        for path in &clause.patterns {
            let (_, selection) = super::paths::prefix(path.prefix.as_ref(), session)?;
            if !matches!(selection, super::paths::Selection::All) {
                return Err(super::unsupported(
                    "selective paths in a multi-path DIFFERENT EDGES MATCH",
                ));
            }
        }
    }
    let mut compiler = super::patterns::Compiler {
        session,
        graph_id,
        data: graph,
        ctx,
        trace,
        different,
        predicates: Vec::new(),
    };
    let mut edge_lists: Vec<Expr> = Vec::new();
    for path in &clause.patterns {
        compiler.validate(path, None)?;
        let (next, state) = compiler
            .pattern(plan, scope, path, None, None, true)
            .await?;
        plan = next;
        if different {
            for previous in &edge_lists {
                plan = plan.filter(Expr::Not(Box::new(
                    datafusion::functions_nested::expr_fn::array_has_any(
                        previous.clone(),
                        state.edges.clone(),
                    ),
                )))?;
            }
        }
        edge_lists.push(state.edges);
    }
    plan = compiler.apply_predicates(plan, scope, 0)?;
    if let Some(predicate) = &clause.where_clause {
        let expr = Binder::with_bindings(session, plan.schema(), scope).predicate(predicate)?;
        plan = plan.filter(expr)?;
    }
    Ok(plan)
}

pub(super) fn check_binding(
    binding: &ElementBinding,
    graph: ObjectId,
    kind: ElementKind,
) -> Result<()> {
    if binding.graph != graph || binding.kind != kind {
        return Err(Error::InvalidQuery(
            "element variable reused with a different graph or kind".into(),
        ));
    }
    Ok(())
}
pub(super) fn declare(scope: &mut Bindings, name: &str, binding: &ElementBinding) -> Result<()> {
    if scope.contains(name) {
        return Err(Error::InvalidQuery(format!(
            "variable {name} is already bound"
        )));
    }
    scope.elements.insert(name.into(), binding.clone());
    scope.order.push(name.into());
    Ok(())
}
pub(super) fn node(
    mut plan: LogicalPlanBuilder,
    scope: &mut Bindings,
    graph_id: ObjectId,
    graph: &GraphData,
    pattern: &ast::NodePattern,
    endpoint: Option<Expr>,
) -> Result<(LogicalPlanBuilder, ElementBinding)> {
    if pattern.temporary || pattern.quantifier.is_some() {
        return Err(super::unsupported("temporary or quantified node pattern"));
    }
    if pattern
        .variable
        .as_ref()
        .is_some_and(|name| scope.conditional.contains(&name.value))
    {
        return Err(Error::InvalidQuery(
            "implicit reuse of a conditional path variable".into(),
        ));
    }
    let previous = pattern
        .variable
        .as_ref()
        .and_then(|name| scope.elements.get(&name.value))
        .cloned();
    let binding = if let Some(binding) = previous {
        check_binding(&binding, graph_id, ElementKind::Node)?;
        plan = plan.filter(binding.column(graph::ID).is_not_null())?;
        if let Some(endpoint) = endpoint {
            plan = plan.filter(binding.column(graph::ID).eq(endpoint))?;
        }
        binding
    } else {
        let (scan, binding) = scan(scope, graph_id, graph, ElementKind::Node, None)?;
        plan = if let Some(endpoint) = endpoint {
            plan.join_on(
                scan,
                JoinType::Inner,
                [endpoint.eq(binding.column(graph::ID))],
            )?
        } else {
            plan.cross_join(scan)?
        };
        if let Some(name) = &pattern.variable {
            declare(scope, &name.value, &binding)?;
        }
        binding
    };
    Ok((plan, binding))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn predicates_for(
    mut plan: LogicalPlanBuilder,
    scope: &Bindings,
    session: &SessionState,
    binding: &ElementBinding,
    labels: &[ast::Identifier],
    expression: Option<&ast::LabelExpression>,
    properties: Option<&ast::MapLiteral>,
    predicate: Option<&ast::Expr>,
) -> Result<LogicalPlanBuilder> {
    if let Some(expression) = expression {
        plan = plan.filter(binding.label(expression))?;
    } else {
        for label in labels {
            plan = plan.filter(binding.label(&ast::LabelExpression::Label(label.clone())))?;
        }
    }
    if let Some(properties) = properties {
        for (name, value) in &properties.entries {
            let right = Binder::with_bindings(session, plan.schema(), scope).bind(value)?;
            let equality = Binder::with_bindings(session, plan.schema(), scope)
                .equals(binding.property(&name.value), right)?;
            plan = plan.filter(equality)?;
        }
    }
    if let Some(predicate) = predicate {
        let expression =
            Binder::with_bindings(session, plan.schema(), scope).predicate(predicate)?;
        plan = plan.filter(expression)?;
    }
    Ok(plan)
}

pub(super) fn scan(
    scope: &mut Bindings,
    graph_id: ObjectId,
    graph: &GraphData,
    kind: ElementKind,
    direction: Option<ast::Direction>,
) -> Result<(LogicalPlan, ElementBinding)> {
    let tables: Vec<(&Table, bool)> = match kind {
        ElementKind::Node => graph.nodes.iter().map(|table| (&table.0, false)).collect(),
        ElementKind::Edge => graph
            .edges
            .iter()
            .map(|table| (&table.table, table.directed))
            .collect(),
    };
    let properties = graph::property_types(tables.iter().map(|(table, _)| *table))?;
    let labels: BTreeSet<_> = tables
        .iter()
        .flat_map(|(table, _)| table.labels.iter().cloned())
        .collect();
    let binding = ElementBinding {
        alias: format!("__gf_element_{}", scope.next),
        graph: graph_id,
        kind,
        labels: labels
            .into_iter()
            .enumerate()
            .map(|(i, name)| (name, format!("__gf_label_{i}")))
            .collect(),
        present: properties
            .keys()
            .enumerate()
            .map(|(i, name)| (name.clone(), format!("__gf_present_{i}")))
            .collect(),
        properties,
    };
    scope.next += 1;
    let mut branches = Vec::new();
    for (table, directed) in tables {
        let orientations = match direction {
            None => vec![false],
            Some(ast::Direction::Right) if directed => vec![false],
            Some(ast::Direction::Left) if directed => vec![true],
            Some(ast::Direction::Undirected) if !directed => vec![false, true],
            Some(ast::Direction::LeftOrRight) if directed => vec![false, true],
            Some(ast::Direction::UndirectedOrRight) => {
                if directed {
                    vec![false]
                } else {
                    vec![false, true]
                }
            }
            Some(ast::Direction::LeftOrUndirected) => {
                if directed {
                    vec![true]
                } else {
                    vec![false, true]
                }
            }
            Some(ast::Direction::Any) => vec![false, true],
            _ => vec![],
        };
        for reverse in orientations {
            let mut scan = LogicalPlanBuilder::scan(
                "__gf_scan",
                provider_as_source(table.provider.clone()),
                None,
            )?;
            // An unoriented self-loop is one match, not two orientation copies.
            let both = matches!(
                direction,
                Some(
                    ast::Direction::Any | ast::Direction::LeftOrRight | ast::Direction::Undirected
                )
            ) || !directed;
            if reverse && both {
                scan = scan.filter(col(graph::SOURCE).not_eq(col(graph::DESTINATION)))?;
            }
            let mut projections = vec![col(graph::ID)];
            if kind == ElementKind::Edge {
                projections.extend([
                    col(graph::SOURCE),
                    col(graph::DESTINATION),
                    lit(directed).alias("__gf_directed"),
                    col(if reverse {
                        graph::DESTINATION
                    } else {
                        graph::SOURCE
                    })
                    .alias(graph::FROM),
                    col(if reverse {
                        graph::SOURCE
                    } else {
                        graph::DESTINATION
                    })
                    .alias(graph::TO),
                ]);
            }
            for (name, data_type) in &binding.properties {
                let present = table.schema.field_with_name(name).is_ok();
                projections.push(if present {
                    col(name)
                } else {
                    lit(ScalarValue::try_from(data_type)?).alias(name)
                });
                projections.push(lit(present).alias(&binding.present[name]));
            }
            for (label, column) in &binding.labels {
                projections.push(lit(table.labels.contains(label)).alias(column));
            }
            branches.push(scan.project(projections)?.build()?);
        }
    }
    let plan = if branches.is_empty() {
        let mut fields = vec![lit(ScalarValue::UInt64(None)).alias(graph::ID)];
        if kind == ElementKind::Edge {
            for name in [graph::SOURCE, graph::DESTINATION] {
                fields.push(lit(ScalarValue::UInt64(None)).alias(name));
            }
            fields.push(lit(false).alias("__gf_directed"));
            for name in [graph::FROM, graph::TO] {
                fields.push(lit(ScalarValue::UInt64(None)).alias(name));
            }
        }
        for (name, data_type) in &binding.properties {
            fields.push(lit(ScalarValue::try_from(data_type)?).alias(name));
            fields.push(lit(false).alias(&binding.present[name]));
        }
        for column in binding.labels.values() {
            fields.push(lit(false).alias(column));
        }
        LogicalPlanBuilder::empty(false).project(fields)?
    } else {
        let mut iter = branches.into_iter();
        let mut plan = LogicalPlanBuilder::from(iter.next().expect("nonempty branches"));
        for branch in iter {
            plan = plan.union(branch)?;
        }
        plan
    };
    Ok((
        plan.alias(TableReference::bare(binding.alias.clone()))?
            .build()?,
        binding,
    ))
}
fn col(name: &str) -> Expr {
    Expr::Column(Column::new_unqualified(name))
}
