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
    graph::{Bindings, ElementBinding, ElementKind},
};
use crate::{gql as ast, graph::Table, Error, Result, SessionState};
use datafusion::{
    arrow::datatypes::{DataType, Schema, SchemaRef},
    common::{Column, DFSchema, ScalarValue, TableReference},
    datasource::provider_as_source,
    functions::core::expr_fn::coalesce,
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

/// A property-to-property comparison compiled on stored fragment types.
pub(super) struct PropertyPair {
    pub expr: Expr,
    pub ids: Vec<Expr>,
}

/// Equality of a pattern property and another element's property, using stored
/// types when a unified column would compare integers as text.
pub(super) fn equated_property(
    plan: LogicalPlanBuilder,
    scope: &mut Bindings,
    _session: &SessionState,
    binding: &ElementBinding,
    property: &str,
    value: &ast::Expr,
) -> Result<(LogicalPlanBuilder, Option<Expr>)> {
    let Some((right_name, right_property)) = element_property(value, scope) else {
        return Ok((plan, None));
    };
    let Some(right) = scope.elements.get(&right_name).cloned() else {
        return Ok((plan, None));
    };
    if !pair_needs_stored_types(scope, binding, property, &right, &right_property) {
        return Ok((plan, None));
    }
    let (plan, expr) = compile_property_pair(
        plan,
        scope,
        binding,
        property,
        Operator::Eq,
        &right,
        &right_property,
    )?;
    Ok((plan, Some(expr)))
}

/// Boolean tree containing a property-to-property comparison. `None` leaves the
/// unified columns in place: either the tree is not that comparison, or every
/// stored pair already agrees with them.
pub(super) fn property_pair(
    plan: LogicalPlanBuilder,
    scope: &mut Bindings,
    session: &SessionState,
    expr: &ast::Expr,
) -> Result<(LogicalPlanBuilder, Option<PropertyPair>)> {
    let Some(tree) = pair_tree(session, plan.schema(), scope, expr) else {
        return Ok((plan, None));
    };
    if !tree_needs_stored_types(scope, &tree) {
        return Ok((plan, None));
    }
    let mut joined = Joined {
        plan,
        scope,
        fragments: BTreeMap::new(),
    };
    let expr = joined.compile(&tree)?;
    let ids = tree_variables(&tree)
        .into_iter()
        .map(|name| {
            joined
                .scope
                .elements
                .get(&name)
                .expect("pair element")
                .column(crate::graph::ID)
        })
        .collect();
    Ok((joined.plan, Some(PropertyPair { expr, ids })))
}

enum PairNode {
    And(Box<PairNode>, Box<PairNode>),
    Or(Box<PairNode>, Box<PairNode>),
    Xor(Box<PairNode>, Box<PairNode>),
    Not(Box<PairNode>),
    Closed(Expr),
    One {
        variable: String,
        property: String,
        op: Operator,
        value: Expr,
        value_type: DataType,
    },
    Pair {
        left: String,
        left_property: String,
        op: Operator,
        right: String,
        right_property: String,
    },
}

fn pair_tree(
    session: &SessionState,
    schema: &DFSchema,
    scope: &Bindings,
    expr: &ast::Expr,
) -> Option<PairNode> {
    use ast::Expr as E;
    match expr {
        E::Binary {
            left,
            op: ast::BinaryOp::And,
            right,
        } => Some(PairNode::And(
            Box::new(pair_tree(session, schema, scope, left)?),
            Box::new(pair_tree(session, schema, scope, right)?),
        )),
        E::Binary {
            left,
            op: ast::BinaryOp::Or,
            right,
        } => Some(PairNode::Or(
            Box::new(pair_tree(session, schema, scope, left)?),
            Box::new(pair_tree(session, schema, scope, right)?),
        )),
        E::Binary {
            left,
            op: ast::BinaryOp::Xor,
            right,
        } => Some(PairNode::Xor(
            Box::new(pair_tree(session, schema, scope, left)?),
            Box::new(pair_tree(session, schema, scope, right)?),
        )),
        E::Unary {
            op: ast::UnaryOp::Not,
            expr,
        } => Some(PairNode::Not(Box::new(pair_tree(
            session, schema, scope, expr,
        )?))),
        other => comparison_node(session, schema, scope, other).or_else(|| {
            let bound = Binder::with_bindings(session, schema, scope)
                .bind(other)
                .ok()?;
            if !bound.column_refs().is_empty() {
                return None;
            }
            let data_type = bound.get_type(schema).ok()?;
            matches!(data_type, DataType::Boolean | DataType::Null)
                .then_some(PairNode::Closed(bound))
        }),
    }
}

fn comparison_node(
    session: &SessionState,
    schema: &DFSchema,
    scope: &Bindings,
    expr: &ast::Expr,
) -> Option<PairNode> {
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
    if let (Some((left_name, left_property)), Some((right_name, right_property))) = (
        element_property(left, scope),
        element_property(right, scope),
    ) {
        return Some(PairNode::Pair {
            left: left_name,
            left_property,
            op,
            right: right_name,
            right_property,
        });
    }
    if let Some((variable, property)) = element_property(left, scope) {
        let (value, value_type) = bind_closed(session, schema, scope, right).ok().flatten()?;
        return Some(PairNode::One {
            variable,
            property,
            op,
            value,
            value_type,
        });
    }
    if let Some((variable, property)) = element_property(right, scope) {
        let (value, value_type) = bind_closed(session, schema, scope, left).ok().flatten()?;
        return Some(PairNode::One {
            variable,
            property,
            op: flip(op),
            value,
            value_type,
        });
    }
    None
}

fn element_property(expr: &ast::Expr, scope: &Bindings) -> Option<(String, String)> {
    let ast::Expr::Property { base, key } = expr else {
        return None;
    };
    let ast::Expr::Identifier(name) = base.as_ref() else {
        return None;
    };
    scope
        .elements
        .contains_key(&name.value)
        .then(|| (name.value.clone(), key.value.clone()))
}

fn tree_needs_stored_types(scope: &Bindings, node: &PairNode) -> bool {
    match node {
        PairNode::And(left, right) | PairNode::Or(left, right) | PairNode::Xor(left, right) => {
            tree_needs_stored_types(scope, left) || tree_needs_stored_types(scope, right)
        }
        PairNode::Not(inner) => tree_needs_stored_types(scope, inner),
        PairNode::Closed(_) | PairNode::One { .. } => false,
        PairNode::Pair {
            left,
            left_property,
            right,
            right_property,
            ..
        } => {
            let Some(left_binding) = scope.elements.get(left) else {
                return false;
            };
            let Some(right_binding) = scope.elements.get(right) else {
                return false;
            };
            pair_needs_stored_types(
                scope,
                left_binding,
                left_property,
                right_binding,
                right_property,
            )
        }
    }
}

fn tree_variables(node: &PairNode) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    fn walk(node: &PairNode, names: &mut BTreeSet<String>) {
        match node {
            PairNode::And(left, right) | PairNode::Or(left, right) | PairNode::Xor(left, right) => {
                walk(left, names);
                walk(right, names);
            }
            PairNode::Not(inner) => walk(inner, names),
            PairNode::Closed(_) => {}
            PairNode::One { variable, .. } => {
                names.insert(variable.clone());
            }
            PairNode::Pair { left, right, .. } => {
                names.insert(left.clone());
                names.insert(right.clone());
            }
        }
    }
    walk(node, &mut names);
    names
}

