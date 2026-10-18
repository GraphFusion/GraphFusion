//! Compositional path-pattern compiler; every match is a DataFusion relation.
mod groups;
mod union;
use super::{
    execution::{self, Trace},
    expressions::Binder,
    graph::{self, Bindings, ElementKind},
    native::{NativeAtom, NativePush},
    paths::{self, Selection, State},
};
use crate::{
    catalog::ObjectId,
    gql as ast,
    graph::{GraphData, FROM, ID, TO},
    Error, Result, SessionState,
};
use datafusion::{
    common::Column,
    execution::context::SessionContext,
    logical_expr::{Expr, JoinType, LogicalPlanBuilder},
};
use std::collections::{BTreeMap, BTreeSet};

type Plan = LogicalPlanBuilder;
type Built = (Plan, Bindings, State);
pub(super) struct Predicate {
    binding: graph::ElementBinding,
    labels: Vec<ast::Identifier>,
    expression: Option<ast::LabelExpression>,
    properties: Option<ast::MapLiteral>,
    predicate: Option<ast::Expr>,
    native: NativePush,
}
pub(super) struct Compiler<'a> {
    pub session: &'a SessionState,
    pub graph_id: ObjectId,
    pub data: &'a GraphData,
    pub ctx: &'a SessionContext,
    pub trace: &'a mut Trace,
    pub different: bool,
    pub predicates: Vec<Predicate>,
    /// Match-level comparisons already safe to apply on each fragment.
    pub clause_filters: BTreeMap<String, NativeAtom>,
    pub clause_push: bool,
    pub clause_hit: BTreeSet<String>,
}
fn narrower(a: Option<u64>, b: Option<u64>) -> Option<u64> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}
fn empty_node() -> ast::NodePattern {
    ast::NodePattern {
        variable: None,
        temporary: false,
        labels: vec![],
        label_expression: None,
        properties: None,
        where_clause: None,
        quantifier: None,
    }
}
/// An anonymous node with no label, property, or WHERE constraint adds nothing
/// the incoming endpoint does not already say. Edge endpoints are stored node
/// ids, so joining the node tables only copies that id.
fn reuses_known_endpoint(node: &ast::NodePattern) -> bool {
    node.variable.is_none()
        && !node.temporary
        && node.quantifier.is_none()
        && node.labels.is_empty()
        && node.label_expression.is_none()
        && node.properties.is_none()
        && node.where_clause.is_none()
}
fn raw_bounds(q: &ast::PathPatternQuantifier) -> Result<(u64, Option<u64>)> {
    use ast::PathPatternQuantifier as Q;
    let bounds = match q {
        Q::ZeroOrMore => (0, None),
        Q::OneOrMore => (1, None),
        Q::Optional => (0, Some(1)),
        Q::Fixed(n) => (*n, Some(*n)),
        Q::Range { min, max } => (min.unwrap_or(0), *max),
    };
    if bounds.1 == Some(0) || bounds.1.is_some_and(|max| max < bounds.0) {
        return Err(Error::InvalidQuery(
            "path quantifier requires a positive upper bound and lower <= upper".into(),
        ));
    }
    Ok(bounds)
}
impl Compiler<'_> {
    pub(super) fn apply_predicates(
        &mut self,
        mut plan: Plan,
        scope: &Bindings,
        start: usize,
    ) -> Result<Plan> {
        for p in self.predicates.split_off(start) {
            plan = graph::predicates_for(
                plan,
                scope,
                self.session,
                &p.binding,
                &p.labels,
                p.expression.as_ref(),
                p.properties.as_ref(),
                p.predicate.as_ref(),
                &p.native,
            )?;
        }
        Ok(plan)
    }
    fn defer_node(
        &mut self,
        binding: graph::ElementBinding,
        node: &ast::NodePattern,
        native: NativePush,
    ) {
        self.predicates.push(Predicate {
            binding,
            labels: node.labels.clone(),
            expression: node.label_expression.clone(),
            properties: node.properties.clone(),
            predicate: node.where_clause.clone(),
            native,
        });
    }
    /// Build the fragment predicate for a newly scanned element. A reused
    /// variable keeps the unified predicate: its rows were already cast.
    fn native_for(
        &mut self,
        plan: &Plan,
        scope: &Bindings,
        variable: Option<&ast::Identifier>,
        properties: Option<&ast::MapLiteral>,
        where_clause: Option<&ast::Expr>,
    ) -> Result<NativePush> {
        let fresh = variable.is_none_or(|name| !scope.elements.contains_key(&name.value));
        if !fresh {
            return Ok(NativePush::default());
        }
        let mut native = NativePush::prepare(
            self.session,
            plan.schema(),
            scope,
            variable.map(|name| name.value.as_str()),
            properties,
            where_clause,
        )?;
        if let Some(name) = variable {
            if let Some(extra) = self.clause_filters.get(&name.value) {
                native.conjunct(extra.clone());
                self.clause_hit.insert(name.value.clone());
            }
        }
        Ok(native)
    }
    // For a homogeneous quantified edge, predicates depend only on the edge and
    // the incoming/start bindings. With lower bound m, every shortest walk has
    // length <= m + N - 1: keep its m-edge prefix and remove cycles in the suffix.
    // For k paths/length groups, use the product graph (node, min(depth,m)), with
    // V=N*(m+1) states. A walk of length >= k*V contains k disjoint cycles;
    // successively removing them gives k strictly shorter accepted walks. Thus
    // k*V-1 is a completeness bound, including every tie in the kth length group.
    // The same cap is sound for trails: cycle removal only drops edges. It is
    // applied under DIFFERENT EDGES as well as repeatable WALK, and narrowed by
    // any smaller mode limit.
    fn hint(
        &self,
        path: &ast::PathPattern,
        mode: ast::PathMode,
        selection: Selection,
        inherited: Option<u64>,
    ) -> Result<Option<u64>> {
        let finite = narrower(inherited, self.capacity(mode));
        if path.alternation.is_some() {
            return Ok(finite);
        }
        let count = match selection {
            Selection::All => return Ok(finite),
            Selection::Any(k) | Selection::Shortest(k) | Selection::Groups(k) => k,
        };
        let mut edge = None;
        for factor in &path.factors {
            match factor {
                ast::PathPatternFactor::Node(node) if node.quantifier.is_none() => (),
                ast::PathPatternFactor::Relationship(value) if edge.is_none() => edge = Some(value),
                _ => return Ok(finite),
            }
        }
        let Some(edge) = edge else {
            return Ok(finite);
        };
        // Endpoint predicates over the group list can constrain the entire walk;
        // cycle removal is not sound for those history-dependent predicates.
        let history = edge.variable.as_ref().map_or("\0", |n| n.value.as_str());
        for factor in &path.factors {
            if let ast::PathPatternFactor::Node(node) = factor {
                if node
                    .where_clause
                    .as_ref()
                    .is_some_and(|e| depends_on(e, history))
                    || node
                        .properties
                        .as_ref()
                        .is_some_and(|p| p.entries.iter().any(|(_, e)| depends_on(e, history)))
                {
                    return Ok(finite);
                }
            }
        }
        // Unknown/new function kinds need a determinism audit even without an
        // exposed group variable; an edge predicate must be invariant by visit.
        if edge
            .where_clause
            .as_ref()
            .is_some_and(|e| depends_on(e, "\0"))
            || edge
                .properties
                .as_ref()
                .is_some_and(|p| p.entries.iter().any(|(_, e)| depends_on(e, "\0")))
        {
            return Ok(finite);
        }
        let Some(q) = edge.quantifier.as_ref() else {
            return Ok(finite);
        };
        let (min, max) = raw_bounds(q)?;
        if max.is_some() {
            return Ok(finite);
        }
        let nodes = self.data.node_count() as u64;
        let bound = if count == 1 {
            min.saturating_add(nodes.saturating_sub(1))
        } else {
            nodes
                .saturating_mul(min.saturating_add(1))
                .saturating_mul(count)
                .saturating_sub(1)
        };
        let bound = bound.max(min).max(1);
        Ok(Some(match finite {
            Some(finite) => finite.min(bound),
            None => bound,
        }))
    }
    fn capacity(&self, mode: ast::PathMode) -> Option<u64> {
        let mode_bound = match mode {
            ast::PathMode::Walk => None,
            ast::PathMode::Trail => Some(self.data.edge_count() as u64),
            ast::PathMode::Acyclic => Some((self.data.node_count() as u64).saturating_sub(1)),
            ast::PathMode::Simple => Some(self.data.node_count() as u64),
        };
        narrower(
            mode_bound,
            self.different.then_some(self.data.edge_count() as u64),
        )
    }
    /// Hop cap already applied inside quantified expansion. A tighter selective
    /// bound has to be written into the quantifier; an equal cap must stay unset
    /// so an empty edge set does not become an illegal explicit maximum of zero.
    fn expansion_limit(&self, mode: ast::PathMode) -> Option<u64> {
        if self.different || mode == ast::PathMode::Trail {
            Some(self.data.edge_count() as u64)
        } else if mode == ast::PathMode::Acyclic {
            Some((self.data.node_count() as u64).saturating_sub(1))
        } else if mode == ast::PathMode::Simple {
            Some(self.data.node_count() as u64)
        } else {
            None
        }
    }
    pub(super) fn validate(
        &self,
        path: &ast::PathPattern,
        finite: Option<u64>,
    ) -> Result<(u64, u64)> {
        let (mode, selection) = paths::prefix(path.prefix.as_ref(), self.session)?;
        let finite = self.hint(path, mode, selection, finite)?;
        let mut alternatives = vec![self.factor_bounds(&path.factors, finite)?];
        for branch in &path.alternatives {
            alternatives.push(self.factor_bounds(&branch.factors, finite)?);
        }
        let min = alternatives.iter().map(|b| b.0).min().unwrap_or(0);
        let max = alternatives.iter().map(|b| b.1).max().unwrap_or(0);
        let max = finite.map_or(max, |bound| max.min(bound));
        if max > self.session.query_limits.max_path_hops {
            return Err(Error::InvalidQuery(format!(
                "path exceeds the {}-hop implementation limit",
                self.session.query_limits.max_path_hops
            )));
        }
        Ok((min, max))
    }
    fn factor_bounds(
        &self,
        factors: &[ast::PathPatternFactor],
        finite: Option<u64>,
    ) -> Result<(u64, u64)> {
        let mut total = (0_u64, 0_u64);
        for factor in factors {
            let next = match factor {
                ast::PathPatternFactor::Node(n) => {
                    if n.quantifier.is_some() {
                        return Err(Error::InvalidQuery(
                            "a quantified path primary must have positive minimum length".into(),
                        ));
                    }
                    (0, 0)
                }
                ast::PathPatternFactor::Relationship(e) => match &e.quantifier {
                    None => (1, 1),
                    Some(q) => {
                        let (min, max) = raw_bounds(q)?;
                        let max=narrower(max,finite).ok_or_else(|| super::unsupported("unbounded repeatable WALK; specify an upper bound or a finite path mode"))?;
                        (min, max)
                    }
                },
                ast::PathPatternFactor::Parenthesized(g) => {
                    let inner_hint = narrower(
                        finite,
                        paths::prefix(g.prefix.as_ref(), self.session)
                            .map(|p| self.capacity(p.0))?,
                    );
                    let (min, max) = self.validate(&g.pattern, inner_hint)?;
                    if let Some(q) = &g.quantifier {
                        if min == 0 {
                            return Err(Error::InvalidQuery(
                                "a quantified path primary must have positive minimum length"
                                    .into(),
                            ));
                        }
                        let (lower, upper) = raw_bounds(q)?;
                        let upper = narrower(upper, finite.map(|n| n / min))
                            .ok_or_else(|| super::unsupported("unbounded repeatable WALK group"))?;
                        (min.saturating_mul(lower), max.saturating_mul(upper))
                    } else if g.questioned {
                        (0, max)
                    } else {
                        (min, max)
                    }
                }
                ast::PathPatternFactor::Alternation { alternatives, .. } => {
                    let branches = alternatives
                        .iter()
                        .map(|b| self.factor_bounds(b, finite))
                        .collect::<Result<Vec<_>>>()?;
                    (
                        branches.iter().map(|b| b.0).min().unwrap_or(0),
                        branches.iter().map(|b| b.1).max().unwrap_or(0),
                    )
                }
            };
            total = (
                total.0.saturating_add(next.0),
                total.1.saturating_add(next.1),
            );
        }
        if let Some(cap) = finite {
            total.1 = total.1.min(cap);
        }
        Ok(total)
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn pattern(
        &mut self,
        mut plan: Plan,
        scope: &mut Bindings,
        path: &ast::PathPattern,
        endpoint: Option<Expr>,
        finite: Option<u64>,
        defer_predicates: bool,
    ) -> Result<(Plan, State)> {
        let predicate_start = self.predicates.len();
        let (mode, selection) = paths::prefix(path.prefix.as_ref(), self.session)?;
        let finite = self.hint(path, mode, selection, finite)?;
        let row = if matches!(selection, Selection::All) {
            None
        } else {
            let name = scope.fresh("path_row");
            plan = execution::ordinal(plan, self.ctx, self.trace, &name).await?;
            Some(Expr::Column(Column::new_unqualified(name)))
        };
        let (mut plan, state) = if let Some(alternation) = path.alternation {
            let mut alternatives = vec![path.factors.clone()];
            alternatives.extend(path.alternatives.iter().map(|b| b.factors.clone()));
            Box::pin(self.alternatives(
                plan,
                scope,
                &alternatives,
                alternation,
                endpoint,
                mode,
                finite,
            ))
            .await?
        } else {
            Box::pin(self.factors(plan, scope, &path.factors, endpoint, mode, finite)).await?
        };
        // Bind the complete path before its inline predicates, and filter
        // candidates before selection. Plain MATCH paths may refer forward
        // across other paths, so their predicates remain pending until then.
        if !defer_predicates || !matches!(selection, Selection::All) {
            plan = self.apply_predicates(plan, scope, predicate_start)?;
        }
        plan = plan.filter(state.valid(mode, self.different))?;
        plan = paths::select(plan, scope, &state, selection, row)?;
        if let Some(name) = &path.variable {
            plan = self.bind_path(plan, scope, &name.value, &state)?;
        }
        Ok((plan, state))
    }
    fn bind_path(
        &self,
        plan: Plan,
        scope: &mut Bindings,
        name: &str,
        state: &State,
    ) -> Result<Plan> {
        if scope.contains(name) {
            return Err(Error::InvalidQuery(format!(
                "variable {name} is already bound"
            )));
        }
        let column = scope.scalar(name);
        scope.domains.insert(
            name.into(),
            [
                (self.graph_id, ElementKind::Node),
                (self.graph_id, ElementKind::Edge),
            ]
            .into(),
        );
        let mut projection: Vec<_> = plan
            .schema()
            .columns()
            .into_iter()
            .map(Expr::Column)
            .collect();
        projection.push(state.value(self.graph_id).alias(column.name));
        Ok(plan.project(projection)?)
    }
    async fn factors(
        &mut self,
        plan: Plan,
        scope: &mut Bindings,
        factors: &[ast::PathPatternFactor],
        endpoint: Option<Expr>,
        mode: ast::PathMode,
        finite: Option<u64>,
    ) -> Result<(Plan, State)> {
        let empty = empty_node();
        let (first, remaining) = match factors.split_first() {
            Some((ast::PathPatternFactor::Node(node), rest)) => (node, rest),
            _ => (&empty, factors),
        };
        // Keep predicates from earlier patterns pending until their variables are bound.
        let pending = self.predicates.len();
        // The first edge already carries both anonymous endpoint identities.
        let opened = if endpoint.is_none() && reuses_known_endpoint(first) {
            match remaining.split_first() {
                Some((ast::PathPatternFactor::Relationship(edge), rest))
                    if !edge.temporary
                        && edge
                            .quantifier
                            .as_ref()
                            .is_none_or(|q| matches!(raw_bounds(q), Ok((min, _)) if min >= 1)) =>
                {
                    Some((edge, rest))
                }
                _ => None,
            }
        } else {
            None
        };
        let (mut plan, mut state, rest) = if let Some((edge, rest)) = opened {
            let (plan, state) = if let Some(q) = &edge.quantifier {
                let mut edge = edge.clone();
                let (min, max) = raw_bounds(q)?;
                if max.is_none() {
                    if let Some(bound) = finite.filter(|bound| {
                        self.expansion_limit(mode)
                            .is_none_or(|natural| *bound < natural)
                    }) {
                        edge.quantifier = Some(ast::PathPatternQuantifier::Range {
                            min: Some(min),
                            max: Some(bound.max(min).max(1)),
                        });
                    }
                }
                let native = NativePush::prepare(
                    self.session,
                    plan.schema(),
                    scope,
                    edge.variable.as_ref().map(|name| name.value.as_str()),
                    edge.properties.as_ref(),
                    edge.where_clause.as_ref(),
                )?;
                paths::expand(
                    plan,
                    scope,
                    self.session,
                    self.graph_id,
                    self.data,
                    &edge,
                    None,
                    mode,
                    self.different,
                    &native,
                )?
            } else {
                self.open_edge(plan, scope, edge)?
            };
            (plan.filter(state.valid(mode, self.different))?, state, rest)
        } else {
            let (plan, state) = match endpoint {
                Some(endpoint) if reuses_known_endpoint(first) => (
                    plan.filter(endpoint.clone().is_not_null())?,
                    State::new(endpoint),
                ),
                endpoint => self.bind_node(plan, scope, first, endpoint)?,
            };
            (plan, state, remaining)
        };
        plan = self.flush_ready(plan, scope, pending)?;
        for factor in rest {
            match factor {
                ast::PathPatternFactor::Node(node) => {
                    if reuses_known_endpoint(node) {
                        plan = plan.filter(state.end().is_not_null())?;
                    } else {
                        let (next, _) = self.bind_node(plan, scope, node, Some(state.end()))?;
                        plan = next;
                    }
                }
                ast::PathPatternFactor::Relationship(edge) => {
                    if edge.temporary {
                        return Err(super::unsupported("temporary edge pattern"));
                    }
                    if let Some(q) = &edge.quantifier {
                        let mut edge = edge.clone();
                        let (min, max) = raw_bounds(q)?;
                        if max.is_none() {
                            if let Some(bound) = finite.filter(|bound| {
                                self.expansion_limit(mode)
                                    .is_none_or(|natural| *bound < natural)
                            }) {
                                // A selective search can stop before the trail cap. Writing a
                                // zero cap would reject a quantifier that the natural limit
                                // still treats as unbounded.
                                edge.quantifier = Some(ast::PathPatternQuantifier::Range {
                                    min: Some(min),
                                    max: Some(bound.max(min).max(1)),
                                });
                            }
                        }
                        let native = NativePush::prepare(
                            self.session,
                            plan.schema(),
                            scope,
                            edge.variable.as_ref().map(|name| name.value.as_str()),
                            edge.properties.as_ref(),
                            edge.where_clause.as_ref(),
                        )?;
                        (plan, state) = paths::expand(
                            plan,
                            scope,
                            self.session,
                            self.graph_id,
                            self.data,
                            &edge,
                            Some(&state),
                            mode,
                            self.different,
                            &native,
                        )?;
                    } else {
                        let native = self.native_for(
                            &plan,
                            scope,
                            edge.variable.as_ref(),
                            edge.properties.as_ref(),
                            edge.where_clause.as_ref(),
                        )?;
                        let (scan, binding) = graph::scan_filtered(
                            scope,
                            self.graph_id,
                            self.data,
                            ElementKind::Edge,
                            Some(edge.direction),
                            graph::label_filter(edge.label_expression.as_ref(), &edge.labels),
                            &native,
                        )?;
                        let mut predicates = vec![state.end().eq(binding.column(FROM))];
                        if let Some(name) = &edge.variable {
                            if scope.conditional.contains(&name.value) {
                                return Err(Error::InvalidQuery(
                                    "implicit reuse of a conditional path variable".into(),
                                ));
                            }
                            if let Some(previous) = scope.elements.get(&name.value) {
                                graph::check_binding(previous, self.graph_id, ElementKind::Edge)?;
                                predicates.push(previous.column(ID).eq(binding.column(ID)));
                            } else if let Some(reference) =
                                super::references::scalar(scope, &name.value, plan.schema())?
                            {
                                predicates.push(super::references::constraint(reference, &binding));
                                scope.elements.insert(name.value.clone(), binding.clone());
                            } else {
                                graph::declare(scope, &name.value, &binding)?;
                            }
                        }
                        plan = plan.join_on(scan, JoinType::Inner, predicates)?;
                        self.predicates.push(Predicate {
                            binding: binding.clone(),
                            labels: edge.labels.clone(),
                            expression: edge.label_expression.clone(),
                            properties: edge.properties.clone(),
                            predicate: edge.where_clause.clone(),
                            native,
                        });
                        state = state.append(binding.column(ID), binding.column(TO));
                    }
                }
                ast::PathPatternFactor::Parenthesized(group) => {
                    if group.quantifier.is_some() {
                        (plan, state) =
                            Box::pin(self.repeat(plan, scope, group, &state, mode, finite)).await?;
                    } else {
                        let (next, sub) =
                            Box::pin(self.group(plan, scope, group, Some(state.end()), finite))
                                .await?;
                        plan = next;
                        state = state.concat(&sub);
                    }
                }
                ast::PathPatternFactor::Alternation {
                    alternation,
                    alternatives,
                } => {
                    let (next, sub) = Box::pin(self.alternatives(
                        plan,
                        scope,
                        alternatives,
                        *alternation,
                        Some(state.end()),
                        mode,
                        finite,
                    ))
                    .await?;
                    plan = next;
                    state = state.concat(&sub);
                }
            }
            plan = plan.filter(state.valid(mode, self.different))?;
            plan = self.flush_ready(plan, scope, pending)?;
        }
        Ok((plan, state))
    }
    fn flush_ready(&mut self, mut plan: Plan, scope: &Bindings, start: usize) -> Result<Plan> {
        let pending = self.predicates.split_off(start);
        let mut deferred = Vec::new();
        for predicate in pending {
            if predicate_is_noop(&predicate) {
                continue;
            }
            if predicate_is_ready(&predicate, scope) {
                plan = graph::predicates_for(
                    plan,
                    scope,
                    self.session,
                    &predicate.binding,
                    &predicate.labels,
                    predicate.expression.as_ref(),
                    predicate.properties.as_ref(),
                    predicate.predicate.as_ref(),
                    &predicate.native,
                )?;
            } else {
                deferred.push(predicate);
            }
        }
        self.predicates.extend(deferred);
        Ok(plan)
    }
    fn open_edge(
        &mut self,
        mut plan: Plan,
        scope: &mut Bindings,
        edge: &ast::RelationshipPattern,
    ) -> Result<(Plan, State)> {
        let native = self.native_for(
            &plan,
            scope,
            edge.variable.as_ref(),
            edge.properties.as_ref(),
            edge.where_clause.as_ref(),
        )?;
        let (scan, binding) = graph::scan_filtered(
            scope,
            self.graph_id,
            self.data,
            ElementKind::Edge,
            Some(edge.direction),
            graph::label_filter(edge.label_expression.as_ref(), &edge.labels),
            &native,
        )?;
        let mut predicates = Vec::new();
        if let Some(name) = &edge.variable {
            if scope.conditional.contains(&name.value) {
                return Err(Error::InvalidQuery(
                    "implicit reuse of a conditional path variable".into(),
                ));
            }
            if let Some(previous) = scope.elements.get(&name.value) {
                graph::check_binding(previous, self.graph_id, ElementKind::Edge)?;
                predicates.push(previous.column(ID).eq(binding.column(ID)));
            } else if let Some(reference) =
                super::references::scalar(scope, &name.value, plan.schema())?
            {
                predicates.push(super::references::constraint(reference, &binding));
                scope.elements.insert(name.value.clone(), binding.clone());
            } else {
                graph::declare(scope, &name.value, &binding)?;
            }
        }
        plan = plan.cross_join(scan)?;
        for predicate in predicates {
            plan = plan.filter(predicate)?;
        }
        self.predicates.push(Predicate {
            binding: binding.clone(),
            labels: edge.labels.clone(),
            expression: edge.label_expression.clone(),
            properties: edge.properties.clone(),
            predicate: edge.where_clause.clone(),
            native,
        });
        let state = State::new(binding.column(FROM)).append(binding.column(ID), binding.column(TO));
        Ok((plan, state))
    }
    fn bind_node(
        &mut self,
        plan: Plan,
        scope: &mut Bindings,
        node: &ast::NodePattern,
        endpoint: Option<Expr>,
    ) -> Result<(Plan, State)> {
        let native = self.native_for(
            &plan,
            scope,
            node.variable.as_ref(),
            node.properties.as_ref(),
            node.where_clause.as_ref(),
        )?;
        let (plan, binding) = graph::node(
            plan,
            scope,
            self.graph_id,
            self.data,
            node,
            endpoint,
            &native,
        )?;
        self.defer_node(binding.clone(), node, native);
        Ok((plan, State::new(binding.column(ID))))
    }
    async fn group_once(
        &mut self,
        plan: Plan,
        scope: &mut Bindings,
        group: &ast::ParenthesizedPathPatternExpression,
        endpoint: Option<Expr>,
        finite: Option<u64>,
    ) -> Result<(Plan, State)> {
        let (mode, selection) = paths::prefix(group.prefix.as_ref(), self.session)?;
        if !matches!(selection, Selection::All) {
            return Err(super::unsupported(
                "search prefix inside a parenthesized path primary",
            ));
        }
        let finite = narrower(finite, self.capacity(mode));
        let (mut plan, state) =
            Box::pin(self.pattern(plan, scope, &group.pattern, endpoint, finite, false)).await?;
        plan = plan.filter(state.valid(mode, self.different))?;
        if let Some(predicate) = &group.where_clause {
            let value =
                Binder::with_bindings(self.session, plan.schema(), scope).predicate(predicate)?;
            plan = plan.filter(value)?;
        }
        if let Some(name) = &group.variable {
            plan = self.bind_path(plan, scope, &name.value, &state)?;
        }
        Ok((plan, state))
    }
    async fn group(
        &mut self,
        plan: Plan,
        scope: &mut Bindings,
        group: &ast::ParenthesizedPathPatternExpression,
        endpoint: Option<Expr>,
        finite: Option<u64>,
    ) -> Result<(Plan, State)> {
        if !group.questioned {
            return Box::pin(self.group_once(plan, scope, group, endpoint, finite)).await;
        }
        let row = scope.fresh("question_row");
        let input = execution::ordinal(plan, self.ctx, self.trace, &row).await?;
        let mut branch_scope = scope.clone();
        let (present, state) = Box::pin(self.group_once(
            input.clone(),
            &mut branch_scope,
            group,
            endpoint.clone(),
            finite,
        ))
        .await?;
        scope.advance(&branch_scope);
        let mut empty_scope = scope.clone();
        // An anchored optional segment already knows its endpoint. Joining that
        // id back to a full node scan only copies the same value.
        let (absent, absent_state) = if let Some(endpoint) = endpoint {
            let absent = input.clone().filter(endpoint.clone().is_not_null())?;
            (absent, State::new(endpoint))
        } else {
            let (absent, node) = graph::node(
                input.clone(),
                &mut empty_scope,
                self.graph_id,
                self.data,
                &empty_node(),
                None,
                &NativePush::default(),
            )?;
            (absent, State::new(node.column(ID)))
        };
        scope.advance(&empty_scope);
        self.merge(
            input,
            scope,
            vec![
                (present, branch_scope, state),
                (absent, empty_scope, absent_state),
            ],
            ast::PathPatternAlternation::Multiset,
        )
        .await
    }
    #[allow(clippy::too_many_arguments)]
    async fn alternatives(
        &mut self,
        plan: Plan,
        scope: &mut Bindings,
        alternatives: &[Vec<ast::PathPatternFactor>],
        alternation: ast::PathPatternAlternation,
        endpoint: Option<Expr>,
        mode: ast::PathMode,
        finite: Option<u64>,
    ) -> Result<(Plan, State)> {
        let row = scope.fresh("alternative_row");
        let input = execution::ordinal(plan, self.ctx, self.trace, &row).await?;
        let mut branches = Vec::new();
        for factors in alternatives {
            let predicate_start = self.predicates.len();
            let mut branch_scope = scope.clone();
            let (plan, state) = Box::pin(self.factors(
                input.clone(),
                &mut branch_scope,
                factors,
                endpoint.clone(),
                mode,
                finite,
            ))
            .await?;
            let plan = self.apply_predicates(plan, &branch_scope, predicate_start)?;
            scope.advance(&branch_scope);
            branches.push((plan, branch_scope, state));
        }
        self.merge(input, scope, branches, alternation).await
    }
}

