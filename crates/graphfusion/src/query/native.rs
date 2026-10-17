//! Compare a pattern predicate on each fragment before property types are unified.
//!
//! A multi-type scan casts every fragment into one Arrow column. Doing the
//! comparison after that cast rejects an integer literal against a stringified
//! column, and casting both sides to strings would make `10 > 9` false.
//! Each fragment therefore keeps its stored type. A missing property, or a
//! stored type that cannot be compared with the literal, is unknown: `NOT`
//! unknown stays unknown and does not keep the row.

use super::{
    expressions::Binder,
    graph::{Bindings, ElementKind},
};
use crate::{
    gql as ast,
    graph::{Table, ID},
    Error, Result, SessionState,
};
use datafusion::{
    arrow::datatypes::{DataType, Schema, SchemaRef},
    common::{Column, DFSchema, ScalarValue, TableReference},
    datasource::provider_as_source,
    logical_expr::{cast, lit, Expr, ExprSchemable, JoinType, LogicalPlanBuilder, Operator},
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug)]
pub(super) enum NativeAtom {
    And(Box<NativeAtom>, Box<NativeAtom>),
    Or(Box<NativeAtom>, Box<NativeAtom>),
    Not(Box<NativeAtom>),
    Compare {
        property: String,
        op: Operator,
        value: Expr,
        value_type: DataType,
    },
}

#[derive(Clone, Debug, Default)]
pub(super) struct NativePush {
    pub filter: Option<NativeAtom>,
    /// Property-map keys already applied on each fragment.
    pub properties: BTreeSet<String>,
    /// The pattern `WHERE` was applied on each fragment, so the unified
    /// predicate must not bind it again.
    pub pushed_where: bool,
}

impl NativePush {
    pub(super) fn prepare(
        session: &SessionState,
        schema: &DFSchema,
        scope: &Bindings,
        variable: Option<&str>,
        properties: Option<&ast::MapLiteral>,
        where_clause: Option<&ast::Expr>,
    ) -> Result<Self> {
        let mut pushed = Self::default();
        let mut atoms = Vec::new();
        if let Some(map) = properties {
            for (name, value) in &map.entries {
                let Some((expr, value_type)) = bind_closed(session, schema, scope, value)? else {
                    continue;
                };
                pushed.properties.insert(name.value.clone());
                atoms.push(NativeAtom::Compare {
                    property: name.value.clone(),
                    op: Operator::Eq,
                    value: expr,
                    value_type,
                });
            }
        }
        if let Some(variable) = variable {
            if let Some(where_clause) = where_clause {
                if let Some(local) = local_tree(where_clause, variable) {
                    if let Some(atom) = bind_local(session, schema, scope, &local)? {
                        atoms.push(atom);
                        pushed.pushed_where = true;
                    }
                }
            }
        }
        pushed.filter = conjoin(atoms);
        Ok(pushed)
    }

    pub(super) fn conjunct(&mut self, extra: NativeAtom) {
        self.filter = Some(match self.filter.take() {
            Some(current) => NativeAtom::And(Box::new(current), Box::new(extra)),
            None => extra,
        });
    }
}

/// Split a match-level `WHERE` into per-variable fragment predicates.
///
/// `AND` can be split across variables. `OR` and `NOT` stay together, so they
/// are pushed only when every comparison names the same variable. A value that
/// reads another column stays on the unified plan.
pub(super) fn clause_filters(
    session: &SessionState,
    schema: &DFSchema,
    scope: &Bindings,
    expr: &ast::Expr,
) -> Result<Option<BTreeMap<String, NativeAtom>>> {
    let Some(parts) = partition(expr) else {
        return Ok(None);
    };
    let mut filters = BTreeMap::new();
    for (variable, local) in parts {
        let Some(atom) = bind_local(session, schema, scope, &local)? else {
            return Ok(None);
        };
        filters.insert(variable, atom);
    }
    Ok((!filters.is_empty()).then_some(filters))
}