fn pair_needs_stored_types(
    scope: &Bindings,
    left: &ElementBinding,
    left_property: &str,
    right: &ElementBinding,
    right_property: &str,
) -> bool {
    let left_tables = fragment_tables(scope, left);
    let right_tables = fragment_tables(scope, right);
    let mut comparable_pair = false;
    let mut disagree = false;
    for left_table in &left_tables {
        for right_table in &right_tables {
            match (
                left_table.schema.field_with_name(left_property).ok(),
                right_table.schema.field_with_name(right_property).ok(),
            ) {
                (Some(left_field), Some(right_field))
                    if comparable(left_field.data_type(), right_field.data_type()) =>
                {
                    comparable_pair = true;
                    if !(left_field.data_type() == right_field.data_type()
                        || (left_field.data_type().is_numeric()
                            && right_field.data_type().is_numeric()))
                    {
                        disagree = true;
                    }
                }
                (Some(_), Some(_)) => disagree = true,
                _ => {}
            }
        }
    }
    comparable_pair
        && (disagree
            || property_type_mixes(&left_tables, left_property)
            || property_type_mixes(&right_tables, right_property))
}

fn property_type_mixes(tables: &[&Table], property: &str) -> bool {
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

fn fragment_tables<'a>(scope: &'a Bindings, binding: &ElementBinding) -> Vec<&'a Table> {
    let Some(data) = scope.sources.get(&binding.graph) else {
        return Vec::new();
    };
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

