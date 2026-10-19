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

mod operators;

#[derive(Clone, Default)]
pub(super) struct OperatorCache {
    values: BTreeMap<String, operators::Cached>,
}

use super::{
    casefold::CaseFoldMap,
    expressions::Binder,
    graph::{Bindings, ElementBinding, ElementKind},
};
use crate::{gql as ast, graph::Table, Error, Result, SessionState};
use datafusion::{
    arrow::datatypes::{DataType, Schema},
    common::{Column, DFSchema, ScalarValue, TableReference},
    datasource::provider_as_source,
    functions::core::expr_fn::{coalesce, nullif},
    logical_expr::{cast, lit, Expr, ExprSchemable, JoinType, LogicalPlanBuilder, Operator},
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Default)]
struct BindContext<'a> {
    aliases: Option<&'a CaseFoldMap<Expr>>,
    aggregates: bool,
}
impl<'a> BindContext<'a> {
    fn binder(
        self,
        session: &'a SessionState,
        schema: &'a DFSchema,
        scope: &'a Bindings,
    ) -> Binder<'a> {
        let mut binder = Binder::with_bindings(session, schema, scope);
        if self.aggregates {
            binder = binder.aggregating();
        }
        if let Some(aliases) = self.aliases {
            binder = binder.aliases(aliases);
        }
        binder
    }
}

/// Partition key for a conjunct that does not read an element property.
const CLOSED: &str = "\0";

