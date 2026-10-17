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
    logical_expr::{cast, lit, Expr, ExprSchemable, Operator},
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug)]
pub(super) enum NativeAtom {
    And(Box<NativeAtom>, Box<NativeAtom>),
    Or(Box<NativeAtom>, Box<NativeAtom>),
    Not(Box<NativeAtom>),
    /// `PROPERTY_EXISTS(variable, property)`: a missing column or a null value
    /// is false, and a null element is null.
    Exists {
        property: String,
    },
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
/// are pushed only when every comparison or `PROPERTY_EXISTS` names the same
/// variable. That existence test stays with the variable's stored-type
/// comparison. A value that reads another column stays on the unified plan.
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
        NativeAtom::Exists { .. } => Ok(()),
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
        NativeAtom::Exists { property } => exists_expr(schema, property),
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

/// Match `ElementBinding::property_exists`: a stored null and a column the
/// fragment does not have are both false. A null element stays null.
fn exists_expr(schema: &Schema, property: &str) -> Expr {
    let present = if schema.field_with_name(property).is_ok() {
        column(property).is_not_null()
    } else {
        lit(false)
    };
    Expr::Case(datafusion::logical_expr::expr::Case::new(
        None,
        vec![(
            Box::new(column(crate::graph::ID).is_null()),
            Box::new(lit(ScalarValue::Boolean(None))),
        )],
        Some(Box::new(present)),
    ))
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
    Exists {
        property: String,
    },
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
        ast::Expr::PropertyExists {
            variable: name,
            property,
        } if name.value == variable => Some(Local::Exists {
            property: property.value.clone(),
        }),
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
        Local::Exists { property } => Some(NativeAtom::Exists {
            property: property.clone(),
        }),
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
    if let ast::Expr::PropertyExists { variable, .. } = expr {
        names.insert(variable.value.clone());
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