/// A comparison that no selected fragment can evaluate is a type error, matching
/// a single-type filter. Fragments that can evaluate it still run natively, and
/// the others stay unknown.
pub(super) fn validate<'a>(
    atom: &NativeAtom,
    tables: impl IntoIterator<Item = &'a Table>,
) -> Result<()> {
    let tables: Vec<&Table> = tables.into_iter().collect();
    validate_atom(atom, &tables)
}

fn validate_atom(atom: &NativeAtom, tables: &[&Table]) -> Result<()> {
    match atom {
        NativeAtom::And(left, right) | NativeAtom::Or(left, right) => {
            validate_atom(left, tables)?;
            validate_atom(right, tables)
        }
        NativeAtom::Not(inner) => validate_atom(inner, tables),
        NativeAtom::Compare {
            property,
            value_type,
            ..
        } => {
            let mut stored = None;
            for table in tables {
                let Some(field) = table.schema.field_with_name(property).ok() else {
                    continue;
                };
                if comparable(field.data_type(), value_type) {
                    return Ok(());
                }
                stored.get_or_insert_with(|| field.data_type().clone());
            }
            match stored {
                Some(stored) => Err(Error::InvalidQuery(format!(
                    "incomparable types {stored} and {value_type}"
                ))),
                None => Ok(()),
            }
        }
    }
}

pub(super) fn compile(atom: &NativeAtom, schema: &Schema) -> Result<Expr> {
    Ok(match atom {
        NativeAtom::And(left, right) => compile(left, schema)?.and(compile(right, schema)?),
        NativeAtom::Or(left, right) => compile(left, schema)?.or(compile(right, schema)?),
        NativeAtom::Not(inner) => Expr::Not(Box::new(compile(inner, schema)?)),
        NativeAtom::Compare {
            property,
            op,
            value,
            value_type,
        } => compare_property(schema, property, *op, value.clone(), value_type)?,
    })
}

fn compare_property(
    schema: &Schema,
    property: &str,
    op: Operator,
    value: Expr,
    value_type: &DataType,
) -> Result<Expr> {
    let Some(stored) = schema
        .field_with_name(property)
        .ok()
        .map(|field| field.data_type().clone())
    else {
        return Ok(unknown());
    };
    if !comparable(&stored, value_type) {
        return Ok(unknown());
    }
    let mut left = column(property);
    let mut right = value;
    if stored != *value_type && stored.is_numeric() && value_type.is_numeric() {
        left = cast(left, DataType::Float64);
        right = cast(right, DataType::Float64);
    }
    Ok(match op {
        Operator::Eq => left.eq(right),
        Operator::NotEq => left.not_eq(right),
        Operator::Lt => left.lt(right),
        Operator::LtEq => left.lt_eq(right),
        Operator::Gt => left.gt(right),
        Operator::GtEq => left.gt_eq(right),
        _ => unknown(),
    })
}

fn comparable(stored: &DataType, value: &DataType) -> bool {
    stored == value
        || *stored == DataType::Null
        || *value == DataType::Null
        || (stored.is_numeric() && value.is_numeric())
}

fn unknown() -> Expr {
    lit(ScalarValue::Boolean(None))
}

fn column(name: &str) -> Expr {
    Expr::Column(Column::new_unqualified(name))
}

fn conjoin(atoms: Vec<NativeAtom>) -> Option<NativeAtom> {
    atoms
        .into_iter()
        .reduce(|left, right| NativeAtom::And(Box::new(left), Box::new(right)))
}

#[derive(Clone)]
enum Local {
    And(Box<Local>, Box<Local>),
    Or(Box<Local>, Box<Local>),
    Not(Box<Local>),
    Atom {
        property: String,
        op: Operator,
        value: ast::Expr,
    },
}

fn partition(expr: &ast::Expr) -> Option<BTreeMap<String, Local>> {
    if let ast::Expr::Binary {
        left,
        op: ast::BinaryOp::And,
        right,
    } = expr
    {
        let mut combined = partition(left)?;
        for (name, local) in partition(right)? {
            if let Some(existing) = combined.remove(&name) {
                combined.insert(name, Local::And(Box::new(existing), Box::new(local)));
            } else {
                combined.insert(name, local);
            }
        }
        return Some(combined);
    }
    let variable = sole_variable(expr)?;
    Some(BTreeMap::from([(
        variable.clone(),
        local_tree(expr, &variable)?,
    )]))
}