fn depends_on(expr: &ast::Expr, name: &str) -> bool {
    use ast::Expr as E;
    match expr {
        E::Literal(_) | E::Parameter(_) => false,
        E::Identifier(id) => id.value == name,
        E::Property { base, .. } => depends_on(base, name),
        E::Unary { expr, .. }
        | E::IsNull { expr, .. }
        | E::IsUnknown { expr, .. }
        | E::IsTruth { expr, .. } => depends_on(expr, name),
        E::Binary { left, right, .. } | E::NullIf { left, right } => {
            depends_on(left, name) || depends_on(right, name)
        }
        E::NumericFunction { args, .. } | E::Coalesce(args) | E::List(args) => {
            args.iter().any(|e| depends_on(e, name))
        }
        E::ElementId { variable }
        | E::PropertyExists { variable, .. }
        | E::IsLabeled { variable, .. }
        | E::IsDirected { variable, .. } => variable.value == name,
        E::Same { variables } | E::AllDifferent { variables } => {
            variables.iter().any(|v| v.value == name)
        }
        _ => true, // A new expression kind must be audited before it enables this proof.
    }
}

fn predicate_is_noop(predicate: &Predicate) -> bool {
    predicate.labels.is_empty()
        && predicate.expression.is_none()
        && predicate
            .properties
            .as_ref()
            .is_none_or(|properties| properties.entries.is_empty())
        && predicate.predicate.is_none()
}