struct Joined<'a> {
    plan: LogicalPlanBuilder,
    scope: &'a mut Bindings,
    fragments: BTreeMap<String, Vec<(String, SchemaRef)>>,
}

impl Joined<'_> {
    fn compile(&mut self, node: &PairNode) -> Result<Expr> {
        Ok(match node {
            PairNode::And(left, right) => self.compile(left)?.and(self.compile(right)?),
            PairNode::Or(left, right) => self.compile(left)?.or(self.compile(right)?),
            PairNode::Xor(left, right) => self.compile(left)?.not_eq(self.compile(right)?),
            PairNode::Not(inner) => Expr::Not(Box::new(self.compile(inner)?)),
            PairNode::Closed(expr) => expr.clone(),
            PairNode::One {
                variable,
                property,
                op,
                value,
                value_type,
            } => {
                let binding = self.scope.elements.get(variable).cloned().ok_or_else(|| {
                    Error::InvalidQuery(format!("unbound element variable {variable}"))
                })?;
                ensure_literal(&fragment_tables(self.scope, &binding), property, value_type)?;
                let fragments = self.attach(&binding)?;
                coalesce_parts(fragments.iter().map(|(alias, schema)| {
                    compare_property_qualified(
                        schema,
                        alias,
                        property,
                        *op,
                        value.clone(),
                        value_type,
                    )
                }))?
            }
            PairNode::Pair {
                left,
                left_property,
                op,
                right,
                right_property,
            } => {
                let left_binding = self.scope.elements.get(left).cloned().ok_or_else(|| {
                    Error::InvalidQuery(format!("unbound element variable {left}"))
                })?;
                let right_binding = self.scope.elements.get(right).cloned().ok_or_else(|| {
                    Error::InvalidQuery(format!("unbound element variable {right}"))
                })?;
                self.property_pair(
                    &left_binding,
                    left_property,
                    *op,
                    &right_binding,
                    right_property,
                )?
            }
        })
    }

    fn property_pair(
        &mut self,
        left: &ElementBinding,
        left_property: &str,
        op: Operator,
        right: &ElementBinding,
        right_property: &str,
    ) -> Result<Expr> {
        ensure_pair(
            &fragment_tables(self.scope, left),
            left_property,
            &fragment_tables(self.scope, right),
            right_property,
        )?;
        let left_fragments = self.attach(left)?;
        let right_fragments = if left.alias == right.alias {
            left_fragments.clone()
        } else {
            self.attach(right)?
        };
        let mut parts = Vec::new();
        for (left_alias, left_schema) in &left_fragments {
            for (right_alias, right_schema) in &right_fragments {
                if left.alias == right.alias && left_alias != right_alias {
                    continue;
                }
                parts.push(compare_stored_columns(
                    left_schema,
                    left_alias,
                    left_property,
                    op,
                    right_schema,
                    right_alias,
                    right_property,
                ));
            }
        }
        coalesce_parts(parts)
    }

    fn attach(&mut self, binding: &ElementBinding) -> Result<Vec<(String, SchemaRef)>> {
        if let Some(existing) = self.fragments.get(&binding.alias) {
            return Ok(existing.clone());
        }
        let tables = fragment_tables(self.scope, binding)
            .into_iter()
            .map(|table| (table.schema.clone(), table.provider.clone()))
            .collect::<Vec<_>>();
        let mut attached = Vec::new();
        for (schema, provider) in tables {
            let alias = self.scope.fresh("pair");
            let scan =
                LogicalPlanBuilder::scan(&alias, provider_as_source(provider), None)?.build()?;
            let id = Expr::Column(Column::new(
                Some(TableReference::bare(alias.as_str())),
                crate::graph::ID,
            ));
            self.plan = std::mem::replace(&mut self.plan, LogicalPlanBuilder::empty(false))
                .join_on(
                    scan,
                    JoinType::Left,
                    [binding.column(crate::graph::ID).eq(id)],
                )?;
            attached.push((alias, schema));
        }
        self.fragments
            .insert(binding.alias.clone(), attached.clone());
        Ok(attached)
    }
}