fn sole_variable(expr: &ast::Expr) -> Option<String> {
    let mut names = BTreeSet::new();
    property_bases(expr, &mut names);
    if names.len() == 1 {
        names.into_iter().next()
    } else {
        None
    }
}

fn local_tree(expr: &ast::Expr, variable: &str) -> Option<Local> {
    match expr {
        ast::Expr::Binary {
            left,
            op: ast::BinaryOp::And,
            right,
        } => Some(Local::And(
            Box::new(local_tree(left, variable)?),
            Box::new(local_tree(right, variable)?),
        )),
        ast::Expr::Binary {
            left,
            op: ast::BinaryOp::Or,
            right,
        } => Some(Local::Or(
            Box::new(local_tree(left, variable)?),
            Box::new(local_tree(right, variable)?),
        )),
        ast::Expr::Unary {
            op: ast::UnaryOp::Not,
            expr,
        } => Some(Local::Not(Box::new(local_tree(expr, variable)?))),
        other => comparison(other, variable).map(|(property, op, value)| Local::Atom {
            property,
            op,
            value,
        }),
    }
}

fn comparison(expr: &ast::Expr, variable: &str) -> Option<(String, Operator, ast::Expr)> {
    let ast::Expr::Binary { left, op, right } = expr else {
        return None;
    };
    let op = match op {
        ast::BinaryOp::Eq => Operator::Eq,
        ast::BinaryOp::Neq => Operator::NotEq,
        ast::BinaryOp::Lt => Operator::Lt,
        ast::BinaryOp::Le => Operator::LtEq,
        ast::BinaryOp::Gt => Operator::Gt,
        ast::BinaryOp::Ge => Operator::GtEq,
        _ => return None,
    };
    if let Some(property) = property_of(left, variable) {
        if mentions(right, variable) {
            return None;
        }
        return Some((property, op, right.as_ref().clone()));
    }
    if let Some(property) = property_of(right, variable) {
        if mentions(left, variable) {
            return None;
        }
        return Some((property, flip(op), left.as_ref().clone()));
    }
    None
}

fn property_of(expr: &ast::Expr, variable: &str) -> Option<String> {
    let ast::Expr::Property { base, key } = expr else {
        return None;
    };
    let ast::Expr::Identifier(name) = base.as_ref() else {
        return None;
    };
    (name.value == variable).then(|| key.value.clone())
}

fn flip(op: Operator) -> Operator {
    match op {
        Operator::Lt => Operator::Gt,
        Operator::Gt => Operator::Lt,
        Operator::LtEq => Operator::GtEq,
        Operator::GtEq => Operator::LtEq,
        other => other,
    }
}

fn bind_local(
    session: &SessionState,
    schema: &DFSchema,
    scope: &Bindings,
    local: &Local,
) -> Result<Option<NativeAtom>> {
    Ok(match local {
        Local::And(left, right) => match (
            bind_local(session, schema, scope, left)?,
            bind_local(session, schema, scope, right)?,
        ) {
            (Some(left), Some(right)) => Some(NativeAtom::And(Box::new(left), Box::new(right))),
            _ => None,
        },
        Local::Or(left, right) => match (
            bind_local(session, schema, scope, left)?,
            bind_local(session, schema, scope, right)?,
        ) {
            (Some(left), Some(right)) => Some(NativeAtom::Or(Box::new(left), Box::new(right))),
            _ => None,
        },
        Local::Not(inner) => {
            bind_local(session, schema, scope, inner)?.map(|inner| NativeAtom::Not(Box::new(inner)))
        }
        Local::Atom {
            property,
            op,
            value,
        } => bind_closed(session, schema, scope, value)?.map(|(expr, value_type)| {
            NativeAtom::Compare {
                property: property.clone(),
                op: *op,
                value: expr,
                value_type,
            }
        }),
    })
}

