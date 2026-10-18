//! Compare a pattern predicate on each fragment before property types are unified.
//!
//! A multi-type scan casts every fragment into one Arrow column. Doing the
//! comparison after that cast rejects an integer literal against a stringified
//! column, and casting both sides to strings would make `10 > 9` false.
//! Each fragment therefore keeps its stored type. A missing property, or a
//! stored type that cannot be compared with the literal, is unknown: `NOT`
//! unknown stays unknown and does not keep the row. `AND`, `OR`, `XOR`, and
//! closed booleans stay on that per-fragment result, so `OR FALSE` cannot
//! compare the stringified column instead.

use super::{
    expressions::Binder,
    graph::{Bindings, ElementBinding, ElementKind},
};
use crate::{gql as ast, graph::Table, Error, Result, SessionState};
use datafusion::{
    arrow::datatypes::{DataType, Schema},
    common::{Column, DFSchema, ScalarValue, TableReference},
    datasource::provider_as_source,
    functions::core::expr_fn::coalesce,
    logical_expr::{
        cast, expr::BinaryExpr, lit, Expr, ExprSchemable, JoinType, LogicalPlanBuilder, Operator,
    },
};
use std::collections::{BTreeMap, BTreeSet};

/// Partition key for a conjunct that does not read an element property.
const CLOSED: &str = "\0";

#[derive(Clone, Debug)]
pub(super) enum NativeAtom {
    And(Box<NativeAtom>, Box<NativeAtom>),
    Or(Box<NativeAtom>, Box<NativeAtom>),
    Xor(Box<NativeAtom>, Box<NativeAtom>),
    Not(Box<NativeAtom>),
    /// A boolean that does not read the fragment, such as `TRUE` or `1 = 1`.
    Closed(Expr),
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
/// `AND` can be split across variables. `OR`, `XOR`, and `NOT` stay together, so
/// they are pushed only when every comparison names the same variable. A closed
/// boolean (`TRUE`, `FALSE`, `UNKNOWN`, or a row-independent expression) stays
/// inside that predicate. A value that reads another column stays on the unified
/// plan.
pub(super) fn clause_filters(
    session: &SessionState,
    schema: &DFSchema,
    scope: &Bindings,
    expr: &ast::Expr,
) -> Result<Option<BTreeMap<String, NativeAtom>>> {
    let Some(parts) = partition(expr) else {
        return Ok(None);
    };
    let parts = fold_closed(parts);
    // A predicate with no element property is bound on the working table.
    if parts.contains_key(CLOSED) {
        return Ok(None);
    }
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
        NativeAtom::And(left, right)
        | NativeAtom::Or(left, right)
        | NativeAtom::Xor(left, right) => {
            validate_atom(left, tables)?;
            validate_atom(right, tables)
        }
        NativeAtom::Not(inner) => validate_atom(inner, tables),
        NativeAtom::Closed(_) => Ok(()),
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
        NativeAtom::Xor(left, right) => xor(compile(left, schema)?, compile(right, schema)?),
        NativeAtom::Not(inner) => Expr::Not(Box::new(compile(inner, schema)?)),
        NativeAtom::Closed(expr) => expr.clone(),
        NativeAtom::Compare {
            property,
            op,
            value,
            value_type,
        } => compare_property(
            schema,
            &column(property),
            property,
            *op,
            value.clone(),
            value_type,
        )?,
    })
}

fn xor(left: Expr, right: Expr) -> Expr {
    Expr::BinaryExpr(BinaryExpr::new(
        Box::new(left),
        Operator::NotEq,
        Box::new(right),
    ))
}

fn compare_property(
    schema: &Schema,
    left: &Expr,
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
    let mut left = left.clone();
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
    Xor(Box<Local>, Box<Local>),
    Not(Box<Local>),
    Closed(ast::Expr),
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
        return Some(fold_closed(combined));
    }
    if property_names(expr).is_empty() {
        return Some(BTreeMap::from([(
            CLOSED.to_string(),
            Local::Closed(expr.clone()),
        )]));
    }
    let variable = sole_variable(expr)?;
    Some(BTreeMap::from([(
        variable.clone(),
        local_tree(expr, &variable)?,
    )]))
}

