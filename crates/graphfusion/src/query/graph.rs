use super::casefold::{CaseFoldMap, CaseFoldSet};
use super::{expressions::Binder, native::NativePush};
use crate::{
    catalog::ObjectId,
    gql as ast,
    graph::{self, GraphData, Table},
    Error, Result, SessionState,
};
use datafusion::{
    arrow::{
        array::{Array, BooleanArray, NullArray},
        datatypes::DataType,
    },
    common::{Column, ScalarValue, TableReference},
    datasource::provider_as_source,
    error::DataFusionError,
    logical_expr::{
        cast, create_udf, lit, ColumnarValue, Expr, JoinType, LogicalPlan, LogicalPlanBuilder,
        Volatility,
    },
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
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
    /// Label sets this binding was read from. A later write refreshes those
    /// fragments instead of widening every property onto unrelated types.
    pub source_labels: Vec<BTreeSet<String>>,
    /// Set when a label pattern or an insert selected the source fragments.
    /// An unconstrained match keeps the full element-kind union.
    pub constrained: bool,
    /// Identities deleted earlier in this statement that this variable still
    /// carries on some rows. Other rows of the variable stay readable.
    pub tombstones: BTreeSet<u64>,
    /// Identity from before a post-delete refresh. The refreshed id is null
    /// for a removed row, which would otherwise look like an optional miss.
    pub prior_id: Option<String>,
}
impl ElementBinding {
    pub fn column(&self, name: &str) -> Expr {
        Expr::Column(Column::new(
            Some(TableReference::bare(self.alias.clone())),
            name,
        ))
    }
    fn stored_property<'a>(&'a self, name: &str) -> Option<&'a str> {
        self.properties
            .keys()
            .find(|stored| stored.eq_ignore_ascii_case(name))
            .map(String::as_str)
    }
    fn raw_property(&self, name: &str) -> Expr {
        match self.stored_property(name) {
            Some(stored) => self.column(stored),
            None => lit(ScalarValue::Null),
        }
    }
    pub fn property(&self, name: &str) -> Expr {
        let data_type = self
            .stored_property(name)
            .and_then(|stored| self.properties.get(stored))
            .cloned()
            .unwrap_or(DataType::Null);
        self.ensure_live(self.raw_property(name), data_type)
    }
    pub fn property_exists(&self, name: &str) -> Expr {
        let value = self
            .stored_property(name)
            .and_then(|stored| self.present.get(stored))
            .map_or_else(
                || lit(false),
                |column| {
                    self.column(column)
                        .and(self.raw_property(name).is_not_null())
                },
            );
        self.ensure_live(self.nullable_predicate(value), DataType::Boolean)
    }
    pub fn label(&self, label: &ast::LabelExpression) -> Expr {
        self.ensure_live(
            self.nullable_predicate(self.raw_label(label)),
            DataType::Boolean,
        )
    }
    fn raw_label(&self, label: &ast::LabelExpression) -> Expr {
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
            ast::LabelExpression::And(a, b) => self.raw_label(a).and(self.raw_label(b)),
            ast::LabelExpression::Or(a, b) => self.raw_label(a).or(self.raw_label(b)),
            ast::LabelExpression::Not(a) => Expr::Not(Box::new(self.raw_label(a))),
            ast::LabelExpression::Parenthesized(a) => self.raw_label(a),
        }
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
        self.ensure_live(
            Expr::Case(datafusion::logical_expr::expr::Case::new(
                None,
                vec![(
                    Box::new(self.column(graph::ID).is_null()),
                    Box::new(lit(ScalarValue::Utf8(None))),
                )],
                Some(Box::new(value)),
            )),
            DataType::Utf8,
        )
    }
    /// Fail the batch when a row still names an identity deleted earlier in
    /// the statement. Rows whose identity was not deleted pass through.
    pub fn ensure_live(&self, value: Expr, data_type: DataType) -> Expr {
        if self.tombstones.is_empty() {
            return value;
        }
        let id = if let Some(name) = &self.prior_id {
            Expr::Column(Column::new_unqualified(name))
        } else {
            self.column(graph::ID)
        };
        let deleted = id.in_list(self.tombstones.iter().copied().map(lit).collect(), false);
        if data_type == DataType::Null {
            return create_udf(
                "gql_live_element_null",
                vec![DataType::Boolean],
                DataType::Null,
                Volatility::Immutable,
                Arc::new(|args| {
                    let arrays = ColumnarValue::values_to_arrays(args)?;
                    reject_deleted(&arrays[0])?;
                    Ok(ColumnarValue::Array(Arc::new(NullArray::new(
                        arrays[0].len(),
                    ))))
                }),
            )
            .call(vec![deleted]);
        }
        let name = format!("gql_live_element_{data_type:?}").to_lowercase();
        create_udf(
            &name,
            vec![DataType::Boolean, data_type.clone()],
            data_type,
            Volatility::Immutable,
            Arc::new(|args| {
                let arrays = ColumnarValue::values_to_arrays(args)?;
                reject_deleted(&arrays[0])?;
                Ok(args[1].clone())
            }),
        )
        .call(vec![deleted, value])
    }
}