/// Bind a value that does not read the working row. An unbound name or a
/// column reference is left for the unified predicate, which can see variables
/// declared later in the same pattern.
fn bind_closed(
    session: &SessionState,
    schema: &DFSchema,
    scope: &Bindings,
    value: &ast::Expr,
) -> Result<Option<(Expr, DataType)>> {
    let Ok(expr) = Binder::with_bindings(session, schema, scope).bind(value) else {
        return Ok(None);
    };
    if !expr.column_refs().is_empty() {
        return Ok(None);
    }
    let value_type = expr.get_type(schema)?;
    Ok(Some((expr, value_type)))
}

fn property_bases(expr: &ast::Expr, names: &mut BTreeSet<String>) {
    if let ast::Expr::Property { base, .. } = expr {
        if let ast::Expr::Identifier(name) = base.as_ref() {
            names.insert(name.value.clone());
        }
    }
    for child in children(expr) {
        property_bases(child, names);
    }
}

fn mentions(expr: &ast::Expr, variable: &str) -> bool {
    opaque(expr)
        || named_variable(expr, variable)
        || children(expr)
            .into_iter()
            .any(|child| mentions(child, variable))
}

/// Nested queries are not walked. Treat them as depending on the filtered
/// variable so the comparison stays on the unified plan.
fn opaque(expr: &ast::Expr) -> bool {
    matches!(
        expr,
        ast::Expr::GraphReference { .. }
            | ast::Expr::BindingTableReference { .. }
            | ast::Expr::ValueQuery { .. }
            | ast::Expr::Exists { .. }
            | ast::Expr::ExistsQuery { .. }
            | ast::Expr::ExistsMatch { .. }
    )
}

fn named_variable(expr: &ast::Expr, variable: &str) -> bool {
    match expr {
        ast::Expr::Identifier(name)
        | ast::Expr::PropertyExists { variable: name, .. }
        | ast::Expr::ElementId { variable: name }
        | ast::Expr::IsDirected { variable: name, .. }
        | ast::Expr::IsLabeled { variable: name, .. } => name.value == variable,
        ast::Expr::Same { variables } | ast::Expr::AllDifferent { variables } => {
            variables.iter().any(|name| name.value == variable)
        }
        ast::Expr::SourceDestination { node, edge, .. } => {
            node.value == variable || edge.value == variable
        }
        _ => false,
    }
}

fn children(expr: &ast::Expr) -> Vec<&ast::Expr> {
    use ast::Expr as E;
    match expr {
        E::Property { base, .. } => vec![base],
        E::Unary { expr, .. }
        | E::IsTyped { expr, .. }
        | E::IsNull { expr, .. }
        | E::IsUnknown { expr, .. }
        | E::IsTruth { expr, .. }
        | E::IsNormalized { expr, .. }
        | E::AbsoluteValue { expr }
        | E::DurationFunction { value: expr }
        | E::Cast { expr, .. }
        | E::Elements { path: expr }
        | E::StringLength { value: expr, .. }
        | E::Fold { value: expr, .. }
        | E::Normalize { value: expr, .. } => vec![expr],
        E::Binary { left, right, .. }
        | E::DurationBetween { left, right }
        | E::NullIf { left, right } => vec![left, right],
        E::Function { args, .. }
        | E::NumericFunction { args, .. }
        | E::Coalesce(args)
        | E::List(args)
        | E::Path(args) => args.iter().collect(),
        E::TypedList { values, .. } => values.iter().collect(),
        E::SubstringByLength { value, length, .. } => vec![value, length],
        E::Trim {
            trim_character,
            value,
            ..
        } => trim_character
            .as_ref()
            .map(|expr| vec![expr.as_ref(), value])
            .unwrap_or_else(|| vec![value]),
        E::MultiTrim {
            value,
            trim_characters,
            ..
        } => trim_characters
            .as_ref()
            .map(|expr| vec![value, expr.as_ref()])
            .unwrap_or_else(|| vec![value]),
        E::TrimList { value, count } => vec![value, count],
        E::DateTimeFunction { value, .. } => value.iter().map(Box::as_ref).collect(),
        E::Let { items, value } => items
            .iter()
            .map(|item| &item.value)
            .chain(std::iter::once(value.as_ref()))
            .collect(),
        E::Case {
            operand,
            when_clauses,
            else_expr,
        } => operand
            .as_ref()
            .map(Box::as_ref)
            .into_iter()
            .chain(when_clauses.iter().flat_map(|when| {
                std::iter::once(&when.condition)
                    .chain(&when.additional_operands)
                    .chain(std::iter::once(&when.result))
            }))
            .chain(else_expr.as_ref().map(Box::as_ref))
            .collect(),
        E::Record { fields, .. } | E::Map(fields) => {
            fields.entries.iter().map(|(_, value)| value).collect()
        }
        // Nested queries and bare literals do not contribute expression children.
        // `mentions` treats an unsupported nested form as closed unless a child
        // identifier names the variable; callers only push closed values.
        E::GraphReference { .. }
        | E::BindingTableReference { .. }
        | E::Parameter(_)
        | E::Literal(_)
        | E::ValueQuery { .. }
        | E::Exists { .. }
        | E::ExistsQuery { .. }
        | E::ExistsMatch { .. }
        | E::CurrentDate
        | E::CurrentTime { .. }
        | E::CurrentTimestamp { .. }
        | E::CurrentUser
        | E::LocalTime { .. }
        | E::LocalTimestamp { .. }
        | E::Wildcard
        | E::Same { .. }
        | E::AllDifferent { .. }
        | E::SourceDestination { .. }
        | E::PropertyExists { .. }
        | E::ElementId { .. }
        | E::IsDirected { .. }
        | E::IsLabeled { .. }
        | E::Identifier(_) => vec![],
    }
}

