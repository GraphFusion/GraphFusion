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
    next: usize,
}
impl Bindings {
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
    use super::paths::{self, Selection, State};
    if clause.optional || clause.keep.is_some() || clause.yield_clause.is_some() {
        return Err(super::unsupported("OPTIONAL MATCH, KEEP, or graph YIELD"));
    }
    let different = !matches!(clause.mode, Some(ast::MatchMode::RepeatableElements { .. }));
    let mut edge_lists: Vec<Expr> = Vec::new();
    // Unselected patterns bind every element before resolving their predicates.
    let mut element_predicates = Vec::new();
    for path in &clause.patterns {
        if path.parenthesized.is_some()
            || path.alternation.is_some()
            || !path.alternatives.is_empty()
            || path.factors.iter().any(|factor| {
                !matches!(
                    factor,
                    ast::PathPatternFactor::Node(_) | ast::PathPatternFactor::Relationship(_)
                )
            })
            || path.factors.len() != path.chains.len() * 2 + 1
        {
            return Err(super::unsupported(
                "parenthesized or alternative path pattern",
            ));
        }
        let (mode, selection) = paths::prefix(path.prefix.as_ref(), session)?;
        let row = if matches!(selection, Selection::All) {
            None
        } else {
            let name = scope.fresh("path_row");
            plan = super::execution::ordinal(plan, ctx, trace, &name).await?;
            Some(Expr::Column(Column::new_unqualified(name)))
        };
        let mut max_hops = 0_u64;
        for chain in &path.chains {
            max_hops = max_hops.saturating_add(match &chain.relationship.quantifier {
                Some(q) => {
                    paths::bounds(
                        q,
                        mode,
                        different,
                        graph,
                        session.query_limits.max_path_hops,
                    )?
                    .1
                }
                None => 1,
            });
            if max_hops > session.query_limits.max_path_hops {
                return Err(Error::InvalidQuery(format!(
                    "path exceeds the {}-hop implementation limit",
                    session.query_limits.max_path_hops
                )));
            }
        }
        let (next, start_node) = node(plan, scope, graph_id, graph, &path.start, None)?;
        plan = next;
        let mut path_predicates = vec![(
            start_node.clone(),
            &path.start.labels,
            path.start.label_expression.as_ref(),
            path.start.properties.as_ref(),
            path.start.where_clause.as_ref(),
        )];
        let mut state = State::new(start_node.column(graph::ID));
        for chain in &path.chains {
            let edge = &chain.relationship;
            if edge.temporary {
                return Err(super::unsupported("temporary edge pattern"));
            }
            let endpoint = if edge.quantifier.is_some() {
                (plan, state) = paths::expand(
                    plan, scope, session, graph_id, graph, edge, &state, mode, different,
                )?;
                state.end()
            } else {
                let (edge_plan, binding) = scan(
                    scope,
                    graph_id,
                    graph,
                    ElementKind::Edge,
                    Some(edge.direction),
                )?;
                let mut predicates = vec![state.end().eq(binding.column(graph::FROM))];
                if let Some(name) = &edge.variable {
                    if let Some(previous) = scope.elements.get(&name.value) {
                        check_binding(previous, graph_id, ElementKind::Edge)?;
                        predicates.push(previous.column(graph::ID).eq(binding.column(graph::ID)));
                    } else {
                        declare(scope, &name.value, &binding)?;
                    }
                }
                plan = plan.join_on(edge_plan, JoinType::Inner, predicates)?;
                path_predicates.push((
                    binding.clone(),
                    &edge.labels,
                    edge.label_expression.as_ref(),
                    edge.properties.as_ref(),
                    edge.where_clause.as_ref(),
                ));
                state = state.append(binding.column(graph::ID), binding.column(graph::TO));
                binding.column(graph::TO)
            };
            let (next, next_node) =
                node(plan, scope, graph_id, graph, &chain.node, Some(endpoint))?;
            plan = next;
            path_predicates.push((
                next_node,
                &chain.node.labels,
                chain.node.label_expression.as_ref(),
                chain.node.properties.as_ref(),
                chain.node.where_clause.as_ref(),
            ));
        }
        if matches!(selection, Selection::All) {
            element_predicates.extend(path_predicates);
        } else {
            // Inline predicates constrain candidates before shortest/ANY selection.
            for (binding, labels, expression, properties, predicate) in path_predicates {
                plan = predicates_for(
                    plan, scope, session, &binding, labels, expression, properties, predicate,
                )?;
            }
        }
        plan = plan.filter(state.valid(mode, different))?;
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
        plan = paths::select(plan, scope, &state, selection, row)?;
        if let Some(name) = &path.variable {
            if scope.contains(&name.value) {
                return Err(Error::InvalidQuery(format!(
                    "variable {} is already bound",
                    name.value
                )));
            }
            let column = scope.scalar(&name.value);
            let mut projection: Vec<_> = plan
                .schema()
                .columns()
                .into_iter()
                .map(Expr::Column)
                .collect();
            projection.push(state.value(graph_id).alias(column.name));
            plan = plan.project(projection)?;
        }
        edge_lists.push(state.edges);
    }
    for (binding, labels, expression, properties, predicate) in element_predicates {
        plan = predicates_for(
            plan, scope, session, &binding, labels, expression, properties, predicate,
        )?;
    }
    if let Some(predicate) = &clause.where_clause {
        let expr = Binder::with_bindings(session, plan.schema(), scope).predicate(predicate)?;
        plan = plan.filter(expr)?;
    }
    Ok(plan)
}

fn check_binding(binding: &ElementBinding, graph: ObjectId, kind: ElementKind) -> Result<()> {
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
fn node(
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