fn compile_property_pair(
    plan: LogicalPlanBuilder,
    scope: &mut Bindings,
    left: &ElementBinding,
    left_property: &str,
    op: Operator,
    right: &ElementBinding,
    right_property: &str,
) -> Result<(LogicalPlanBuilder, Expr)> {
    let mut joined = Joined {
        plan,
        scope,
        fragments: BTreeMap::new(),
    };
    let expr = joined.property_pair(left, left_property, op, right, right_property)?;
    Ok((joined.plan, expr))
}

fn ensure_literal(tables: &[&Table], property: &str, value_type: &DataType) -> Result<()> {
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

fn ensure_pair(
    left_tables: &[&Table],
    left_property: &str,
    right_tables: &[&Table],
    right_property: &str,
) -> Result<()> {
    let mut left_type = None;
    let mut right_type = None;
    for left in left_tables {
        for right in right_tables {
            match (
                left.schema.field_with_name(left_property).ok(),
                right.schema.field_with_name(right_property).ok(),
            ) {
                (Some(left_field), Some(right_field))
                    if comparable(left_field.data_type(), right_field.data_type()) =>
                {
                    return Ok(());
                }
                (Some(left_field), Some(right_field)) => {
                    left_type.get_or_insert_with(|| left_field.data_type().clone());
                    right_type.get_or_insert_with(|| right_field.data_type().clone());
                }
                _ => {}
            }
        }
    }
    match (left_type, right_type) {
        (Some(left_type), Some(right_type)) => Err(Error::InvalidQuery(format!(
            "incomparable types {left_type} and {right_type}"
        ))),
        _ => Ok(()),
    }
}

fn compare_property_qualified(
    schema: &Schema,
    alias: &str,
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
    let mut left = qualified(alias, property);
    let mut right = value;
    if stored != *value_type && stored.is_numeric() && value_type.is_numeric() {
        left = cast(left, DataType::Float64);
        right = cast(right, DataType::Float64);
    }
    Ok(apply_op(left, op, right))
}

fn compare_stored_columns(
    left_schema: &Schema,
    left_alias: &str,
    left_property: &str,
    op: Operator,
    right_schema: &Schema,
    right_alias: &str,
    right_property: &str,
) -> Result<Expr> {
    let Some(left_type) = left_schema
        .field_with_name(left_property)
        .ok()
        .map(|field| field.data_type().clone())
    else {
        return Ok(unknown());
    };
    let Some(right_type) = right_schema
        .field_with_name(right_property)
        .ok()
        .map(|field| field.data_type().clone())
    else {
        return Ok(unknown());
    };
    if !comparable(&left_type, &right_type) {
        return Ok(unknown());
    }
    let mut left = qualified(left_alias, left_property);
    let mut right = qualified(right_alias, right_property);
    if left_type != right_type && left_type.is_numeric() && right_type.is_numeric() {
        left = cast(left, DataType::Float64);
        right = cast(right, DataType::Float64);
    }
    Ok(apply_op(left, op, right))
}

fn apply_op(left: Expr, op: Operator, right: Expr) -> Expr {
    match op {
        Operator::Eq => left.eq(right),
        Operator::NotEq => left.not_eq(right),
        Operator::Lt => left.lt(right),
        Operator::LtEq => left.lt_eq(right),
        Operator::Gt => left.gt(right),
        Operator::GtEq => left.gt_eq(right),
        _ => unknown(),
    }
}

fn qualified(alias: &str, name: &str) -> Expr {
    Expr::Column(Column::new(Some(TableReference::bare(alias)), name))
}

fn coalesce_parts(parts: impl IntoIterator<Item = Result<Expr>>) -> Result<Expr> {
    let parts = parts.into_iter().collect::<Result<Vec<_>>>()?;
    Ok(match parts.len() {
        0 => unknown(),
        1 => parts.into_iter().next().expect("comparison"),
        _ => coalesce(parts),
    })
}