/// A closed conjunct is part of every variable predicate. Alone, it is not a
/// fragment filter and the caller binds it on the working table.
fn fold_closed(mut parts: BTreeMap<String, Local>) -> BTreeMap<String, Local> {
    let Some(closed) = parts.remove(CLOSED) else {
        return parts;
    };
    let names: Vec<_> = parts.keys().cloned().collect();
    if names.is_empty() {
        parts.insert(CLOSED.to_string(), closed);
        return parts;
    }
    for name in names {
        let existing = parts.remove(&name).expect("variable conjunct");
        parts.insert(
            name,
            Local::And(Box::new(existing), Box::new(closed.clone())),
        );
    }
    parts
}

fn property_names(expr: &ast::Expr) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    property_bases(expr, &mut names);
    names
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
        ast::Expr::Binary {
            left,
            op: ast::BinaryOp::Xor,
            right,
        } => Some(Local::Xor(
            Box::new(local_tree(left, variable)?),
            Box::new(local_tree(right, variable)?),
        )),
        ast::Expr::Unary {
            op: ast::UnaryOp::Not,
            expr,
        } => Some(Local::Not(Box::new(local_tree(expr, variable)?))),
        other => {
            if let Some((property, op, value)) = comparison(other, variable) {
                Some(Local::Atom {
                    property,
                    op,
                    value,
                })
            } else if mentions(other, variable) {
                None
            } else {
                Some(Local::Closed(other.clone()))
            }
        }
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
        Local::Xor(left, right) => match (
            bind_local(session, schema, scope, left)?,
            bind_local(session, schema, scope, right)?,
        ) {
            (Some(left), Some(right)) => Some(NativeAtom::Xor(Box::new(left), Box::new(right))),
            _ => None,
        },
        Local::Not(inner) => {
            bind_local(session, schema, scope, inner)?.map(|inner| NativeAtom::Not(Box::new(inner)))
        }
        Local::Closed(expr) => {
            let Some((bound, data_type)) = bind_closed(session, schema, scope, expr)? else {
                return Ok(None);
            };
            if !matches!(data_type, DataType::Boolean | DataType::Null) {
                return Ok(None);
            }
            Some(NativeAtom::Closed(bound))
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

/// Keep a working-table predicate on stored types when a property was unified
/// across non-numeric fragments. A single stored type, including integer/float
/// widening, stays on the unified column.
pub(super) fn fragment_predicate(
    plan: LogicalPlanBuilder,
    scope: &mut Bindings,
    session: &SessionState,
    predicate: &ast::Expr,
) -> Result<Option<LogicalPlanBuilder>> {
    let columns = plan.schema().columns();
    let (plan, expr) = fragment_boolean(plan, scope, session, predicate)?;
    let Some(expr) = expr else {
        return Ok(None);
    };
    Ok(Some(plan.filter(expr)?.project(
        columns.into_iter().map(Expr::Column).collect::<Vec<_>>(),
    )?))
}

/// Boolean value of a mixed-type comparison tree, or `None` when the unified
/// column can compare it.
pub(super) fn fragment_boolean(
    plan: LogicalPlanBuilder,
    scope: &mut Bindings,
    session: &SessionState,
    predicate: &ast::Expr,
) -> Result<(LogicalPlanBuilder, Option<Expr>)> {
    let Some(filters) = clause_filters(session, plan.schema(), scope, predicate)? else {
        return Ok((plan, None));
    };
    if filters
        .keys()
        .any(|name| !scope.elements.contains_key(name))
    {
        return Ok((plan, None));
    }
    let mixed = filters.iter().any(|(name, atom)| {
        let Some(binding) = scope.elements.get(name).cloned() else {
            return false;
        };
        property_mixes(scope, &binding, atom)
    });
    if !mixed {
        return Ok((plan, None));
    }
    let mut plan = plan;
    let mut combined: Option<Expr> = None;
    for (name, atom) in filters {
        let binding = scope.elements.get(&name).expect("element variable").clone();
        let (next, expr) = compile_on_fragments(plan, scope, &binding, &atom)?;
        plan = next;
        combined = Some(match combined {
            Some(previous) => previous.and(expr),
            None => expr,
        });
    }
    Ok((plan, combined))
}

fn property_mixes(scope: &Bindings, binding: &ElementBinding, atom: &NativeAtom) -> bool {
    let Some(data) = scope.sources.get(&binding.graph) else {
        return false;
    };
    atom_mixes(atom, &element_tables(data, binding))
}

fn atom_mixes(atom: &NativeAtom, tables: &[&Table]) -> bool {
    match atom {
        NativeAtom::And(left, right)
        | NativeAtom::Or(left, right)
        | NativeAtom::Xor(left, right) => atom_mixes(left, tables) || atom_mixes(right, tables),
        NativeAtom::Not(inner) => atom_mixes(inner, tables),
        NativeAtom::Closed(_) => false,
        NativeAtom::Compare { property, .. } => {
            let mut seen: Vec<&DataType> = Vec::new();
            for table in tables {
                let Some(field) = table.schema.field_with_name(property).ok() else {
                    continue;
                };
                let data_type = field.data_type();
                if seen.iter().any(|previous| {
                    *previous != data_type && !(previous.is_numeric() && data_type.is_numeric())
                }) {
                    return true;
                }
                if !seen.contains(&data_type) {
                    seen.push(data_type);
                }
            }
            false
        }
    }
}

fn element_tables<'a>(
    data: &'a crate::graph::GraphData,
    binding: &ElementBinding,
) -> Vec<&'a Table> {
    let tables: Vec<&Table> = match binding.kind {
        ElementKind::Node => data.nodes.iter().map(|node| &node.0).collect(),
        ElementKind::Edge => data.edges.iter().map(|edge| &edge.table).collect(),
    };
    tables
        .into_iter()
        .filter(|table| {
            binding
                .source_labels
                .iter()
                .any(|labels| labels == &table.labels)
        })
        .collect()
}