#[derive(Clone, Debug)]
pub(super) enum NativeAtom {
    And(Box<NativeAtom>, Box<NativeAtom>),
    Or(Box<NativeAtom>, Box<NativeAtom>),
    Xor(Box<NativeAtom>, Box<NativeAtom>),
    Not(Box<NativeAtom>),
    ElementSelf {
        op: Operator,
    },
    ElementCompare {
        op: Operator,
        value: Expr,
        value_type: DataType,
    },
    Identity {
        different: bool,
        copies: usize,
    },
    Directed {
        negated: bool,
    },
    Labeled {
        expression: ast::LabelExpression,
        negated: bool,
    },
    Exists {
        property: String,
    },
    Null {
        property: Option<String>,
        negated: bool,
    },
    /// `IS [NOT] TRUE` / `IS [NOT] FALSE`. Unknown is its own category: it is
    /// not true and not false, unlike `NOT`, which leaves unknown unknown.
    Truth {
        inner: Box<NativeAtom>,
        negated: bool,
        value: bool,
    },
    /// `IS [NOT] UNKNOWN`.
    Unknown {
        inner: Box<NativeAtom>,
        negated: bool,
    },
    /// A boolean that does not read the fragment, such as `TRUE` or `1 = 1`.
    Closed(Expr),
    Compare {
        property: String,
        op: Operator,
        value: Expr,
        value_type: DataType,
    },
    /// Boolean `COALESCE` of per-fragment predicates and closed booleans.
    Coalesce(Vec<NativeAtom>),
    /// Boolean `NULLIF` of per-fragment predicates and closed booleans.
    NullIf(Box<NativeAtom>, Box<NativeAtom>),
    /// Closed boolean. Null is unknown.
    Literal(Expr),
    /// `NULLIF(property, value) IS [NOT] NULL` on the stored type.
    NullIfNull {
        property: String,
        value: Expr,
        value_type: DataType,
        negated: bool,
    },
    /// `COALESCE(property, fallback, ...) <op> value` on the stored type.
    ValueCoalesce {
        property: String,
        fallbacks: Vec<(Expr, DataType)>,
        op: Operator,
        right: Expr,
        right_type: DataType,
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
) -> Result<Option<CaseFoldMap<NativeAtom>>> {
    filters_with_values(session, schema, scope, expr, false, BindContext::default())
}

fn filters_with_values(
    session: &SessionState,
    schema: &DFSchema,
    scope: &Bindings,
    expr: &ast::Expr,
    working_row: bool,
    context: BindContext<'_>,
) -> Result<Option<CaseFoldMap<NativeAtom>>> {
    let Some(parts) = partition(expr) else {
        return Ok(None);
    };
    let parts = fold_closed(parts);
    // A predicate with no element property is bound on the working table.
    if parts.contains_key(CLOSED) {
        return Ok(None);
    }
    let mut filters = CaseFoldMap::new();
    for (variable, local) in parts {
        let Some(atom) = bind_local_value(session, schema, scope, &local, working_row, context)?
        else {
            return Ok(None);
        };
        // `n` and `N` are one variable, so combine their conjuncts.
        let atom = match filters.remove(&variable) {
            Some(existing) => NativeAtom::And(Box::new(existing), Box::new(atom)),
            None => atom,
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
        NativeAtom::Exists { .. } => Ok(()),
        NativeAtom::Labeled { .. } => Ok(()),
        NativeAtom::Directed { .. } => Ok(()),
        NativeAtom::Identity { .. } => Ok(()),
        NativeAtom::ElementSelf { .. } => Ok(()),
        NativeAtom::ElementCompare { value_type, .. } => validate_identity_type(value_type),
        NativeAtom::And(left, right)
        | NativeAtom::Or(left, right)
        | NativeAtom::Xor(left, right) => {
            validate_atom(left, tables)?;
            validate_atom(right, tables)
        }
        NativeAtom::Not(inner)
        | NativeAtom::Truth { inner, .. }
        | NativeAtom::Unknown { inner, .. } => validate_atom(inner, tables),
        NativeAtom::Compare {
            property,
            value_type,
            ..
        } => validate_comparison(property, value_type, tables, false),
        NativeAtom::Coalesce(args) => {
            for arg in args {
                validate_atom(arg, tables)?;
            }
            Ok(())
        }
        NativeAtom::NullIf(left, right) => {
            validate_atom(left, tables)?;
            validate_atom(right, tables)
        }
        NativeAtom::Literal(_) | NativeAtom::Closed(_) | NativeAtom::Null { .. } => Ok(()),
        NativeAtom::NullIfNull {
            property,
            value_type,
            ..
        } => validate_comparison(property, value_type, tables, true),
        NativeAtom::ValueCoalesce {
            property,
            fallbacks,
            right_type,
            ..
        } => {
            let mut conflict = None;
            for table in tables {
                let stored = crate::graph::property_field(table.schema.as_ref(), property)
                    .map(|field| field.data_type().clone());
                if unify_coalesce(stored.as_ref(), fallbacks, right_type).is_some() {
                    return Ok(());
                }
                if conflict.is_none() {
                    conflict = type_conflict(stored.as_ref(), fallbacks, right_type);
                }
            }
            match conflict {
                Some((left, right)) => Err(Error::InvalidQuery(format!(
                    "incomparable types {left} and {right}"
                ))),
                None => Ok(()),
            }
        }
    }
}

/// A fragment that does not store `property` can still evaluate `NULLIF` of
/// that null. A comparison cannot: a missing property is unknown, and the
/// caller reports a type error only when some stored type is incomparable.
fn validate_comparison(
    property: &str,
    value_type: &DataType,
    tables: &[&Table],
    absent_is_value: bool,
) -> Result<()> {
    let mut stored = None;
    let mut absent = false;
    for table in tables {
        let Some(field) = crate::graph::property_field(table.schema.as_ref(), property) else {
            absent = true;
            continue;
        };
        if comparable(field.data_type(), value_type) {
            return Ok(());
        }
        stored.get_or_insert_with(|| field.data_type().clone());
    }
    if absent && absent_is_value {
        return Ok(());
    }
    match stored {
        Some(stored) => Err(Error::InvalidQuery(format!(
            "incomparable types {stored} and {value_type}"
        ))),
        None => Ok(()),
    }
}

#[derive(Clone, Copy, Default)]
pub(super) struct FragmentMeta<'a> {
    pub labels: Option<&'a BTreeSet<String>>,
    pub directed: Option<bool>,
    pub identity: Option<(crate::catalog::ObjectId, ElementKind)>,
}

pub(super) fn compile_fragment(
    atom: &NativeAtom,
    schema: &Schema,
    metadata: FragmentMeta<'_>,
) -> Result<Expr> {
    compile_in(atom, schema, None, metadata)
}

pub(super) fn compile(atom: &NativeAtom, schema: &Schema) -> Result<Expr> {
    compile_in(atom, schema, None, FragmentMeta::default())
}

fn qualified_column(qualifier: Option<&str>, property: &str) -> Expr {
    match qualifier {
        Some(alias) => Expr::Column(Column::new(Some(TableReference::bare(alias)), property)),
        None => column(property),
    }
}

fn qualified_property(schema: &Schema, qualifier: Option<&str>, property: &str) -> Expr {
    let stored = crate::graph::property_field(schema, property)
        .map_or(property, |field| field.name().as_str());
    qualified_column(qualifier, stored)
}

fn compile_in(
    atom: &NativeAtom,
    schema: &Schema,
    qualifier: Option<&str>,
    metadata: FragmentMeta<'_>,
) -> Result<Expr> {
    Ok(match atom {
        NativeAtom::Exists { property } => exists_expr(schema, property, qualifier),
        NativeAtom::Labeled {
            expression,
            negated,
        } => labeled_expr(schema, metadata.labels, expression, *negated, qualifier),
        NativeAtom::Directed { negated } => {
            directed_expr(schema, metadata.directed, *negated, qualifier)?
        }
        NativeAtom::Identity { different, copies } => {
            identity_expr(schema, *different, *copies, qualifier)
        }
        NativeAtom::ElementSelf { op } => {
            if schema.field_with_name(crate::graph::ID).is_err() {
                unknown()
            } else {
                self_compare(qualified_column(qualifier, crate::graph::ID), *op)
            }
        }
        NativeAtom::ElementCompare {
            op,
            value,
            value_type,
        } => compare_identity_value(
            stored_element_id(schema, metadata.identity, qualifier),
            *op,
            value.clone(),
            value_type,
        )?,
        NativeAtom::And(left, right) => super::expressions::logical_and(
            compile_in(left, schema, qualifier, metadata)?,
            compile_in(right, schema, qualifier, metadata)?,
        ),
        NativeAtom::Or(left, right) => super::expressions::logical_or(
            compile_in(left, schema, qualifier, metadata)?,
            compile_in(right, schema, qualifier, metadata)?,
        ),
        NativeAtom::Xor(left, right) => xor(
            compile_in(left, schema, qualifier, metadata)?,
            compile_in(right, schema, qualifier, metadata)?,
        ),
        NativeAtom::Truth {
            inner,
            negated,
            value,
        } => {
            let expr = compile_in(inner, schema, qualifier, metadata)?;
            let truth = if *value {
                expr.is_true()
            } else {
                expr.is_false()
            };
            if *negated {
                Expr::Not(Box::new(truth))
            } else {
                truth
            }
        }
        NativeAtom::Unknown { inner, negated } => {
            let expr = compile_in(inner, schema, qualifier, metadata)?;
            if *negated {
                expr.is_not_null()
            } else {
                expr.is_null()
            }
        }
        NativeAtom::Closed(value) => value.clone(),
        NativeAtom::Not(inner) => {
            Expr::Not(Box::new(compile_in(inner, schema, qualifier, metadata)?))
        }
        NativeAtom::Null { property, negated } => {
            null_expr(schema, property.as_deref(), *negated, qualifier)
        }
        NativeAtom::Compare {
            property,
            op,
            value,
            value_type,
        } => compare_property(
            schema,
            &qualified_property(schema, qualifier, property),
            property,
            *op,
            value.clone(),
            value_type,
        )?,
        NativeAtom::Coalesce(args) => {
            let exprs = args
                .iter()
                .map(|arg| compile_in(arg, schema, qualifier, metadata))
                .collect::<Result<Vec<_>>>()?;
            if exprs.is_empty() {
                unknown()
            } else {
                boolean_coalesce(exprs)?
            }
        }
        NativeAtom::NullIf(left, right) => nullif(
            compile_in(left, schema, qualifier, metadata)?,
            compile_in(right, schema, qualifier, metadata)?,
        ),
        NativeAtom::Literal(expr) => expr.clone(),
        NativeAtom::NullIfNull {
            property,
            value,
            value_type,
            negated,
        } => nullif_is_null(
            schema,
            property,
            value.clone(),
            value_type,
            *negated,
            qualifier,
        )?,
        NativeAtom::ValueCoalesce {
            property,
            fallbacks,
            op,
            right,
            right_type,
        } => value_coalesce(
            schema,
            property,
            fallbacks,
            *op,
            right.clone(),
            right_type,
            qualifier,
        )?,
    })
}

fn boolean_coalesce(args: Vec<Expr>) -> Result<Expr> {
    let types = vec![DataType::Boolean; args.len()];
    Ok(super::arithmetic::lazy_coalesce(
        args,
        &types,
        DataType::Boolean,
    )?)
}

fn xor(left: Expr, right: Expr) -> Expr {
    super::expressions::logical_xor(left, right)
}

fn compare_property(
    schema: &Schema,
    left: &Expr,
    property: &str,
    op: Operator,
    value: Expr,
    value_type: &DataType,
) -> Result<Expr> {
    let Some(field) = crate::graph::property_field(schema, property) else {
        return Ok(unknown());
    };
    let stored = field.data_type().clone();
    if !comparable(&stored, value_type) {
        return Ok(unknown());
    }
    if let Some(expr) = super::arithmetic::exact_numeric_compare(
        left.clone(),
        op,
        value.clone(),
        &stored,
        value_type,
    ) {
        return Ok(expr);
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

fn null_expr(
    schema: &Schema,
    property: Option<&str>,
    negated: bool,
    qualifier: Option<&str>,
) -> Expr {
    let name = match property {
        Some(property) => match crate::graph::property_field(schema, property) {
            Some(field) => field.name().as_str(),
            None => return lit(!negated),
        },
        None => crate::graph::ID,
    };
    if schema.field_with_name(name).is_err() {
        return lit(!negated);
    }
    let value = qualified_column(qualifier, name);
    if negated {
        value.is_not_null()
    } else {
        value.is_null()
    }
}

fn exists_expr(schema: &Schema, property: &str, qualifier: Option<&str>) -> Expr {
    if schema.field_with_name(crate::graph::ID).is_err() {
        return unknown();
    }
    let present = if crate::graph::property_field(schema, property).is_some() {
        qualified_property(schema, qualifier, property).is_not_null()
    } else {
        lit(false)
    };
    Expr::Case(datafusion::logical_expr::expr::Case::new(
        None,
        vec![(
            Box::new(qualified_column(qualifier, crate::graph::ID).is_null()),
            Box::new(unknown()),
        )],
        Some(Box::new(present)),
    ))
}

fn labeled_expr(
    schema: &Schema,
    labels: Option<&BTreeSet<String>>,
    expression: &ast::LabelExpression,
    negated: bool,
    qualifier: Option<&str>,
) -> Expr {
    let Some(labels) = labels else {
        return unknown();
    };
    if schema.field_with_name(crate::graph::ID).is_err() {
        return unknown();
    }
    let holds = super::graph::label_holds(labels, expression) ^ negated;
    Expr::Case(datafusion::logical_expr::expr::Case::new(
        None,
        vec![(
            Box::new(qualified_column(qualifier, crate::graph::ID).is_null()),
            Box::new(unknown()),
        )],
        Some(Box::new(lit(holds))),
    ))
}

pub(super) fn validate_kind(atom: &NativeAtom, kind: ElementKind) -> Result<()> {
    match atom {
        NativeAtom::Directed { .. } if kind != ElementKind::Edge => {
            Err(Error::InvalidQuery("IS DIRECTED requires an edge".into()))
        }
        NativeAtom::And(left, right)
        | NativeAtom::Or(left, right)
        | NativeAtom::Xor(left, right)
        | NativeAtom::NullIf(left, right) => {
            validate_kind(left, kind)?;
            validate_kind(right, kind)
        }
        NativeAtom::Not(inner)
        | NativeAtom::Truth { inner, .. }
        | NativeAtom::Unknown { inner, .. } => validate_kind(inner, kind),
        NativeAtom::Coalesce(args) => {
            for atom in args {
                validate_kind(atom, kind)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn directed_expr(
    schema: &Schema,
    directed: Option<bool>,
    negated: bool,
    qualifier: Option<&str>,
) -> Result<Expr> {
    if schema.field_with_name(crate::graph::ID).is_err() {
        return Ok(unknown());
    }
    let directed =
        directed.ok_or_else(|| Error::InvalidQuery("IS DIRECTED requires an edge".into()))?;
    Ok(Expr::Case(datafusion::logical_expr::expr::Case::new(
        None,
        vec![(
            Box::new(qualified_column(qualifier, crate::graph::ID).is_null()),
            Box::new(unknown()),
        )],
        Some(Box::new(lit(directed ^ negated))),
    )))
}

fn identity_expr(schema: &Schema, different: bool, copies: usize, qualifier: Option<&str>) -> Expr {
    if copies < 2 {
        return lit(true);
    }
    if schema.field_with_name(crate::graph::ID).is_err() {
        return unknown();
    }
    let id = qualified_column(qualifier, crate::graph::ID);
    let same = id.clone().eq(id);
    if different {
        Expr::Not(Box::new(same))
    } else {
        same
    }
}

fn element_id_comparison(expr: &ast::Expr, variable: &str) -> Option<Local> {
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
    match (left.as_ref(), right.as_ref()) {
        (ast::Expr::ElementId { variable: left }, ast::Expr::ElementId { variable: right })
            if left.value.eq_ignore_ascii_case(variable)
                && right.value.eq_ignore_ascii_case(variable) =>
        {
            Some(Local::ElementSelf { op })
        }
        (ast::Expr::ElementId { variable: name }, other)
            if name.value.eq_ignore_ascii_case(variable) && !mentions(other, variable) =>
        {
            Some(Local::ElementCompare {
                op,
                value: other.clone(),
            })
        }
        (other, ast::Expr::ElementId { variable: name })
            if name.value.eq_ignore_ascii_case(variable) && !mentions(other, variable) =>
        {
            Some(Local::ElementCompare {
                op: flip(op),
                value: other.clone(),
            })
        }
        _ => None,
    }
}

fn self_compare(value: Expr, op: Operator) -> Expr {
    match op {
        Operator::Eq | Operator::LtEq | Operator::GtEq => value.clone().eq(value),
        Operator::NotEq | Operator::Lt | Operator::Gt => {
            Expr::Not(Box::new(value.clone().eq(value.clone())))
        }
        _ => unknown(),
    }
}

fn validate_identity_type(value_type: &DataType) -> Result<()> {
    if matches!(value_type, DataType::Utf8 | DataType::Null) {
        Ok(())
    } else {
        Err(Error::InvalidQuery(format!(
            "incomparable types Utf8 and {value_type}"
        )))
    }
}

fn compare_identity_value(
    left: Expr,
    op: Operator,
    right: Expr,
    value_type: &DataType,
) -> Result<Expr> {
    validate_identity_type(value_type)?;
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

fn stored_element_id(
    schema: &Schema,
    identity: Option<(crate::catalog::ObjectId, ElementKind)>,
    qualifier: Option<&str>,
) -> Expr {
    let null = lit(ScalarValue::Utf8(None));
    if schema.field_with_name(crate::graph::ID).is_err() {
        return null;
    }
    let Some((graph, kind)) = identity else {
        return null;
    };
    let id = qualified_column(qualifier, crate::graph::ID);
    let tag = if kind == ElementKind::Node { "n" } else { "e" };
    let value = datafusion::functions::string::expr_fn::concat(vec![
        lit(format!("g{graph}:{tag}:")),
        cast(id.clone(), DataType::Utf8),
    ]);
    Expr::Case(datafusion::logical_expr::expr::Case::new(
        None,
        vec![(Box::new(id.is_null()), Box::new(null))],
        Some(Box::new(value)),
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
    Xor(Box<Local>, Box<Local>),
    Not(Box<Local>),
    ElementSelf {
        op: Operator,
    },
    ElementCompare {
        op: Operator,
        value: ast::Expr,
    },
    Identity {
        different: bool,
        copies: usize,
    },
    Directed {
        negated: bool,
    },
    Labeled {
        expression: ast::LabelExpression,
        negated: bool,
    },
    Exists {
        property: String,
    },
    Null {
        property: Option<String>,
        negated: bool,
    },
    Truth {
        inner: Box<Local>,
        negated: bool,
        value: bool,
    },
    Unknown {
        inner: Box<Local>,
        negated: bool,
    },
    Closed(ast::Expr),
    Atom {
        property: String,
        op: Operator,
        value: ast::Expr,
    },
    Coalesce(Vec<Local>),
    NullIf(Box<Local>, Box<Local>),
    Literal(ast::Expr),
    NullIfNull {
        property: String,
        value: ast::Expr,
        negated: bool,
    },
    ValueCoalesce {
        property: String,
        fallbacks: Vec<ast::Expr>,
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
        ast::Expr::PropertyExists {
            variable: name,
            property,
        } if name.value.eq_ignore_ascii_case(variable) => Some(Local::Exists {
            property: property.value.clone(),
        }),
        ast::Expr::IsLabeled {
            variable: name,
            negated,
            label_expression,
        } if name.value.eq_ignore_ascii_case(variable) => Some(Local::Labeled {
            expression: label_expression.clone(),
            negated: *negated,
        }),
        ast::Expr::IsDirected {
            variable: name,
            negated,
        } if name.value.eq_ignore_ascii_case(variable) => {
            Some(Local::Directed { negated: *negated })
        }
        ast::Expr::Same { variables }
            if !variables.is_empty()
                && variables
                    .iter()
                    .all(|name| name.value.eq_ignore_ascii_case(variable)) =>
        {
            Some(Local::Identity {
                different: false,
                copies: variables.len(),
            })
        }
        ast::Expr::AllDifferent { variables }
            if !variables.is_empty()
                && variables
                    .iter()
                    .all(|name| name.value.eq_ignore_ascii_case(variable)) =>
        {
            Some(Local::Identity {
                different: true,
                copies: variables.len(),
            })
        }
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
        } => Some(Local::Xor(
            Box::new(local_tree(left, variable)?),
            Box::new(local_tree(right, variable)?),
        )),
        ast::Expr::IsTruth {
            expr,
            negated,
            value,
        } => Some(Local::Truth {
            inner: Box::new(local_tree(expr, variable)?),
            negated: *negated,
            value: *value,
        }),
        ast::Expr::IsUnknown { expr, negated } => Some(Local::Unknown {
            inner: Box::new(local_tree(expr, variable)?),
            negated: *negated,
        }),
        ast::Expr::Coalesce(args) => {
            if args.is_empty() {
                return None;
            }
            Some(Local::Coalesce(
                args.iter()
                    .map(|arg| predicate_or_literal(arg, variable))
                    .collect::<Option<Vec<_>>>()?,
            ))
        }
        ast::Expr::NullIf { left, right } => Some(Local::NullIf(
            Box::new(predicate_or_literal(left, variable)?),
            Box::new(predicate_or_literal(right, variable)?),
        )),
        ast::Expr::IsNull { expr, negated } if mentions(expr, variable) => {
            if let Some(property) = property_of(expr, variable) {
                return Some(Local::Null {
                    property: Some(property),
                    negated: *negated,
                });
            }
            if matches!(expr.as_ref(), ast::Expr::Identifier(name) if name.value.eq_ignore_ascii_case(variable))
            {
                return Some(Local::Null {
                    property: None,
                    negated: *negated,
                });
            }
            if matches!(expr.as_ref(), ast::Expr::ElementId { variable: name } if name.value.eq_ignore_ascii_case(variable))
            {
                return Some(Local::Null {
                    property: None,
                    negated: *negated,
                });
            }
            let ast::Expr::NullIf { left, right } = expr.as_ref() else {
                return None;
            };
            let property = property_of(left, variable)?;
            if mentions(right, variable) {
                return None;
            }
            Some(Local::NullIfNull {
                property,
                value: right.as_ref().clone(),
                negated: *negated,
            })
        }
        other => element_id_comparison(other, variable)
            .or_else(|| value_coalesce_comparison(other, variable))
            .or_else(|| {
                comparison(other, variable).map(|(property, op, value)| Local::Atom {
                    property,
                    op,
                    value,
                })
            })
            .or_else(|| (!mentions(other, variable)).then(|| Local::Closed(other.clone()))),
    }
}

/// A per-fragment predicate, or a closed literal standing in for one.
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
    name.value
        .eq_ignore_ascii_case(variable)
        .then(|| key.value.clone())
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
    bind_local_value(session, schema, scope, local, false, BindContext::default())
}

fn bind_local_value(
    session: &SessionState,
    schema: &DFSchema,
    scope: &Bindings,
    local: &Local,
    working_row: bool,
    context: BindContext<'_>,
) -> Result<Option<NativeAtom>> {
    Ok(match local {
        Local::Exists { property } => Some(NativeAtom::Exists {
            property: property.clone(),
        }),
        Local::Labeled {
            expression,
            negated,
        } => Some(NativeAtom::Labeled {
            expression: expression.clone(),
            negated: *negated,
        }),
        Local::Directed { negated } => Some(NativeAtom::Directed { negated: *negated }),
        Local::Identity { different, copies } => Some(NativeAtom::Identity {
            different: *different,
            copies: *copies,
        }),
        Local::ElementSelf { op } => Some(NativeAtom::ElementSelf { op: *op }),
        Local::ElementCompare { op, value } => {
            bind_value(session, schema, scope, value, working_row, context)?.map(
                |(value, value_type)| NativeAtom::ElementCompare {
                    op: *op,
                    value,
                    value_type,
                },
            )
        }
        Local::And(left, right) => match (
            bind_local_value(session, schema, scope, left, working_row, context)?,
            bind_local_value(session, schema, scope, right, working_row, context)?,
        ) {
            (Some(left), Some(right)) => Some(NativeAtom::And(Box::new(left), Box::new(right))),
            _ => None,
        },
        Local::Or(left, right) => match (
            bind_local_value(session, schema, scope, left, working_row, context)?,
            bind_local_value(session, schema, scope, right, working_row, context)?,
        ) {
            (Some(left), Some(right)) => Some(NativeAtom::Or(Box::new(left), Box::new(right))),
            _ => None,
        },
        Local::Xor(left, right) => match (
            bind_local_value(session, schema, scope, left, working_row, context)?,
            bind_local_value(session, schema, scope, right, working_row, context)?,
        ) {
            (Some(left), Some(right)) => Some(NativeAtom::Xor(Box::new(left), Box::new(right))),
            _ => None,
        },
        Local::Not(inner) => bind_local_value(session, schema, scope, inner, working_row, context)?
            .map(|inner| NativeAtom::Not(Box::new(inner))),
        Local::Truth {
            inner,
            negated,
            value,
        } => bind_local_value(session, schema, scope, inner, working_row, context)?.map(|inner| {
            NativeAtom::Truth {
                inner: Box::new(inner),
                negated: *negated,
                value: *value,
            }
        }),
        Local::Unknown { inner, negated } => {
            bind_local_value(session, schema, scope, inner, working_row, context)?.map(|inner| {
                NativeAtom::Unknown {
                    inner: Box::new(inner),
                    negated: *negated,
                }
            })
        }
        Local::Null { property, negated } => Some(NativeAtom::Null {
            property: property.clone(),
            negated: *negated,
        }),
        Local::Closed(expr) => {
            let Some((bound, data_type)) =
                bind_value(session, schema, scope, expr, working_row, context)?
            else {
                return Ok(None);
            };
            if !matches!(data_type, DataType::Boolean | DataType::Null) {
                return Ok(None);
            }
            Some(NativeAtom::Closed(if data_type == DataType::Null {
                unknown()
            } else {
                bound
            }))
        }
        Local::Atom {
            property,
            op,
            value,
        } => bind_value(session, schema, scope, value, working_row, context)?.map(
            |(expr, value_type)| NativeAtom::Compare {
                property: property.clone(),
                op: *op,
                value: expr,
                value_type,
            },
        ),
        Local::Coalesce(args) => {
            let mut bound = Vec::with_capacity(args.len());
            for arg in args {
                let Some(atom) =
                    bind_local_value(session, schema, scope, arg, working_row, context)?
                else {
                    return Ok(None);
                };
                bound.push(atom);
            }
            Some(NativeAtom::Coalesce(bound))
        }
        Local::NullIf(left, right) => match (
            bind_local_value(session, schema, scope, left, working_row, context)?,
            bind_local_value(session, schema, scope, right, working_row, context)?,
        ) {
            (Some(left), Some(right)) => Some(NativeAtom::NullIf(Box::new(left), Box::new(right))),
            _ => None,
        },
        Local::Literal(value) => {
            let Some((expr, value_type)) =
                bind_value(session, schema, scope, value, working_row, context)?
            else {
                return Ok(None);
            };
            if value_type == DataType::Null {
                Some(NativeAtom::Literal(unknown()))
            } else if value_type == DataType::Boolean {
                Some(NativeAtom::Literal(expr))
            } else {
                return Ok(None);
            }
        }
        Local::NullIfNull {
            property,
            value,
            negated,
        } => bind_value(session, schema, scope, value, working_row, context)?.map(
            |(expr, value_type)| NativeAtom::NullIfNull {
                property: property.clone(),
                value: expr,
                value_type,
                negated: *negated,
            },
        ),
        Local::ValueCoalesce {
            property,
            fallbacks,
            op,
            value,
        } => {
            let mut bound = Vec::with_capacity(fallbacks.len());
            for fallback in fallbacks {
                let Some((expr, data_type)) =
                    bind_value(session, schema, scope, fallback, working_row, context)?
                else {
                    return Ok(None);
                };
                bound.push((expr, data_type));
            }
            let Some((right, right_type)) =
                bind_value(session, schema, scope, value, working_row, context)?
            else {
                return Ok(None);
            };
            Some(NativeAtom::ValueCoalesce {
                property: property.clone(),
                fallbacks: bound,
                op: *op,
                right,
                right_type,
            })
        }
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
    bind_value(session, schema, scope, value, false, BindContext::default())
}

// Fragment joins can read the working row; scan pushdown cannot.
fn bind_value(
    session: &SessionState,
    schema: &DFSchema,
    scope: &Bindings,
    value: &ast::Expr,
    working_row: bool,
    context: BindContext<'_>,
) -> Result<Option<(Expr, DataType)>> {
    let Ok(expr) = context.binder(session, schema, scope).bind(value) else {
        return Ok(None);
    };
    if !working_row && !expr.column_refs().is_empty() {
        return Ok(None);
    }
    let value_type = expr.get_type(schema)?;
    Ok(Some((expr, value_type)))
}

fn property_bases(expr: &ast::Expr, names: &mut BTreeSet<String>) {
    if let ast::Expr::Property { base, .. } = expr {
        if let ast::Expr::Identifier(name) = base.as_ref() {
            if !names
                .iter()
                .any(|existing| existing.eq_ignore_ascii_case(&name.value))
            {
                names.insert(name.value.to_ascii_lowercase());
            }
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

pub(super) fn named_variable(expr: &ast::Expr, variable: &str) -> bool {
    match expr {
        ast::Expr::Identifier(name)
        | ast::Expr::PropertyExists { variable: name, .. }
        | ast::Expr::ElementId { variable: name }
        | ast::Expr::IsDirected { variable: name, .. }
        | ast::Expr::IsLabeled { variable: name, .. } => name.value.eq_ignore_ascii_case(variable),
        ast::Expr::Same { variables } | ast::Expr::AllDifferent { variables } => variables
            .iter()
            .any(|name| name.value.eq_ignore_ascii_case(variable)),
        ast::Expr::SourceDestination { node, edge, .. } => {
            node.value.eq_ignore_ascii_case(variable) || edge.value.eq_ignore_ascii_case(variable)
        }
        _ => false,
    }
}

pub(super) fn children(expr: &ast::Expr) -> Vec<&ast::Expr> {
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
/// across fragments. A single stored type stays on the unified column; mixed
/// numeric comparisons retain integer precision before widening.
pub(super) fn predicate_value(value: Expr, schema: &DFSchema) -> Result<Expr> {
    match value.get_type(schema)? {
        DataType::Boolean => Ok(value),
        DataType::Null => Ok(cast(value, DataType::Boolean)),
        ty => Err(Error::InvalidQuery(format!(
            "expected boolean predicate, got {ty}"
        ))),
    }
}

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
    let expr = predicate_value(expr, plan.schema())?;
    Ok(Some(plan.filter(expr)?.project(
        columns.into_iter().map(Expr::Column).collect::<Vec<_>>(),
    )?))
}

pub(super) struct MixedValue {
    pub expr: Expr,
}

pub(super) fn fragment_value(
    plan: LogicalPlanBuilder,
    scope: &mut Bindings,
    session: &SessionState,
    expression: &ast::Expr,
) -> Result<(LogicalPlanBuilder, Option<MixedValue>)> {
    let (plan, value) = fragment_boolean(plan, scope, session, expression)?;
    Ok((plan, value.map(|expr| MixedValue { expr })))
}

/// Boolean value of a mixed-type comparison tree, or `None` when the unified
/// column can compare it.
pub(super) fn fragment_boolean(
    plan: LogicalPlanBuilder,
    scope: &mut Bindings,
    session: &SessionState,
    predicate: &ast::Expr,
) -> Result<(LogicalPlanBuilder, Option<Expr>)> {
    fragment_boolean_in(plan, scope, session, predicate, BindContext::default())
}

pub(super) fn fragment_result_value(
    plan: LogicalPlanBuilder,
    scope: &mut Bindings,
    session: &SessionState,
    aliases: &CaseFoldMap<Expr>,
    expression: &ast::Expr,
) -> Result<(LogicalPlanBuilder, Option<MixedValue>)> {
    let context = BindContext {
        aliases: Some(aliases),
        aggregates: true,
    };
    let (plan, value) = fragment_boolean_in(plan, scope, session, expression, context)?;
    Ok((plan, value.map(|expr| MixedValue { expr })))
}

fn fragment_boolean_in(
    plan: LogicalPlanBuilder,
    scope: &mut Bindings,
    session: &SessionState,
    predicate: &ast::Expr,
    context: BindContext<'_>,
) -> Result<(LogicalPlanBuilder, Option<Expr>)> {
    let (plan, value) = operators::compile(plan, scope, session, predicate, context)?;
    if value.is_some() {
        return Ok((plan, value));
    }
    let (plan, pair) = property_pair(plan, scope, predicate)?;
    if pair.is_some() {
        return Ok((plan, pair));
    }
    let Some(filters) =
        filters_with_values(session, plan.schema(), scope, predicate, true, context)?
    else {
        return combine_fragments(plan, scope, session, predicate, context);
    };
    if filters
        .keys()
        .any(|name| fragment_binding(scope, name).is_none())
    {
        return Ok((plan, None));
    }
    let mixed = filters.iter().any(|(name, atom)| {
        let Some(binding) = fragment_binding(scope, name) else {
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
        let binding = fragment_binding(scope, &name).expect("element variable");
        let (next, mut expr) = compile_on_fragments(plan, scope, &binding, &atom)?;
        if !scope.elements.contains_key(&name) {
            let reference = scope.references.get(&name).expect("resolved reference");
            let column = scope.scalars.get(&name).expect("reference column").clone();
            expr = Expr::Case(datafusion::logical_expr::expr::Case::new(
                None,
                vec![(
                    Box::new(reference.guard(Expr::Column(column))),
                    Box::new(expr),
                )],
                Some(Box::new(compile(&atom, &Schema::empty())?)),
            ));
        }
        plan = next;
        combined = Some(match combined {
            Some(previous) => previous.and(expr),
            None => expr,
        });
    }
    Ok((plan, combined))
}

#[derive(Clone, Copy)]
enum BooleanComposition {
    And,
    Or,
    Xor,
    Not,
    Truth { value: bool, negated: bool },
    Null { negated: bool, boolean: bool },
    Coalesce,
    NullIf,
}

/// Evaluate each mixed comparison on its own element's fragments, then combine
/// the boolean values on the working row. Joining on identity preserves rows
/// for which another element can make an OR true.
fn combine_fragments(
    mut plan: LogicalPlanBuilder,
    scope: &mut Bindings,
    session: &SessionState,
    predicate: &ast::Expr,
    context: BindContext<'_>,
) -> Result<(LogicalPlanBuilder, Option<Expr>)> {
    use BooleanComposition as C;
    let (operands, composition): (Vec<&ast::Expr>, C) = match predicate {
        ast::Expr::Binary { left, op, right } => {
            let composition = match op {
                ast::BinaryOp::And => C::And,
                ast::BinaryOp::Or => C::Or,
                ast::BinaryOp::Xor => C::Xor,
                _ => return Ok((plan, None)),
            };
            (vec![left, right], composition)
        }
        ast::Expr::Unary {
            op: ast::UnaryOp::Not,
            expr,
        } => (vec![expr], C::Not),
        ast::Expr::IsTruth {
            expr,
            value,
            negated,
        } => (
            vec![expr],
            C::Truth {
                value: *value,
                negated: *negated,
            },
        ),
        ast::Expr::IsUnknown { expr, negated } => (
            vec![expr],
            C::Null {
                negated: *negated,
                boolean: true,
            },
        ),
        ast::Expr::IsNull { expr, negated } => (
            vec![expr],
            C::Null {
                negated: *negated,
                boolean: false,
            },
        ),
        ast::Expr::Coalesce(args) => (args.iter().collect(), C::Coalesce),
        ast::Expr::NullIf { left, right } => (vec![left, right], C::NullIf),
        _ => return Ok((plan, None)),
    };
    let mut lowered = Vec::with_capacity(operands.len());
    for operand in &operands {
        let (next, value) = fragment_boolean_in(plan, scope, session, operand, context)?;
        plan = next;
        lowered.push(value);
    }
    if lowered.iter().all(Option::is_none) {
        return Ok((plan, None));
    }
    let mut values = operands
        .into_iter()
        .zip(lowered)
        .map(|(operand, value)| match value {
            Some(value) if !matches!(composition, C::Null { boolean: false, .. }) => {
                predicate_value(value, plan.schema())
            }
            Some(value) => Ok(value),
            None => context
                .binder(session, plan.schema(), scope)
                .predicate(operand),
        })
        .collect::<Result<Vec<_>>>()?
        .into_iter();
    let expression = match composition {
        C::And => super::expressions::logical_and(
            values.next().expect("left operand"),
            values.next().expect("right operand"),
        ),
        C::Or => super::expressions::logical_or(
            values.next().expect("left operand"),
            values.next().expect("right operand"),
        ),
        C::Xor => xor(
            values.next().expect("left operand"),
            values.next().expect("right operand"),
        ),
        C::Not => Expr::Not(Box::new(values.next().expect("operand"))),
        C::Truth { value, negated } => {
            let expr = values.next().expect("operand");
            let truth = if value {
                expr.is_true()
            } else {
                expr.is_false()
            };
            if negated {
                Expr::Not(Box::new(truth))
            } else {
                truth
            }
        }
        C::Null { negated, .. } => {
            let expr = values.next().expect("operand");
            if negated {
                expr.is_not_null()
            } else {
                expr.is_null()
            }
        }
        C::Coalesce => boolean_coalesce(values.collect())?,
        C::NullIf => nullif(
            values.next().expect("left operand"),
            values.next().expect("right operand"),
        ),
    };
    Ok((plan, Some(expression)))
}

// Multi-part references retain their existing cross-domain type validation.
fn fragment_binding(scope: &Bindings, name: &str) -> Option<ElementBinding> {
    scope.elements.get(name).cloned().or_else(|| {
        let reference = scope.references.get(name)?;
        match reference.parts.as_slice() {
            [binding] => Some(binding.clone()),
            _ => None,
        }
    })
}

fn property_mixes(scope: &Bindings, binding: &ElementBinding, atom: &NativeAtom) -> bool {
    let Some(data) = scope.sources.get(&binding.graph) else {
        return false;
    };
    atom_mixes(atom, &element_tables(data, binding))
}

fn atom_mixes(atom: &NativeAtom, tables: &[&Table]) -> bool {
    match atom {
        NativeAtom::Exists { .. } => false,
        NativeAtom::Labeled { .. } => false,
        NativeAtom::Directed { .. } => false,
        NativeAtom::Identity { .. } => false,
        NativeAtom::ElementSelf { .. } | NativeAtom::ElementCompare { .. } => false,
        NativeAtom::And(left, right)
        | NativeAtom::Or(left, right)
        | NativeAtom::Xor(left, right) => atom_mixes(left, tables) || atom_mixes(right, tables),
        NativeAtom::Not(inner)
        | NativeAtom::Truth { inner, .. }
        | NativeAtom::Unknown { inner, .. } => atom_mixes(inner, tables),
        NativeAtom::Closed(_) | NativeAtom::Literal(_) | NativeAtom::Null { .. } => false,
        NativeAtom::Coalesce(args) => args.iter().any(|atom| atom_mixes(atom, tables)),
        NativeAtom::NullIf(left, right) => atom_mixes(left, tables) || atom_mixes(right, tables),
        NativeAtom::NullIfNull { property, .. } | NativeAtom::ValueCoalesce { property, .. }
            if tables.iter().any(|table| {
                crate::graph::property_field(table.schema.as_ref(), property).is_none()
            }) =>
        {
            true
        }
        NativeAtom::Compare { property, .. }
        | NativeAtom::NullIfNull { property, .. }
        | NativeAtom::ValueCoalesce { property, .. } => {
            let mut seen: Vec<&DataType> = Vec::new();
            for table in tables {
                let Some(field) = crate::graph::property_field(table.schema.as_ref(), property)
                else {
                    continue;
                };
                let data_type = field.data_type();
                if seen.iter().any(|previous| *previous != data_type) {
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

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct FragmentKey {
    alias: String,
    graph: u64,
    kind: ElementKind,
    labels: Vec<BTreeSet<String>>,
    constrained: bool,
    snapshot: usize,
}

#[derive(Clone)]
pub(super) struct Fragment {
    alias: String,
    schema: datafusion::arrow::datatypes::SchemaRef,
    labels: BTreeSet<String>,
    directed: Option<bool>,
}
impl Fragment {
    fn column(&self, name: &str) -> Expr {
        qualified_column(Some(&self.alias), name)
    }
    fn columns(&self) -> impl Iterator<Item = Expr> + '_ {
        self.schema.fields().iter().map(|f| self.column(f.name()))
    }
}

pub(super) type FragmentCache = BTreeMap<FragmentKey, Vec<Fragment>>;

/// Add only fragment dependencies belonging to an already grouped element.
/// Closed scalars and other elements must still pass ordinary grouping checks.
pub(super) fn grouping_columns(
    scope: &Bindings,
    groups: &[Expr],
    expressions: &[&Expr],
) -> Vec<Expr> {
    let referenced: BTreeSet<_> = expressions
        .iter()
        .flat_map(|e| e.column_refs().into_iter().cloned())
        .collect();
    let mut columns: Vec<_> = scope
        .fragment_cache
        .iter()
        .filter(|(key, _)| groups.contains(&qualified_column(Some(&key.alias), crate::graph::ID)))
        .flat_map(|(_, fragments)| fragments.iter().flat_map(Fragment::columns))
        .filter(|expr| matches!(expr, Expr::Column(c) if referenced.contains(c)))
        .collect();
    for (column, dependencies) in &scope.operator_dependencies {
        let value = Expr::Column(column.clone());
        if referenced.contains(column)
            && dependencies
                .iter()
                .all(|dependency| groups.contains(&Expr::Column(dependency.clone())))
            && !columns.contains(&value)
        {
            columns.push(value);
        }
    }
    columns
}
fn attach_fragments(
    mut plan: LogicalPlanBuilder,
    scope: &mut Bindings,
    binding: &ElementBinding,
) -> Result<(LogicalPlanBuilder, Vec<Fragment>)> {
    let data = scope
        .sources
        .get(&binding.graph)
        .ok_or_else(|| Error::Corrupt("missing graph for fragment comparison".into()))?;
    let tables = element_tables(data, binding);
    let key = FragmentKey {
        alias: binding.alias.clone(),
        graph: binding.graph,
        kind: binding.kind,
        labels: binding.source_labels.clone(),
        constrained: binding.constrained,
        snapshot: std::sync::Arc::as_ptr(data) as usize,
    };
    let cached = scope
        .fragment_cache
        .get(&key)
        .filter(|fragments| {
            fragments.iter().flat_map(Fragment::columns).all(|expr| {
                let Expr::Column(column) = expr else {
                    unreachable!()
                };
                plan.schema().qualified_field_from_column(&column).is_ok()
            })
        })
        .cloned();
    let fragments = if let Some(cached) = cached {
        cached
    } else {
        let sources = tables
            .iter()
            .map(|table| {
                let directed = match binding.kind {
                    ElementKind::Node => None,
                    ElementKind::Edge => data
                        .edges
                        .iter()
                        .find(|edge| std::ptr::eq(&edge.table, *table))
                        .map(|e| e.directed),
                };
                (
                    table.schema.clone(),
                    table.provider.clone(),
                    table.labels.clone(),
                    directed,
                )
            })
            .collect::<Vec<_>>();
        let mut fragments = Vec::new();
        for (schema, provider, labels, directed) in sources {
            let alias = scope.fresh("frag");
            let scan =
                LogicalPlanBuilder::scan(&alias, provider_as_source(provider), None)?.build()?;
            let fragment = Fragment {
                alias,
                schema,
                labels,
                directed,
            };
            plan = plan.join_on(
                scan,
                JoinType::Left,
                [binding
                    .column(crate::graph::ID)
                    .eq(fragment.column(crate::graph::ID))],
            )?;
            fragments.push(fragment);
        }
        scope.fragment_cache.insert(key, fragments.clone());
        fragments
    };
    Ok((plan, fragments))
}

fn compile_on_fragments(
    plan: LogicalPlanBuilder,
    scope: &mut Bindings,
    binding: &ElementBinding,
    atom: &NativeAtom,
) -> Result<(LogicalPlanBuilder, Expr)> {
    let data = scope
        .sources
        .get(&binding.graph)
        .ok_or_else(|| Error::Corrupt("missing graph for fragment comparison".into()))?;
    validate_kind(atom, binding.kind)?;
    validate(atom, element_tables(data, binding))?;
    let (plan, fragments) = attach_fragments(plan, scope, binding)?;
    let mut parts = Vec::new();
    for fragment in fragments {
        parts.push(Expr::Case(datafusion::logical_expr::expr::Case::new(
            None,
            vec![(
                Box::new(fragment.column(crate::graph::ID).is_not_null()),
                Box::new(cast(
                    compile_in(
                        atom,
                        fragment.schema.as_ref(),
                        Some(&fragment.alias),
                        FragmentMeta {
                            labels: Some(&fragment.labels),
                            directed: fragment.directed,
                            identity: Some((binding.graph, binding.kind)),
                        },
                    )?,
                    DataType::Int64,
                )),
            )],
            Some(Box::new(lit(ScalarValue::Int64(None)))),
        )));
    }
    let expr = match parts.len() {
        0 => lit(ScalarValue::Int64(None)),
        1 => parts.pop().expect("fragment comparison"),
        _ => coalesce(parts),
    };
    let expr = Expr::Case(datafusion::logical_expr::expr::Case::new(
        None,
        vec![(
            Box::new(binding.column(crate::graph::ID).is_null()),
            Box::new(cast(compile(atom, &Schema::empty())?, DataType::Int64)),
        )],
        Some(Box::new(expr)),
    ));
    let expr = super::expressions::boolean_value(expr);
    Ok((plan, binding.ensure_live(expr, DataType::Boolean)))
}

fn nullif_is_null(
    schema: &Schema,
    property: &str,
    value: Expr,
    value_type: &DataType,
    negated: bool,
    qualifier: Option<&str>,
) -> Result<Expr> {
    let Some(stored) =
        crate::graph::property_field(schema, property).map(|field| field.data_type().clone())
    else {
        // The property is absent, so it is null. NULLIF of null is null.
        return Ok(lit(!negated));
    };
    if !comparable(&stored, value_type) {
        return Ok(unknown());
    }
    let left = qualified_property(schema, qualifier, property);
    if let Some(compared) =
        super::arithmetic::exact_nullif(left.clone(), value.clone(), &stored, value_type)
    {
        return Ok(if negated {
            compared.is_not_null()
        } else {
            compared.is_null()
        });
    }
    let mut left = left;
    let mut right = value;
    if *value_type == DataType::Null {
        right = cast(right, stored);
    } else if stored != *value_type && stored.is_numeric() && value_type.is_numeric() {
        left = cast(left, DataType::Float64);
        right = cast(right, DataType::Float64);
    }
    let compared = nullif(left, right);
    Ok(if negated {
        compared.is_not_null()
    } else {
        compared.is_null()
    })
}

fn value_coalesce(
    schema: &Schema,
    property: &str,
    fallbacks: &[(Expr, DataType)],
    op: Operator,
    right: Expr,
    right_type: &DataType,
    qualifier: Option<&str>,
) -> Result<Expr> {
    let stored =
        crate::graph::property_field(schema, property).map(|field| field.data_type().clone());
    let Some(unified) = unify_coalesce(stored.as_ref(), fallbacks, right_type) else {
        return Ok(unknown());
    };
    let mut args = Vec::new();
    let mut types = Vec::new();
    if let Some(stored) = &stored {
        args.push(qualified_property(schema, qualifier, property));
        types.push(stored.clone());
    }
    for (expr, ty) in fallbacks {
        args.push(expr.clone());
        types.push(ty.clone());
    }
    if args.is_empty() {
        return Ok(unknown());
    }
    let value_type = types.iter().try_fold(DataType::Null, |left, right| {
        super::expressions::common_value_type(&left, right)
    })?;
    let value = match super::arithmetic::exact_coalesce(args.clone(), &types)? {
        Some(value) => value,
        None => super::arithmetic::lazy_coalesce(args, &types, value_type.clone())?,
    };
    if let Some(compared) = super::arithmetic::exact_numeric_compare(
        value.clone(),
        op,
        right.clone(),
        &value_type,
        right_type,
    ) {
        return Ok(compared);
    }
    Ok(apply_compare(
        cast_to(value, &value_type, &unified),
        cast_to(right, right_type, &unified),
        op,
    ))
}

fn apply_compare(left: Expr, right: Expr, op: Operator) -> Expr {
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

fn cast_to(expr: Expr, from: &DataType, to: &DataType) -> Expr {
    if from == to {
        expr
    } else {
        cast(expr, to.clone())
    }
}

fn unify_coalesce(
    stored: Option<&DataType>,
    fallbacks: &[(Expr, DataType)],
    right: &DataType,
) -> Option<DataType> {
    let mut types = Vec::new();
    if let Some(stored) = stored {
        types.push(stored.clone());
    }
    types.extend(fallbacks.iter().map(|(_, data_type)| data_type.clone()));
    types.push(right.clone());
    let concrete: Vec<&DataType> = types
        .iter()
        .filter(|data_type| **data_type != DataType::Null)
        .collect();
    if concrete.is_empty() {
        return Some(DataType::Null);
    }
    if concrete.iter().all(|data_type| *data_type == concrete[0]) {
        return Some(concrete[0].clone());
    }
    if concrete.iter().all(|data_type| data_type.is_numeric()) {
        Some(DataType::Float64)
    } else {
        None
    }
}

fn type_conflict(
    stored: Option<&DataType>,
    fallbacks: &[(Expr, DataType)],
    right: &DataType,
) -> Option<(DataType, DataType)> {
    let mut concrete = Vec::new();
    if let Some(stored) = stored {
        if *stored != DataType::Null {
            concrete.push(stored.clone());
        }
    }
    for (_, data_type) in fallbacks {
        if *data_type != DataType::Null {
            concrete.push(data_type.clone());
        }
    }
    if *right != DataType::Null {
        concrete.push(right.clone());
    }
    let first = concrete.first()?.clone();
    concrete
        .into_iter()
        .find(|data_type| data_type != &first && !(first.is_numeric() && data_type.is_numeric()))
        .map(|data_type| (first, data_type))
}

fn predicate_or_literal(expr: &ast::Expr, variable: &str) -> Option<Local> {
    if let Some(local) = local_tree(expr, variable) {
        return Some(local);
    }
    if mentions(expr, variable) {
        return None;
    }
    Some(Local::Literal(expr.clone()))
}

fn value_coalesce_comparison(expr: &ast::Expr, variable: &str) -> Option<Local> {
    let ast::Expr::Binary { left, op, right } = expr else {
        return None;
    };
    let op = comparison_op(op)?;
    coalesce_comparison(left, op, right, variable)
        .or_else(|| coalesce_comparison(right, flip(op), left, variable))
}

fn coalesce_comparison(
    coalesced: &ast::Expr,
    op: Operator,
    value: &ast::Expr,
    variable: &str,
) -> Option<Local> {
    let ast::Expr::Coalesce(args) = coalesced else {
        return None;
    };
    let (first, fallbacks) = args.split_first()?;
    let property = property_of(first, variable)?;
    if mentions(value, variable) || fallbacks.iter().any(|arg| mentions(arg, variable)) {
        return None;
    }
    Some(Local::ValueCoalesce {
        property,
        fallbacks: fallbacks.to_vec(),
        op,
        value: value.clone(),
    })
}

fn comparison_op(op: &ast::BinaryOp) -> Option<Operator> {
    Some(match op {
        ast::BinaryOp::Eq => Operator::Eq,
        ast::BinaryOp::Neq => Operator::NotEq,
        ast::BinaryOp::Lt => Operator::Lt,
        ast::BinaryOp::Le => Operator::LtEq,
        ast::BinaryOp::Gt => Operator::Gt,
        ast::BinaryOp::Ge => Operator::GtEq,
        _ => return None,
    })
}

fn element_property<'a>(expr: &'a ast::Expr, scope: &Bindings) -> Option<(&'a str, &'a str)> {
    let ast::Expr::Property { base, key } = expr else {
        return None;
    };
    let ast::Expr::Identifier(name) = base.as_ref() else {
        return None;
    };
    fragment_binding(scope, &name.value)?;
    Some((&name.value, &key.value))
}

fn property_pair(
    plan: LogicalPlanBuilder,
    scope: &mut Bindings,
    expr: &ast::Expr,
) -> Result<(LogicalPlanBuilder, Option<Expr>)> {
    let ast::Expr::Binary { left, op, right } = expr else {
        return Ok((plan, None));
    };
    let op = match op {
        ast::BinaryOp::Eq => Operator::Eq,
        ast::BinaryOp::Neq => Operator::NotEq,
        ast::BinaryOp::Lt => Operator::Lt,
        ast::BinaryOp::Le => Operator::LtEq,
        ast::BinaryOp::Gt => Operator::Gt,
        ast::BinaryOp::Ge => Operator::GtEq,
        _ => return Ok((plan, None)),
    };
    let (Some((left_name, left_property)), Some((right_name, right_property))) = (
        element_property(left, scope),
        element_property(right, scope),
    ) else {
        return Ok((plan, None));
    };
    let left = fragment_binding(scope, left_name).expect("property element");
    let right = fragment_binding(scope, right_name).expect("property element");
    if !pair_needs_stored_types(scope, &left, left_property, &right, right_property)? {
        return Ok((plan, None));
    }
    let (plan, expr) = compile_property_pair(
        plan,
        scope,
        &left,
        left_property,
        op,
        &right,
        right_property,
    )?;
    let expr = reference_pair_guard(scope, left_name, expr);
    Ok((plan, Some(reference_pair_guard(scope, right_name, expr))))
}

fn reference_pair_guard(scope: &Bindings, name: &str, expr: Expr) -> Expr {
    if scope.elements.contains_key(name) {
        return expr;
    }
    let reference = scope.references.get(name).expect("resolved reference");
    let column = scope.scalars.get(name).expect("reference column").clone();
    Expr::Case(datafusion::logical_expr::expr::Case::new(
        None,
        vec![(
            Box::new(reference.guard(Expr::Column(column))),
            Box::new(expr),
        )],
        Some(Box::new(unknown())),
    ))
}

pub(super) fn equated_property(
    plan: LogicalPlanBuilder,
    scope: &mut Bindings,
    binding: &ElementBinding,
    property: &str,
    value: &ast::Expr,
) -> Result<(LogicalPlanBuilder, Option<Expr>)> {
    let Some((name, right_property)) = element_property(value, scope) else {
        return Ok((plan, None));
    };
    let right = fragment_binding(scope, name).expect("property element");
    if !pair_needs_stored_types(scope, binding, property, &right, right_property)? {
        return Ok((plan, None));
    }
    let (plan, expr) = compile_property_pair(
        plan,
        scope,
        binding,
        property,
        Operator::Eq,
        &right,
        right_property,
    )?;
    Ok((plan, Some(reference_pair_guard(scope, name, expr))))
}

fn pair_needs_stored_types(
    scope: &Bindings,
    left: &ElementBinding,
    left_property: &str,
    right: &ElementBinding,
    right_property: &str,
) -> Result<bool> {
    let Some(left_data) = scope.sources.get(&left.graph) else {
        return Ok(false);
    };
    let Some(right_data) = scope.sources.get(&right.graph) else {
        return Ok(false);
    };
    let left_tables = element_tables(left_data, left);
    let right_tables = element_tables(right_data, right);
    let types = |tables: &[&Table], property: &str| {
        tables
            .iter()
            .filter_map(|t| {
                crate::graph::property_field(&t.schema, property).map(|f| f.data_type().clone())
            })
            .collect::<Vec<_>>()
    };
    let left_types = types(&left_tables, left_property);
    let right_types = types(&right_tables, right_property);
    let mut comparable_pair = false;
    let mut incompatible_pair = false;
    for a in &left_types {
        for b in &right_types {
            if comparable(a, b) {
                comparable_pair = true;
            } else {
                incompatible_pair = true;
            }
        }
    }
    if !comparable_pair && !left_types.is_empty() && !right_types.is_empty() {
        return Err(Error::InvalidQuery(format!(
            "incomparable types {} and {}",
            left_types[0], right_types[0]
        )));
    }
    // A unified numeric column can round a large integer before comparison.
    let widened = [&left_types, &right_types]
        .into_iter()
        .any(|types| types.iter().any(|ty| types.first() != Some(ty)));
    Ok(comparable_pair && (incompatible_pair || widened))
}

#[allow(clippy::too_many_arguments)]
fn compile_property_pair(
    plan: LogicalPlanBuilder,
    scope: &mut Bindings,
    left: &ElementBinding,
    left_property: &str,
    op: Operator,
    right: &ElementBinding,
    right_property: &str,
) -> Result<(LogicalPlanBuilder, Expr)> {
    let (plan, left_fragments) = attach_fragments(plan, scope, left)?;
    let (plan, right_fragments) = attach_fragments(plan, scope, right)?;
    let mut parts = Vec::new();
    for a in &left_fragments {
        for b in &right_fragments {
            if left.alias == right.alias && a.alias != b.alias {
                continue;
            }
            let Some(field) = crate::graph::property_field(&b.schema, right_property) else {
                continue;
            };
            let value = qualified_property(&b.schema, Some(&b.alias), right_property);
            let comparison = compare_property(
                &a.schema,
                &qualified_property(&a.schema, Some(&a.alias), left_property),
                left_property,
                op,
                value,
                field.data_type(),
            )?;
            parts.push(comparison);
        }
    }
    let expr = match parts.len() {
        0 => unknown(),
        1 => parts.pop().expect("property comparison"),
        _ => super::expressions::boolean_value(cast(coalesce(parts), DataType::Int64)),
    };
    let expr = left.ensure_live(
        right.ensure_live(expr, DataType::Boolean),
        DataType::Boolean,
    );
    Ok((plan, expr))
}

/// Native value lowering only adds fragment joins; compare their counts before
/// executing a barrier so ordinary expressions remain on their original plan.
pub(super) fn added_fragment_joins(
    before: &datafusion::logical_expr::LogicalPlan,
    after: &datafusion::logical_expr::LogicalPlan,
) -> bool {
    fn joins(plan: &datafusion::logical_expr::LogicalPlan) -> usize {
        usize::from(matches!(
            plan,
            datafusion::logical_expr::LogicalPlan::Join(_)
        )) + plan
            .inputs()
            .iter()
            .map(|input| joins(input))
            .sum::<usize>()
    }
    joins(after) > joins(before)
}

/// Preserve working-row order even when a preceding reference lookup has
/// already materialized the sort into a buffered scan.
pub(super) async fn fragment_boolean_ordered(
    plan: LogicalPlanBuilder,
    scope: &mut Bindings,
    session: &SessionState,
    expression: &ast::Expr,
    ctx: &datafusion::execution::context::SessionContext,
    trace: &mut super::execution::Trace,
) -> Result<(LogicalPlanBuilder, Option<Expr>)> {
    let (next, value) = fragment_boolean(plan.clone(), scope, session, expression)?;
    if !added_fragment_joins(plan.plan(), next.plan()) {
        return Ok((next, value));
    }
    let order = scope.fresh("fragment_order");
    let plan = super::execution::ordinal(plan, ctx, trace, &order).await?;
    let (next, value) = fragment_boolean(plan, scope, session, expression)?;
    let columns = next
        .schema()
        .columns()
        .into_iter()
        .filter(|column| column.name != order)
        .map(Expr::Column)
        .collect::<Vec<_>>();
    let next = next
        .sort([Expr::Column(Column::new_unqualified(order)).sort(true, false)])?
        .project(columns)?;
    Ok((next, value))
}

pub(super) async fn fragment_predicate_ordered(
    plan: LogicalPlanBuilder,
    scope: &mut Bindings,
    session: &SessionState,
    predicate: &ast::Expr,
    ctx: &datafusion::execution::context::SessionContext,
    trace: &mut super::execution::Trace,
) -> Result<Option<LogicalPlanBuilder>> {
    let columns = plan.schema().columns();
    let (plan, value) =
        fragment_boolean_ordered(plan, scope, session, predicate, ctx, trace).await?;
    let Some(value) = value else {
        return Ok(None);
    };
    let value = predicate_value(value, plan.schema())?;
    Ok(Some(
        plan.filter(value)?
            .project(columns.into_iter().map(Expr::Column))?,
    ))
}

pub(super) fn contains_aggregate(expression: &ast::Expr) -> bool {
    operators::contains_aggregate(expression)
}
