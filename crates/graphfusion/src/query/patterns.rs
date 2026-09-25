//! Compositional path-pattern compiler; every match is a DataFusion relation.
mod groups;
mod union;
use super::{
    execution::{self, Trace},
    expressions::Binder,
    graph::{self, Bindings, ElementKind},
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

type Plan = LogicalPlanBuilder;
type Built = (Plan, Bindings, State);
pub(super) struct Predicate {
    binding: graph::ElementBinding,
    labels: Vec<ast::Identifier>,
    expression: Option<ast::LabelExpression>,
    properties: Option<ast::MapLiteral>,
    predicate: Option<ast::Expr>,
}
pub(super) struct Compiler<'a> {
    pub session: &'a SessionState,
    pub graph_id: ObjectId,
    pub data: &'a GraphData,
    pub ctx: &'a SessionContext,
    pub trace: &'a mut Trace,
    pub different: bool,
    pub predicates: Vec<Predicate>,
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
            )?;
        }
        Ok(plan)
    }
    fn defer_node(&mut self, binding: graph::ElementBinding, node: &ast::NodePattern) {
        self.predicates.push(Predicate {
            binding,
            labels: node.labels.clone(),
            expression: node.label_expression.clone(),
            properties: node.properties.clone(),
            predicate: node.where_clause.clone(),
        });
    }
    // For a homogeneous quantified edge, predicates depend only on the edge and
    // the incoming/start bindings. With lower bound m, every shortest walk has
    // length <= m + N - 1: keep its m-edge prefix and remove cycles in the suffix.
    // For k paths/length groups, use the product graph (node, min(depth,m)), with
    // V=N*(m+1) states. A walk of length >= k*V contains k disjoint cycles;
    // successively removing them gives k strictly shorter accepted walks. Thus
    // k*V-1 is a completeness bound, including every tie in the kth length group.
    fn hint(
        &self,
        path: &ast::PathPattern,
        mode: ast::PathMode,
        selection: Selection,
        inherited: Option<u64>,
    ) -> Result<Option<u64>> {
        let finite = narrower(inherited, self.capacity(mode));
        if finite.is_some() || path.alternation.is_some() {
            return Ok(finite);
        }
        let count = match selection {
            Selection::All => return Ok(None),
            Selection::Any(k) | Selection::Shortest(k) | Selection::Groups(k) => k,
        };
        let mut edge = None;
        for factor in &path.factors {
            match factor {
                ast::PathPatternFactor::Node(node) if node.quantifier.is_none() => (),
                ast::PathPatternFactor::Relationship(value) if edge.is_none() => edge = Some(value),
                _ => return Ok(None),
            }
        }
        let Some(edge) = edge else {
            return Ok(None);
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
                    return Ok(None);
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
            return Ok(None);
        }
        let Some(q) = edge.quantifier.as_ref() else {
            return Ok(None);
        };
        let (min, max) = raw_bounds(q)?;
        if max.is_some() {
            return Ok(None);
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
        Ok(Some(bound.max(min).max(1)))
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
        let (mut plan, start) =
            graph::node(plan, scope, self.graph_id, self.data, first, endpoint)?;
        self.defer_node(start.clone(), first);
        let mut state = State::new(start.column(ID));
        for factor in remaining {
            match factor {
                ast::PathPatternFactor::Node(node) => {
                    let (next, binding) = graph::node(
                        plan,
                        scope,
                        self.graph_id,
                        self.data,
                        node,
                        Some(state.end()),
                    )?;
                    plan = next;
                    self.defer_node(binding, node);
                }
                ast::PathPatternFactor::Relationship(edge) => {
                    if edge.temporary {
                        return Err(super::unsupported("temporary edge pattern"));
                    }
                    if let Some(q) = &edge.quantifier {
                        let mut edge = edge.clone();
                        let (min, max) = raw_bounds(q)?;
                        if max.is_none() && mode == ast::PathMode::Walk && !self.different {
                            edge.quantifier = Some(ast::PathPatternQuantifier::Range {
                                min: Some(min),
                                max: finite.map(|bound| bound.max(min).max(1)),
                            });
                        }
                        (plan, state) = paths::expand(
                            plan,
                            scope,
                            self.session,
                            self.graph_id,
                            self.data,
                            &edge,
                            &state,
                            mode,
                            self.different,
                        )?;
                    } else {
                        let (scan, binding) = graph::scan(
                            scope,
                            self.graph_id,
                            self.data,
                            ElementKind::Edge,
                            Some(edge.direction),
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
        }
        Ok((plan, state))
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
        let (absent, node) = graph::node(
            input.clone(),
            &mut empty_scope,
            self.graph_id,
            self.data,
            &empty_node(),
            endpoint,
        )?;
        scope.advance(&empty_scope);
        self.merge(
            input,
            scope,
            vec![
                (present, branch_scope, state),
                (absent, empty_scope, State::new(node.column(ID))),
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
