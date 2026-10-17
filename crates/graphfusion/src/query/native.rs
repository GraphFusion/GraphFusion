//! Compare a pattern predicate on each fragment before property types are unified.
//!
//! A multi-type scan casts every fragment into one Arrow column. Doing the
//! comparison after that cast rejects an integer literal against a stringified
//! column, and casting both sides to strings would make `10 > 9` false.
//! Each fragment therefore keeps its stored type. A missing property, or a
//! stored type that cannot be compared with the literal, is unknown: `NOT`
//! unknown stays unknown and does not keep the row.

use super::{expressions::Binder, graph::Bindings};
use crate::{gql as ast, graph::Table, Error, Result, SessionState};
use datafusion::{
    arrow::datatypes::{DataType, Schema},
    common::{Column, DFSchema, ScalarValue},
    logical_expr::{cast, expr::BinaryExpr, lit, Expr, ExprSchemable, Operator},
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug)]
pub(super) enum NativeAtom {
    And(Box<NativeAtom>, Box<NativeAtom>),
    Or(Box<NativeAtom>, Box<NativeAtom>),
    Not(Box<NativeAtom>),
    Xor(Box<NativeAtom>, Box<NativeAtom>),
    /// A closed boolean value, such as `1 = 1`, already bound for every fragment.
    Bool(Expr),
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
/// are pushed only when every comparison names the same variable. A closed
/// comparison that names no variable stays in that variable's tree. A value
/// that reads another column stays on the unified plan.
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
        NativeAtom::And(left, right)
        | NativeAtom::Or(left, right)
        | NativeAtom::Xor(left, right) => {
            validate_atom(left, tables)?;
            validate_atom(right, tables)
        }
        NativeAtom::Not(inner) => validate_atom(inner, tables),
        NativeAtom::Bool(_) => Ok(()),
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
        NativeAtom::Xor(left, right) => Expr::BinaryExpr(BinaryExpr::new(
            Box::new(compile(left, schema)?),
            Operator::NotEq,
            Box::new(compile(right, schema)?),
        )),
        NativeAtom::Bool(expr) => expr.clone(),
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
    Xor(Box<Local>, Box<Local>),
    Constant(ast::Expr),
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
        match (partition(left), partition(right)) {
            (Some(mut combined), Some(right_parts)) => {
                for (name, local) in right_parts {
                    if let Some(existing) = combined.remove(&name) {
                        combined.insert(name, Local::And(Box::new(existing), Box::new(local)));
                    } else {
                        combined.insert(name, local);
                    }
                }
                return Some(combined);
            }
            (Some(parts), None) if closed_predicate(right) => {
                return Some(and_constant(parts, right));
            }
            (None, Some(parts)) if closed_predicate(left) => {
                return Some(and_constant(parts, left));
            }
            _ => {}
        }
    }
    let variable = sole_variable(expr)?;
    Some(BTreeMap::from([(
        variable.clone(),
        local_tree(expr, &variable)?,
    )]))
}

/// Conjoin a closed comparison with one variable's predicate. `AND` with a
/// constant is independent of which side evaluates it.
fn and_constant(
    mut parts: BTreeMap<String, Local>,
    constant: &ast::Expr,
) -> BTreeMap<String, Local> {
    let Some((name, local)) = parts.pop_first() else {
        return parts;
    };
    parts.insert(
        name,
        Local::And(Box::new(local), Box::new(Local::Constant(constant.clone()))),
    );
    parts
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
        ast::Expr::Binary {
            left,
            op: ast::BinaryOp::Xor,
            right,
        } => {
            // Two property comparisons stay on the unified plan.
            if mentions(left, variable) && mentions(right, variable) {
                None
            } else {
                Some(Local::Xor(
                    Box::new(local_tree(left, variable)?),
                    Box::new(local_tree(right, variable)?),
                ))
            }
        }
        other => {
            if let Some((property, op, value)) = comparison(other, variable) {
                return Some(Local::Atom {
                    property,
                    op,
                    value,
                });
            }
            constant_predicate(other, variable).then(|| Local::Constant(other.clone()))
        }
    }
}

/// A boolean expression that does not read `variable`. Bare truth values and
/// parameters stay on the unified plan.
fn constant_predicate(expr: &ast::Expr, variable: &str) -> bool {
    !mentions(expr, variable) && !bare_truth(expr)
}

/// A predicate with no property reference. It can be attached to any variable
/// when an `AND` splits across several variables.
fn closed_predicate(expr: &ast::Expr) -> bool {
    if bare_truth(expr) || contains_opaque(expr) {
        return false;
    }
    let mut names = BTreeSet::new();
    property_bases(expr, &mut names);
    names.is_empty()
}

fn contains_opaque(expr: &ast::Expr) -> bool {
    opaque(expr) || children(expr).into_iter().any(contains_opaque)
}

fn bare_truth(expr: &ast::Expr) -> bool {
    match expr {
        ast::Expr::Literal(ast::Literal::Boolean(_) | ast::Literal::Unknown)
        | ast::Expr::Parameter(_) => true,
        ast::Expr::Unary {
            op: ast::UnaryOp::Not,
            expr,
        } => bare_truth(expr),
        _ => false,
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
        Local::Xor(left, right) => match (
            bind_local(session, schema, scope, left)?,
            bind_local(session, schema, scope, right)?,
        ) {
            (Some(left), Some(right)) => Some(NativeAtom::Xor(Box::new(left), Box::new(right))),
            _ => None,
        },
        Local::Constant(expr) => {
            let Some((bound, value_type)) = bind_closed(session, schema, scope, expr)? else {
                return Ok(None);
            };
            if !matches!(value_type, DataType::Boolean | DataType::Null) {
                return Ok(None);
            }
            Some(NativeAtom::Bool(bound))
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