/// Evaluate FILTER arithmetic and concatenation on each fragment's stored type.
///
/// A plain comparison stays on the unified working table. An operator around a
/// mixed property otherwise sees that text column: addition rejects it, and
/// concatenation matches a stringified integer. Each fragment is bound with
/// its own stored type. A fragment that cannot apply the operator is unknown,
/// and the operator is a type error only when no fragment can.
pub(super) fn apply_filter_operator(
    mut plan: LogicalPlanBuilder,
    session: &SessionState,
    scope: &Bindings,
    predicate: &ast::Expr,
) -> Result<LogicalPlanBuilder> {
    let elements = nested_mixed_elements(predicate, scope);
    if elements.is_empty() {
        let expr = Binder::with_bindings(session, plan.schema(), scope).predicate(predicate)?;
        return Ok(plan.filter(expr)?);
    }
    let mut stored = Vec::new();
    for name in &elements {
        let (next, fragments) = attach_operator_fragments(plan, scope, name)?;
        plan = next;
        if fragments.is_empty() {
            let expr = Binder::with_bindings(session, plan.schema(), scope).predicate(predicate)?;
            return Ok(plan.filter(expr)?);
        }
        stored.push((name.clone(), fragments));
    }
    let contexts = fragment_contexts(&stored);
    let leaves = boolean_leaves(predicate);
    let mut leaf_ok = vec![false; leaves.len()];
    let mut leaf_err = (0..leaves.len()).map(|_| None).collect::<Vec<_>>();
    let mut evaluated = Vec::new();
    for context in &contexts {
        let mut values = Vec::new();
        for (index, leaf) in leaves.iter().enumerate() {
            match bind_leaf(session, plan.schema(), scope, context, leaf) {
                Ok(expr) => {
                    leaf_ok[index] = true;
                    values.push(expr);
                }
                Err(err @ (Error::InvalidQuery(_) | Error::UnsupportedFeature(_))) => {
                    if leaf_err[index].is_none() {
                        leaf_err[index] = Some(err);
                    }
                    values.push(unknown());
                }
                Err(err) => return Err(err),
            }
        }
        let mut index = 0;
        evaluated.push((
            context_condition(context),
            assemble_predicate(predicate, &values, &mut index, plan.schema())?,
        ));
    }
    for (index, ok) in leaf_ok.iter().enumerate() {
        if !ok {
            return Err(leaf_err[index].take().unwrap_or_else(|| {
                Error::InvalidQuery("predicate does not match a stored type".into())
            }));
        }
    }
    let expr = fragment_case(evaluated);
    expect_boolean(&expr, plan.schema())?;
    Ok(plan.filter(expr)?)
}