fn predicate_is_ready(predicate: &Predicate, scope: &Bindings) -> bool {
    predicate.properties.as_ref().is_none_or(|properties| {
        properties
            .entries
            .iter()
            .all(|(_, value)| expr_is_ready(value, scope))
    }) && predicate
        .predicate
        .as_ref()
        .is_none_or(|predicate| expr_is_ready(predicate, scope))
}

/// A predicate can run only when every variable it reads is already bound.
/// Unknown expression kinds stay deferred.
fn expr_is_ready(expr: &ast::Expr, scope: &Bindings) -> bool {
    use ast::Expr as E;
    let bound = |name: &ast::Identifier| {
        scope.contains(&name.value) || scope.references.contains_key(&name.value)
    };
    match expr {
        E::Literal(_) | E::Parameter(_) | E::Wildcard => true,
        E::Identifier(name) => bound(name),
        E::Property { base, .. } => expr_is_ready(base, scope),
        E::Unary { expr, .. }
        | E::IsNull { expr, .. }
        | E::IsUnknown { expr, .. }
        | E::IsTruth { expr, .. }
        | E::AbsoluteValue { expr }
        | E::Elements { path: expr }
        | E::Cast { expr, .. } => expr_is_ready(expr, scope),
        E::Binary { left, right, .. } | E::NullIf { left, right } => {
            expr_is_ready(left, scope) && expr_is_ready(right, scope)
        }
        E::NumericFunction { args, .. }
        | E::Coalesce(args)
        | E::List(args)
        | E::Function { args, .. } => args.iter().all(|expr| expr_is_ready(expr, scope)),
        E::ElementId { variable }
        | E::PropertyExists { variable, .. }
        | E::IsLabeled { variable, .. }
        | E::IsDirected { variable, .. } => bound(variable),
        E::Same { variables } | E::AllDifferent { variables } => variables.iter().all(bound),
        _ => false,
    }
}
