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
        self.present.get(name).map_or_else(
            || lit(false),
            |column| self.column(column).and(self.property(name).is_not_null()),
        )
    }
    pub fn label(&self, label: &ast::LabelExpression) -> Expr {
        match label {
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
        }
    }
    pub fn identity(&self) -> Expr {
        let kind = if self.kind == ElementKind::Node {
            "n"
        } else {
            "e"
        };
        datafusion::functions::string::expr_fn::concat(vec![
            lit(format!("g{}:{kind}:", self.graph)),
            cast(self.column(graph::ID), DataType::Utf8),
        ])
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

pub(super) fn matches(
    mut plan: LogicalPlanBuilder,
    scope: &mut Bindings,
    session: &SessionState,
    graph_id: ObjectId,
    graph: &GraphData,
    clause: &ast::MatchClause,
) -> Result<LogicalPlanBuilder> {
    if clause.optional || clause.keep.is_some() || clause.yield_clause.is_some() {
        return Err(super::unsupported("OPTIONAL MATCH, KEEP, or graph YIELD"));
    }
    let different = !matches!(clause.mode, Some(ast::MatchMode::RepeatableElements { .. }));
    let mut edge_ids: Vec<Expr> = Vec::new();
    // Every element in this MATCH must be bound before resolving its predicates.
    let mut element_predicates = Vec::new();
    for path in &clause.patterns {
        if path.variable.is_some()
            || path.parenthesized.is_some()
            || path.alternation.is_some()
            || !path.alternatives.is_empty()
            || !matches!(
                &path.prefix,
                None | Some(ast::PathPatternPrefix::Mode {
                    mode: ast::PathMode::Walk,
                    ..
                }) | Some(ast::PathPatternPrefix::Search(ast::PathSearchPrefix::All {
                    mode: None | Some(ast::PathMode::Walk),
                    ..
                }))
            )
            || path.factors.iter().any(|factor| {
                !matches!(
                    factor,
                    ast::PathPatternFactor::Node(_) | ast::PathPatternFactor::Relationship(_)
                )
            })
            || path.factors.len() != path.chains.len() * 2 + 1
        {
            return Err(super::unsupported("complex or named path pattern"));
        }
        let (next, mut current_node) = node(plan, scope, graph_id, graph, &path.start, None)?;
        plan = next;
        element_predicates.push((
            current_node.clone(),
            &path.start.labels,
            path.start.label_expression.as_ref(),
            path.start.properties.as_ref(),
            path.start.where_clause.as_ref(),
        ));
        for chain in &path.chains {
            let edge = &chain.relationship;
            if edge.temporary || edge.quantifier.is_some() {
                return Err(super::unsupported("temporary or quantified edge pattern"));
            }
            let (edge_plan, binding) = scan(
                scope,
                graph_id,
                graph,
                ElementKind::Edge,
                Some(edge.direction),
            )?;
            let mut predicates = vec![current_node
                .column(graph::ID)
                .eq(binding.column(graph::FROM))];
            if let Some(name) = &edge.variable {
                if let Some(previous) = scope.elements.get(&name.value) {
                    check_binding(previous, graph_id, ElementKind::Edge)?;
                    predicates.push(previous.column(graph::ID).eq(binding.column(graph::ID)));
                } else {
                    declare(scope, &name.value, &binding)?;
                }
            }
            plan = plan.join_on(edge_plan, JoinType::Inner, predicates)?;
            if different {
                for previous in &edge_ids {
                    plan = plan.filter(previous.clone().not_eq(binding.column(graph::ID)))?;
                }
            }
            edge_ids.push(binding.column(graph::ID));
            element_predicates.push((
                binding.clone(),
                &edge.labels,
                edge.label_expression.as_ref(),
                edge.properties.as_ref(),
                edge.where_clause.as_ref(),
            ));
            let (next, next_node) = node(
                plan,
                scope,
                graph_id,
                graph,
                &chain.node,
                Some(binding.column(graph::TO)),
            )?;
            plan = next;
            element_predicates.push((
                next_node.clone(),
                &chain.node.labels,
                chain.node.label_expression.as_ref(),
                chain.node.properties.as_ref(),
                chain.node.where_clause.as_ref(),
            ));
            current_node = next_node;
        }
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
fn predicates_for(
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