fn nested_mixed_elements(expr: &ast::Expr, scope: &Bindings) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    walk_nested(expr, scope, false, &mut names);
    names
}

fn walk_nested(
    expr: &ast::Expr,
    scope: &Bindings,
    comparison_side: bool,
    names: &mut BTreeSet<String>,
) {
    if let Some((name, property)) = bare_element_property(expr) {
        if !comparison_side
            && scope.elements.contains_key(&name)
            && stored_type_count(scope, &name, &property) > 1
        {
            names.insert(name);
        }
        return;
    }
    if let ast::Expr::Binary { left, op, right } = expr {
        if is_comparison(op) {
            walk_nested(left, scope, bare_element_property(left).is_some(), names);
            walk_nested(right, scope, bare_element_property(right).is_some(), names);
            return;
        }
    }
    for child in children(expr) {
        walk_nested(child, scope, false, names);
    }
}

fn bare_element_property(expr: &ast::Expr) -> Option<(String, String)> {
    let ast::Expr::Property { base, key } = expr else {
        return None;
    };
    let ast::Expr::Identifier(name) = base.as_ref() else {
        return None;
    };
    Some((name.value.clone(), key.value.clone()))
}

fn is_comparison(op: &ast::BinaryOp) -> bool {
    matches!(
        op,
        ast::BinaryOp::Eq
            | ast::BinaryOp::Neq
            | ast::BinaryOp::Lt
            | ast::BinaryOp::Le
            | ast::BinaryOp::Gt
            | ast::BinaryOp::Ge
    )
}

fn boolean_leaves(expr: &ast::Expr) -> Vec<&ast::Expr> {
    match expr {
        ast::Expr::Binary {
            left,
            op: ast::BinaryOp::And | ast::BinaryOp::Or | ast::BinaryOp::Xor,
            right,
        } => boolean_leaves(left)
            .into_iter()
            .chain(boolean_leaves(right))
            .collect(),
        ast::Expr::Unary {
            op: ast::UnaryOp::Not,
            expr,
        } => boolean_leaves(expr),
        other => vec![other],
    }
}

#[derive(Clone)]
struct OperatorFragment {
    scan: String,
    schema: SchemaRef,
}

impl OperatorFragment {
    fn id(&self) -> Expr {
        Expr::Column(Column::new(
            Some(TableReference::bare(self.scan.as_str())),
            ID,
        ))
    }
}

fn stored_type_count(scope: &Bindings, name: &str, property: &str) -> usize {
    let mut types = Vec::new();
    for table in operator_tables(scope, name) {
        let Some(data_type) = table
            .schema
            .field_with_name(property)
            .ok()
            .map(|field| field.data_type().clone())
        else {
            continue;
        };
        if !types.contains(&data_type) {
            types.push(data_type);
        }
    }
    types.len()
}

fn operator_tables<'a>(scope: &'a Bindings, name: &str) -> Vec<&'a Table> {
    let Some(binding) = scope.elements.get(name) else {
        return Vec::new();
    };
    let Some(data) = scope.sources.get(&binding.graph) else {
        return Vec::new();
    };
    let selected = |table: &Table| {
        binding
            .source_labels
            .iter()
            .any(|labels| labels == &table.labels)
    };
    match binding.kind {
        ElementKind::Node => data
            .nodes
            .iter()
            .map(|table| &table.0)
            .filter(|table| selected(table))
            .collect(),
        ElementKind::Edge => data
            .edges
            .iter()
            .map(|table| &table.table)
            .filter(|table| selected(table))
            .collect(),
    }
}