fn reject_deleted(flags: &dyn Array) -> datafusion::error::Result<()> {
    let flags = flags
        .as_any()
        .downcast_ref::<BooleanArray>()
        .ok_or_else(|| DataFusionError::Internal("expected boolean deleted flag".into()))?;
    if flags.iter().flatten().any(|deleted| deleted) {
        return Err(DataFusionError::Execution(
            "dereference of a deleted or unavailable graph element".into(),
        ));
    }
    Ok(())
}

#[derive(Clone, Default)]
pub(super) struct Bindings {
    pub scalars: CaseFoldMap<Column>,
    pub elements: CaseFoldMap<ElementBinding>,
    pub order: Vec<String>,
    pub conditional: CaseFoldSet,
    pub groups: CaseFoldSet,
    pub references: CaseFoldMap<super::references::ReferenceBinding>,
    pub sources: BTreeMap<ObjectId, std::sync::Arc<GraphData>>,
    pub domains: CaseFoldMap<super::references::Domain>,
    /// Element a `LET` name was copied from. Resolving that reference reads
    /// the source label set instead of every fragment of the element kind.
    pub reference_sources: CaseFoldMap<Vec<ElementBinding>>,
    pub fragment_cache: super::native::FragmentCache,
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
        if let Some(column) = self.scalars.get(name) {
            return column.clone();
        }
        let column = Column::new_unqualified(format!("__gf_scalar_{}", self.next));
        self.next += 1;
        self.scalars.insert(name, column.clone());
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
    let source = std::sync::Arc::new(graph.clone());
    scope.sources.insert(graph_id, source.clone());
    trace.sources.insert(graph_id, source);
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
    let (clause_filters, clause_push) = match &clause.where_clause {
        Some(predicate) if !where_runs_after_search(clause, session)? => {
            match super::native::clause_filters(session, plan.schema(), scope, predicate)? {
                Some(filters) => (filters, true),
                None => (CaseFoldMap::new(), false),
            }
        }
        _ => (CaseFoldMap::new(), false),
    };
    let mut compiler = super::patterns::Compiler {
        session,
        graph_id,
        data: graph,
        ctx,
        trace,
        different,
        predicates: Vec::new(),
        clause_filters,
        clause_push,
        clause_hit: CaseFoldSet::new(),
        match_filter: clause.where_clause.clone(),
    };
    let mut edge_lists: Vec<Expr> = Vec::new();
    for path in &clause.patterns {
        compiler.validate(path, None)?;
        let (_, selection) = super::paths::prefix(path.prefix.as_ref(), session)?;
        let (next, state) = compiler
            .pattern(
                plan,
                scope,
                path,
                None,
                None,
                true,
                path.alternation.is_none() && matches!(selection, super::paths::Selection::All),
            )
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
    // A pushed comparison only saw rows where that name was an element.
    // Conditional rows leave it null, and a repeated group exposes a list;
    // both still need the original predicate.
    compiler
        .clause_hit
        .retain(|name| !scope.conditional.contains(name) && !scope.groups.contains(name));
    let pushed = compiler.clause_push
        && compiler
            .clause_filters
            .keys()
            .all(|name| compiler.clause_hit.contains(name));
    if let Some(predicate) = compiler.match_filter.take() {
        if !pushed {
            // A selective path applies WHERE after ranking. Re-read mixed
            // fragments so the stringified column cannot satisfy the comparison.
            if let Some(filtered) =
                super::native::fragment_predicate(plan.clone(), scope, session, &predicate)?
            {
                plan = filtered;
            } else {
                let expr =
                    Binder::with_bindings(session, plan.schema(), scope).predicate(&predicate)?;
                plan = plan.filter(expr)?;
            }
        }
    }
    Ok(plan)
}

/// A selective path search applies the match `WHERE` after ranking. Pushing
/// that predicate into the scan would change which path wins.
fn where_runs_after_search(clause: &ast::MatchClause, session: &SessionState) -> Result<bool> {
    for path in &clause.patterns {
        if search_is_selective(path, session)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn search_is_selective(path: &ast::PathPattern, session: &SessionState) -> Result<bool> {
    if !matches!(
        super::paths::prefix(path.prefix.as_ref(), session)?.1,
        super::paths::Selection::All
    ) {
        return Ok(true);
    }
    if let Some(group) = &path.parenthesized {
        if search_is_selective(&group.pattern, session)? {
            return Ok(true);
        }
    }
    for factor in &path.factors {
        if factor_is_selective(factor, session)? {
            return Ok(true);
        }
    }
    for term in &path.alternatives {
        for factor in &term.factors {
            if factor_is_selective(factor, session)? {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn factor_is_selective(factor: &ast::PathPatternFactor, session: &SessionState) -> Result<bool> {
    match factor {
        ast::PathPatternFactor::Parenthesized(group) => {
            search_is_selective(&group.pattern, session)
        }
        ast::PathPatternFactor::Alternation { alternatives, .. } => {
            for branch in alternatives {
                for factor in branch {
                    if factor_is_selective(factor, session)? {
                        return Ok(true);
                    }
                }
            }
            Ok(false)
        }
        ast::PathPatternFactor::Node(_) | ast::PathPatternFactor::Relationship(_) => Ok(false),
    }
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
    scope.elements.insert(name, binding.clone());
    scope.order.push(name.into());
    Ok(())
}
#[allow(clippy::too_many_arguments)]
pub(super) fn node(
    mut plan: LogicalPlanBuilder,
    scope: &mut Bindings,
    graph_id: ObjectId,
    graph: &GraphData,
    pattern: &ast::NodePattern,
    endpoint: Option<Expr>,
    native: &NativePush,
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
        let (scan, binding) = scan_tables(
            scope,
            graph_id,
            graph,
            ElementKind::Node,
            None,
            None,
            label_filter(pattern.label_expression.as_ref(), &pattern.labels),
            None,
            native,
        )?;
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
            if let Some(reference) = super::references::scalar(scope, &name.value, plan.schema())? {
                plan = plan.filter(super::references::constraint(reference, &binding))?;
                scope.elements.insert(name.value.clone(), binding.clone());
            } else {
                declare(scope, &name.value, &binding)?;
            }
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
    native: &NativePush,
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
            if native.properties.contains(&name.value) {
                continue;
            }
            let columns = plan.schema().columns();
            let mut fragments = scope.clone();
            let (next, equality) =
                super::native::equated_property(plan, &mut fragments, binding, &name.value, value)?;
            if let Some(equality) = equality {
                plan = next
                    .filter(equality)?
                    .project(columns.into_iter().map(Expr::Column))?;
                continue;
            }
            plan = next;
            let right = Binder::with_bindings(session, plan.schema(), scope).bind(value)?;
            let equality = Binder::with_bindings(session, plan.schema(), scope)
                .equals(binding.property(&name.value), right)?;
            plan = plan.filter(equality)?;
        }
    }
    if let Some(predicate) = predicate.filter(|_| !native.pushed_where) {
        // Auxiliary fragment columns are projected away by fragment_predicate,
        // so this local allocation does not change the caller's bindings.
        let mut fragments = scope.clone();
        if let Some(filtered) =
            super::native::fragment_predicate(plan.clone(), &mut fragments, session, predicate)?
        {
            plan = filtered;
        } else {
            let expression =
                Binder::with_bindings(session, plan.schema(), scope).predicate(predicate)?;
            plan = plan.filter(expression)?;
        }
    }
    Ok(plan)
}

/// Label sets are uniform on each fragment, so a pattern can reject a fragment
/// before its rows or property schema are unified with any other type.
#[derive(Clone, Copy)]
pub(super) struct LabelFilter<'a> {
    pub expression: Option<&'a ast::LabelExpression>,
    pub labels: &'a [ast::Identifier],
}

pub(super) fn label_filter<'a>(
    expression: Option<&'a ast::LabelExpression>,
    labels: &'a [ast::Identifier],
) -> Option<LabelFilter<'a>> {
    (expression.is_some() || !labels.is_empty()).then_some(LabelFilter { expression, labels })
}

pub(super) fn scan(
    scope: &mut Bindings,
    graph_id: ObjectId,
    graph: &GraphData,
    kind: ElementKind,
    direction: Option<ast::Direction>,
) -> Result<(LogicalPlan, ElementBinding)> {
    scan_filtered(
        scope,
        graph_id,
        graph,
        kind,
        direction,
        None,
        &NativePush::default(),
    )
}

pub(super) fn scan_filtered(
    scope: &mut Bindings,
    graph_id: ObjectId,
    graph: &GraphData,
    kind: ElementKind,
    direction: Option<ast::Direction>,
    filter: Option<LabelFilter<'_>>,
    native: &NativePush,
) -> Result<(LogicalPlan, ElementBinding)> {
    scan_tables(
        scope, graph_id, graph, kind, direction, None, filter, None, native,
    )
}

pub(super) fn scan_insert(
    scope: &mut Bindings,
    graph_id: ObjectId,
    graph: &GraphData,
    kind: ElementKind,
    inserted: &Table,
    directed: bool,
) -> Result<(LogicalPlan, ElementBinding)> {
    scan_tables(
        scope,
        graph_id,
        graph,
        kind,
        None,
        Some((inserted, directed)),
        None,
        None,
        &NativePush::default(),
    )
}

/// Derive refresh metadata from the identities still carried by the working
/// table, rather than unrelated fragments adjacent to its previous label sets.
pub(super) fn refresh_source(
    data: &GraphData,
    before: &ElementBinding,
    ids: &BTreeSet<u64>,
) -> Result<ElementBinding> {
    let tables: Vec<_> = match before.kind {
        ElementKind::Node => data.nodes.iter().map(|table| &table.0).collect(),
        ElementKind::Edge => data.edges.iter().map(|table| &table.table).collect(),
    };
    let mut selected = Vec::new();
    for table in tables {
        if table.contains_any(ids)? {
            selected.push(table);
        }
    }
    let mut source = before.clone();
    if !selected.is_empty() {
        source.properties = graph::property_types(selected.iter().copied())?;
        source.source_labels = selected.iter().map(|table| table.labels.clone()).collect();
    }
    Ok(source)
}

/// Re-read an element after a write without adopting another type's schema.
pub(super) fn scan_refresh(
    scope: &mut Bindings,
    graph_id: ObjectId,
    graph: &GraphData,
    before: &ElementBinding,
) -> Result<(LogicalPlan, ElementBinding)> {
    scan_tables(
        scope,
        graph_id,
        graph,
        before.kind,
        None,
        None,
        None,
        before.constrained.then_some(before),
        &NativePush::default(),
    )
}

#[allow(clippy::too_many_arguments)]
fn scan_tables(
    scope: &mut Bindings,
    graph_id: ObjectId,
    graph: &GraphData,
    kind: ElementKind,
    direction: Option<ast::Direction>,
    inserted: Option<(&Table, bool)>,
    filter: Option<LabelFilter<'_>>,
    refresh_of: Option<&ElementBinding>,
    native: &NativePush,
) -> Result<(LogicalPlan, ElementBinding)> {
    let all_tables: Vec<(&Table, bool)> = match kind {
        ElementKind::Node => graph.nodes.iter().map(|table| (&table.0, false)).collect(),
        ElementKind::Edge => graph
            .edges
            .iter()
            .map(|table| (&table.table, table.directed))
            .collect(),
    };
    let matched: Vec<_> = all_tables
        .iter()
        .copied()
        .filter(|(table, _)| match refresh_of {
            Some(before) => refreshes(table, before),
            None => fragment_matches(&table.labels, filter),
        })
        .collect();
    // An insert binding is the new row's own layout. Unifying it with every
    // other fragment would stringify a typed property whenever another label
    // stored the same name differently.
    let scanned: Vec<_> = match inserted {
        Some(table) => vec![table],
        None => matched,
    };
    // A label that matches nothing still names the element kind's properties, so
    // an empty aggregate does not invent a group from a constant null. The
    // columns stay untyped: copying another label's type makes `n.id = 1` fail
    // when some unrelated fragment stores that name as a string.
    for (table, _) in &scanned {
        graph::validate_query_property_names(table.schema.as_ref())?;
    }
    let properties = if let Some(before) = refresh_of {
        before.properties.clone()
    } else if scanned.is_empty() && inserted.is_none() {
        let mut properties = BTreeMap::new();
        for (table, _) in &all_tables {
            for field in table.schema.fields() {
                if !graph::is_reserved_property(field.name()) {
                    properties
                        .entry(field.name().clone())
                        .or_insert(DataType::Null);
                }
            }
        }
        properties
    } else {
        graph::property_types(scanned.iter().map(|(table, _)| *table))?
    };
    if let Some(atom) = &native.filter {
        super::native::validate_kind(atom, kind)?;
        super::native::validate(atom, scanned.iter().map(|(table, _)| *table))?;
    }
    // A refresh also reads a one-label-apart fragment so a label edit can find
    // the moved row. That neighbor must not contribute property names or types:
    // unifying it would stringify a value the pattern had already excluded.
    let (properties, source_labels, labels) = if let Some(before) = refresh_of {
        let exact = scanned.iter().filter(|(table, _)| {
            before
                .source_labels
                .iter()
                .any(|labels| labels == &table.labels)
        });
        let mut properties = before.properties.clone();
        for (name, data_type) in graph::property_types(exact.map(|(table, _)| *table))? {
            properties.entry(name).or_insert(data_type);
        }
        let labels = if scanned.is_empty() {
            before
                .source_labels
                .iter()
                .flatten()
                .cloned()
                .collect::<BTreeSet<_>>()
        } else {
            scanned
                .iter()
                .flat_map(|(table, _)| table.labels.iter().cloned())
                .collect::<BTreeSet<_>>()
        };
        (properties, before.source_labels.clone(), labels)
    } else {
        let source_labels: Vec<_> = scanned
            .iter()
            .map(|(table, _)| table.labels.clone())
            .collect();
        let labels = source_labels.iter().flatten().cloned().collect();
        (properties, source_labels, labels)
    };
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
        source_labels,
        constrained: filter.is_some() || inserted.is_some() || refresh_of.is_some(),
        tombstones: BTreeSet::new(),
        prior_id: None,
    };
    scope.next += 1;
    // Each scan needs its own qualifier. Joining two leaves that are both
    // named `__gf_scan` makes `__gf_id` ambiguous once a graph has one table
    // per element kind.
    let scan_name = format!("__gf_scan_{}", scope.next);
    let mut branches = Vec::new();
    for (table, directed) in scanned {
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
                &scan_name,
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
            if let Some(atom) = &native.filter {
                scan = scan.filter(super::native::compile_fragment(
                    atom,
                    table.schema.as_ref(),
                    super::native::FragmentMeta {
                        labels: Some(&table.labels),
                        directed: (kind == ElementKind::Edge).then_some(directed),
                        identity: Some((graph_id, kind)),
                    },
                )?)?;
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
                let field = graph::property_field(table.schema.as_ref(), name);
                // A neighbor fragment can store the same name differently. Cast
                // only a widening that fits; an incompatible value stays null
                // instead of failing the rows this fragment does not describe.
                let compatible = field.is_some_and(|field| {
                    field.data_type() == data_type
                        || graph::property_fits(field.data_type(), data_type)
                });
                projections.push(if compatible {
                    if field.is_some_and(|field| field.data_type() == data_type) {
                        col(field.expect("compatible property").name()).alias(name)
                    } else {
                        cast(
                            col(field.expect("compatible property").name()),
                            data_type.clone(),
                        )
                        .alias(name)
                    }
                } else {
                    lit(ScalarValue::try_from(data_type)?).alias(name)
                });
                projections.push(lit(compatible).alias(&binding.present[name]));
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
/// Exact label sets define the binding schema. A one-label-apart fragment is
/// included only so a label edit can find the moved row.
fn refreshes(table: &Table, before: &ElementBinding) -> bool {
    if before
        .source_labels
        .iter()
        .any(|labels| labels == &table.labels)
    {
        return true;
    }
    let neighbor = before
        .source_labels
        .iter()
        .any(|labels| one_label_apart(labels, &table.labels));
    neighbor && shares_property(table, before) && properties_fit(table, before)
}

fn one_label_apart(left: &BTreeSet<String>, right: &BTreeSet<String>) -> bool {
    let (larger, smaller) = if left.len() >= right.len() {
        (left, right)
    } else {
        (right, left)
    };
    larger.len() == smaller.len() + 1 && smaller.is_subset(larger)
}

fn shares_property(table: &Table, before: &ElementBinding) -> bool {
    table.schema.fields().iter().any(|field| {
        !graph::is_reserved_property(field.name())
            && before
                .properties
                .keys()
                .any(|stored| stored.eq_ignore_ascii_case(field.name()))
    })
}

fn properties_fit(table: &Table, before: &ElementBinding) -> bool {
    table.schema.fields().iter().all(|field| {
        graph::is_reserved_property(field.name())
            || before
                .properties
                .iter()
                .find(|(stored, _)| stored.eq_ignore_ascii_case(field.name()))
                .is_none_or(|(_, bound)| graph::property_fits(field.data_type(), bound))
    })
}

fn fragment_matches(labels: &BTreeSet<String>, filter: Option<LabelFilter<'_>>) -> bool {
    let Some(filter) = filter else {
        return true;
    };
    if let Some(expression) = filter.expression {
        return label_holds(labels, expression);
    }
    filter
        .labels
        .iter()
        .all(|label| labels.contains(&label.value))
}

pub(super) fn label_holds(labels: &BTreeSet<String>, expression: &ast::LabelExpression) -> bool {
    match expression {
        ast::LabelExpression::Label(name) => labels.contains(&name.value),
        ast::LabelExpression::Wildcard => !labels.is_empty(),
        ast::LabelExpression::And(left, right) => {
            label_holds(labels, left) && label_holds(labels, right)
        }
        ast::LabelExpression::Or(left, right) => {
            label_holds(labels, left) || label_holds(labels, right)
        }
        ast::LabelExpression::Not(inner) => !label_holds(labels, inner),
        ast::LabelExpression::Parenthesized(inner) => label_holds(labels, inner),
    }
}

fn col(name: &str) -> Expr {
    Expr::Column(Column::new_unqualified(name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{graph::NodeTable, transaction::StatementTxn, Database};
    use datafusion::arrow::{
        array::{Int64Array, UInt64Array},
        datatypes::{Field, Schema},
        record_batch::RecordBatch,
    };

    #[tokio::test]
    async fn ambiguous_imported_property_case_does_not_lose_columns_on_update() {
        let db = Database::new();
        let mut s = db.session();
        s.execute("CREATE GRAPH g ANY; SESSION SET GRAPH g")
            .unwrap();
        let id = s.state().current_graph.unwrap();
        let schema = Arc::new(Schema::new(vec![
            Field::new(crate::graph::ID, DataType::UInt64, false),
            Field::new("age", DataType::Int64, false),
            Field::new("AGE", DataType::Int64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(UInt64Array::from(vec![1])),
                Arc::new(Int64Array::from(vec![10])),
                Arc::new(Int64Array::from(vec![20])),
            ],
        )
        .unwrap();
        let committed = s
            .replace_graph_data(
                GraphData::try_new(
                    vec![NodeTable::try_new(vec!["N".into()], schema, vec![batch]).unwrap()],
                    vec![],
                )
                .unwrap(),
            )
            .unwrap();
        let result = s.run("MATCH(n) SET n.age=99").await;
        assert!(
            matches!(result,Err(Error::InvalidQuery(ref message)) if message.contains("ambiguous")),
            "{result:?}"
        );
        assert!(s.query("MATCH(n) RETURN n.age").await.is_err());
        let mut read = StatementTxn::begin(&db).unwrap();
        assert_eq!(read.base.commit_seq, committed);
        let data = read.graph_data(id).unwrap();
        let batch = &data.nodes[0].0.batches[0];
        assert_eq!(
            batch
                .schema()
                .fields()
                .iter()
                .map(|f| f.name().clone())
                .collect::<Vec<_>>(),
            vec![crate::graph::ID.to_string(), "age".into(), "AGE".into()]
        );
        for (name, value) in [("age", 10), ("AGE", 20)] {
            assert_eq!(
                batch
                    .column_by_name(name)
                    .unwrap()
                    .as_any()
                    .downcast_ref::<Int64Array>()
                    .unwrap()
                    .value(0),
                value
            );
        }
    }
}