fn compile_on_fragments(
    mut plan: LogicalPlanBuilder,
    scope: &mut Bindings,
    binding: &ElementBinding,
    atom: &NativeAtom,
) -> Result<(LogicalPlanBuilder, Expr)> {
    let fragments = {
        let data = scope
            .sources
            .get(&binding.graph)
            .ok_or_else(|| Error::Corrupt("missing graph for fragment comparison".into()))?;
        let tables = element_tables(data, binding);
        validate(atom, tables.iter().copied())?;
        tables
            .iter()
            .map(|table| (table.schema.clone(), table.provider.clone()))
            .collect::<Vec<_>>()
    };
    let mut parts = Vec::new();
    for (schema, provider) in fragments {
        let alias = scope.fresh("frag");
        let scan = LogicalPlanBuilder::scan(&alias, provider_as_source(provider), None)?.build()?;
        let id = Expr::Column(Column::new(
            Some(TableReference::bare(alias.as_str())),
            crate::graph::ID,
        ));
        plan = plan.join_on(
            scan,
            JoinType::Left,
            [binding.column(crate::graph::ID).eq(id)],
        )?;
        parts.push(compile_qualified(atom, &alias, schema.as_ref())?);
    }
    let expr = match parts.len() {
        0 => unknown(),
        1 => parts.pop().expect("fragment comparison"),
        _ => coalesce(parts),
    };
    Ok((plan, binding.ensure_live(expr, DataType::Boolean)))
}

fn compile_qualified(atom: &NativeAtom, alias: &str, schema: &Schema) -> Result<Expr> {
    Ok(match atom {
        NativeAtom::And(left, right) => {
            compile_qualified(left, alias, schema)?.and(compile_qualified(right, alias, schema)?)
        }
        NativeAtom::Or(left, right) => {
            compile_qualified(left, alias, schema)?.or(compile_qualified(right, alias, schema)?)
        }
        NativeAtom::Xor(left, right) => xor(
            compile_qualified(left, alias, schema)?,
            compile_qualified(right, alias, schema)?,
        ),
        NativeAtom::Not(inner) => Expr::Not(Box::new(compile_qualified(inner, alias, schema)?)),
        NativeAtom::Closed(expr) => expr.clone(),
        NativeAtom::Compare {
            property,
            op,
            value,
            value_type,
        } => {
            let left = Expr::Column(Column::new(Some(TableReference::bare(alias)), property));
            compare_property(schema, &left, property, *op, value.clone(), value_type)?
        }
    })
}