fn attach_operator_fragments(
    mut plan: LogicalPlanBuilder,
    scope: &Bindings,
    name: &str,
) -> Result<(LogicalPlanBuilder, Vec<OperatorFragment>)> {
    let Some(binding) = scope.elements.get(name) else {
        return Ok((plan, Vec::new()));
    };
    let mut stored = Vec::new();
    for (index, table) in operator_tables(scope, name).into_iter().enumerate() {
        let scan_name = format!("__gf_filter_op_{}_{index}", binding.alias);
        let scan =
            LogicalPlanBuilder::scan(&scan_name, provider_as_source(table.provider.clone()), None)?
                .build()?;
        plan = plan.join_on(
            scan,
            JoinType::Left,
            [binding.column(ID).eq(Expr::Column(Column::new(
                Some(TableReference::bare(scan_name.as_str())),
                ID,
            )))],
        )?;
        stored.push(OperatorFragment {
            scan: scan_name,
            schema: table.schema.clone(),
        });
    }
    Ok((plan, stored))
}

fn fragment_contexts(
    stored: &[(String, Vec<OperatorFragment>)],
) -> Vec<Vec<(String, OperatorFragment)>> {
    let mut contexts = vec![Vec::new()];
    for (name, fragments) in stored {
        let mut next = Vec::new();
        for context in &contexts {
            for fragment in fragments {
                let mut row = context.clone();
                row.push((name.clone(), fragment.clone()));
                next.push(row);
            }
        }
        contexts = next;
    }
    contexts
}

fn bind_leaf(
    session: &SessionState,
    schema: &DFSchema,
    scope: &Bindings,
    context: &[(String, OperatorFragment)],
    leaf: &ast::Expr,
) -> Result<Expr> {
    let mut local = scope.clone();
    for (name, fragment) in context {
        let Some(binding) = local.elements.get_mut(name) else {
            continue;
        };
        binding.property_scan = Some(fragment.scan.clone());
        binding.properties = fragment
            .schema
            .fields()
            .iter()
            .filter(|field| !field.name().starts_with("__gf_"))
            .map(|field| (field.name().clone(), field.data_type().clone()))
            .collect();
    }
    Binder::with_bindings(session, schema, &local).predicate(leaf)
}

fn context_condition(context: &[(String, OperatorFragment)]) -> Expr {
    context
        .iter()
        .map(|(_, fragment)| fragment.id().is_not_null())
        .reduce(Expr::and)
        .unwrap_or_else(|| lit(true))
}

fn assemble_predicate(
    expr: &ast::Expr,
    values: &[Expr],
    index: &mut usize,
    schema: &DFSchema,
) -> Result<Expr> {
    match expr {
        ast::Expr::Binary {
            left,
            op: op @ (ast::BinaryOp::And | ast::BinaryOp::Or | ast::BinaryOp::Xor),
            right,
        } => {
            let left = assemble_predicate(left, values, index, schema)?;
            let right = assemble_predicate(right, values, index, schema)?;
            expect_boolean(&left, schema)?;
            expect_boolean(&right, schema)?;
            Ok(match op {
                ast::BinaryOp::And => left.and(right),
                ast::BinaryOp::Or => left.or(right),
                _ => left.not_eq(right),
            })
        }
        ast::Expr::Unary {
            op: ast::UnaryOp::Not,
            expr,
        } => {
            let inner = assemble_predicate(expr, values, index, schema)?;
            expect_boolean(&inner, schema)?;
            Ok(Expr::Not(Box::new(inner)))
        }
        _ => {
            let expr = values
                .get(*index)
                .cloned()
                .ok_or_else(|| Error::InvalidQuery("predicate leaf is missing".into()))?;
            *index += 1;
            Ok(expr)
        }
    }
}

fn fragment_case(arms: Vec<(Expr, Expr)>) -> Expr {
    if arms.is_empty() {
        return unknown();
    }
    Expr::Case(datafusion::logical_expr::expr::Case::new(
        None,
        arms.into_iter()
            .map(|(condition, value)| (Box::new(condition), Box::new(value)))
            .collect(),
        Some(Box::new(unknown())),
    ))
}

fn expect_boolean(expr: &Expr, schema: &DFSchema) -> Result<()> {
    let data_type = expr.get_type(schema)?;
    if matches!(data_type, DataType::Boolean | DataType::Null) {
        Ok(())
    } else {
        Err(Error::InvalidQuery(format!(
            "expected boolean predicate, got {data_type}"
        )))
    }
}
