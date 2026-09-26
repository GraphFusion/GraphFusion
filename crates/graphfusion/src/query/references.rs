//! Resolve scalar graph references through native DataFusion left joins.
use super::{
    graph::{self, Bindings, ElementBinding, ElementKind},
    path_values,
};
use crate::{
    catalog::ObjectId,
    gql as ast,
    graph::{GraphData, ID},
    Error, Result,
};
use datafusion::{
    arrow::{
        array::{Array, BooleanArray},
        datatypes::DataType,
    },
    common::{DFSchema, ScalarValue, TableReference},
    error::DataFusionError,
    functions::core::expr_fn::{coalesce, get_field},
    logical_expr::{
        create_udf, lit, ColumnarValue, Expr, ExprSchemable, JoinType, LogicalPlanBuilder,
        Volatility,
    },
};
use std::{collections::BTreeSet, sync::Arc};

pub(super) type Domain = BTreeSet<(ObjectId, ElementKind)>;

pub(super) fn domain_for_name(name: &str, scope: &Bindings) -> Option<Domain> {
    scope
        .elements
        .get(name)
        .map(|e| [(e.graph, e.kind)].into())
        .or_else(|| scope.domains.get(name).cloned())
}

pub(super) fn domain(expr: &ast::Expr, scope: &Bindings) -> Option<Domain> {
    match expr {
        ast::Expr::Identifier(name) => domain_for_name(&name.value, scope),
        ast::Expr::Literal(ast::Literal::Null) => Some(Domain::new()),
        ast::Expr::List(values) | ast::Expr::Coalesce(values) => {
            let mut result = Domain::new();
            for value in values {
                result.extend(domain(value, scope)?);
            }
            Some(result)
        }
        ast::Expr::NullIf { left, .. } => domain(left, scope),
        ast::Expr::Elements { path } => domain(path, scope),
        ast::Expr::Function { name, args, .. }
            if name.value.eq_ignore_ascii_case("COLLECT_LIST") && args.len() == 1 =>
        {
            domain(&args[0], scope)
        }
        _ => None,
    }
}

pub(super) fn result_domains(
    body: &ast::QueryBody,
    scope: &Bindings,
) -> std::collections::BTreeMap<String, Domain> {
    let mut result = std::collections::BTreeMap::new();
    for (index, item) in body.result_clause.items.iter().enumerate() {
        if matches!(item.expr, ast::Expr::Wildcard) {
            for name in &scope.order {
                if let Some(domain) = domain_for_name(name, scope) {
                    result.insert(name.clone(), domain);
                }
            }
        } else if let Some(domain) = domain(&item.expr, scope) {
            let name = item
                .alias
                .as_ref()
                .map(|n| n.value.clone())
                .unwrap_or_else(|| match &item.expr {
                    ast::Expr::Identifier(name) => name.value.clone(),
                    _ => format!("column_{}", index + 1),
                });
            result.insert(name, domain);
        }
    }
    result
}

#[derive(Clone)]
pub(super) struct ReferenceBinding {
    pub parts: Vec<ElementBinding>,
}
impl ReferenceBinding {
    pub fn guard(&self, reference: Expr) -> Expr {
        let present = self
            .parts
            .iter()
            .map(|p| p.column(ID).is_not_null())
            .reduce(Expr::or)
            .unwrap_or_else(|| lit(false));
        // A deleted reference must not read stale or fabricated properties. This
        // scalar check only validates join presence; it never accesses storage.
        create_udf(
            "gql_live_reference",
            vec![DataType::Boolean, DataType::Boolean],
            DataType::Boolean,
            Volatility::Immutable,
            Arc::new(|args| {
                let arrays = ColumnarValue::values_to_arrays(args)?;
                let null = arrays[0]
                    .as_any()
                    .downcast_ref::<BooleanArray>()
                    .expect("boolean input");
                let live = arrays[1]
                    .as_any()
                    .downcast_ref::<BooleanArray>()
                    .expect("boolean input");
                if (0..null.len()).any(|i| !null.value(i) && !live.value(i)) {
                    return Err(DataFusionError::Execution(
                        "dereference of a deleted or unavailable graph element".into(),
                    ));
                }
                Ok(args[1].clone())
            }),
        )
        .call(vec![reference.is_null(), present])
    }
    fn guarded(&self, reference: Expr, value: Expr) -> Expr {
        Expr::Case(datafusion::logical_expr::expr::Case::new(
            None,
            vec![(Box::new(self.guard(reference)), Box::new(value))],
            Some(Box::new(lit(ScalarValue::Null))),
        ))
    }
    pub fn property(&self, reference: Expr, key: &str, schema: &DFSchema) -> Result<Expr> {
        let values: Vec<_> = self
            .parts
            .iter()
            .filter(|p| p.properties.contains_key(key))
            .map(|p| p.property(key))
            .collect();
        let mut previous = DataType::Null;
        for value in &values {
            let ty = value.get_type(schema)?;
            if previous != DataType::Null
                && previous != ty
                && !(previous.is_numeric() && ty.is_numeric())
            {
                return Err(Error::InvalidQuery(format!(
                    "incompatible types for reference property {key}: {previous} and {ty}"
                )));
            }
            previous = ty;
        }
        Ok(self.guarded(
            reference,
            if values.is_empty() {
                lit(ScalarValue::Null)
            } else {
                coalesce(values)
            },
        ))
    }
    pub fn property_exists(&self, reference: Expr, key: &str) -> Expr {
        self.guarded(
            reference,
            coalesce(
                self.parts
                    .iter()
                    .map(|p| p.property_exists(key))
                    .chain([lit(false)])
                    .collect(),
            ),
        )
    }
    pub fn label(&self, reference: Expr, label: &ast::LabelExpression) -> Expr {
        self.guarded(
            reference,
            coalesce(
                self.parts
                    .iter()
                    .map(|p| p.label(label))
                    .chain([lit(false)])
                    .collect(),
            ),
        )
    }
    pub fn directed(&self, reference: Expr) -> Expr {
        let edge = kind_check(reference.clone(), "e");
        let values = self
            .parts
            .iter()
            .filter(|p| p.kind == ElementKind::Edge)
            .map(|p| p.column("__gf_directed"))
            .chain([lit(ScalarValue::Boolean(None))])
            .collect();
        self.guarded(
            reference,
            Expr::Case(datafusion::logical_expr::expr::Case::new(
                None,
                vec![(Box::new(edge), Box::new(coalesce(values)))],
                Some(Box::new(lit(ScalarValue::Boolean(None)))),
            )),
        )
    }
}

pub(super) fn kind_check(reference: Expr, expected: &'static str) -> Expr {
    create_udf(
        if expected == "e" {
            "gql_require_edge"
        } else {
            "gql_require_node"
        },
        vec![DataType::Boolean],
        DataType::Boolean,
        Volatility::Immutable,
        Arc::new(move |args| {
            let arrays = ColumnarValue::values_to_arrays(args)?;
            let values = arrays[0]
                .as_any()
                .downcast_ref::<BooleanArray>()
                .expect("boolean kind");
            if values.iter().flatten().any(|v| !v) {
                return Err(DataFusionError::Execution(format!(
                    "expected {} reference",
                    if expected == "e" { "edge" } else { "node" }
                )));
            }
            Ok(args[0].clone())
        }),
    )
    .call(vec![
        get_field(reference, path_values::KIND).eq(lit(expected))
    ])
}

pub(super) fn constraint(reference: Expr, binding: &ElementBinding) -> Expr {
    get_field(reference.clone(), path_values::GRAPH)
        .eq(lit(binding.graph))
        .and(get_field(reference.clone(), path_values::KIND).eq(lit(
            if binding.kind == ElementKind::Node {
                "n"
            } else {
                "e"
            },
        )))
        .and(get_field(reference, path_values::ID).eq(binding.column(ID)))
}

pub(super) fn resolve(
    mut plan: LogicalPlanBuilder,
    scope: &mut Bindings,
) -> Result<LogicalPlanBuilder> {
    let names = scope
        .scalars
        .iter()
        .filter(|(name, _)| !scope.references.contains_key(*name))
        .map(|(name, column)| (name.clone(), column.clone()))
        .collect::<Vec<_>>();
    for (name, column) in names {
        let reference = Expr::Column(column);
        if reference.get_type(plan.schema())? != path_values::data_type() {
            continue;
        }
        let keys = [path_values::GRAPH, path_values::KIND, path_values::ID]
            .map(|field| (field, scope.fresh("reference_key")));
        let mut projection: Vec<_> = plan
            .schema()
            .columns()
            .into_iter()
            .map(Expr::Column)
            .collect();
        projection.extend(
            keys.iter()
                .map(|(field, name)| get_field(reference.clone(), *field).alias(name)),
        );
        plan = plan.project(projection)?;
        let key = |i: usize| Expr::Column(datafusion::common::Column::new_unqualified(&keys[i].1));
        let sources = scope.sources.clone();
        let mut parts = Vec::new();
        for (id, data) in sources {
            for kind in [ElementKind::Node, ElementKind::Edge] {
                if scope
                    .domains
                    .get(&name)
                    .is_some_and(|domain| !domain.contains(&(id, kind)))
                {
                    continue;
                }
                let (scan, binding) = graph::scan(scope, id, &data, kind, None)?;
                let matched = key(0)
                    .eq(lit(id))
                    .and(key(1).eq(lit(if kind == ElementKind::Node { "n" } else { "e" })))
                    .and(key(2).eq(binding.column(ID)));
                plan = plan.join_on(scan, JoinType::Left, [matched])?;
                parts.push(binding);
            }
        }
        scope.references.insert(name, ReferenceBinding { parts });
    }
    Ok(plan)
}

pub(super) async fn prepare(
    plan: LogicalPlanBuilder,
    scope: &mut Bindings,
    ctx: &datafusion::execution::context::SessionContext,
    trace: &mut super::execution::Trace,
) -> Result<LogicalPlanBuilder> {
    let mut new_reference = false;
    for (name, column) in &scope.scalars {
        if !scope.references.contains_key(name)
            && Expr::Column(column.clone()).get_type(plan.schema())? == path_values::data_type()
        {
            new_reference = true;
        }
    }
    if !new_reference {
        return Ok(plan);
    }
    // Isolate encoded/unnested reference values from their lookup joins. Otherwise
    // DF55 common-expression and leaf-projection rewrites can duplicate aliases.
    let plan = super::execution::freeze(plan, ctx, trace, None).await?;
    let plan = resolve(plan, scope)?;
    // Keep subsequent repeated property/identity expressions above the lookup
    // schema too; DF55 can otherwise push their CSE aliases through these joins.
    super::execution::freeze(plan, ctx, trace, None).await
}

pub(super) fn refresh(
    mut plan: LogicalPlanBuilder,
    scope: &mut Bindings,
    graph: ObjectId,
    data: &GraphData,
) -> Result<LogicalPlanBuilder> {
    scope.sources.insert(graph, Arc::new(data.clone()));
    // Drop only lookup columns. LET values were materialized before the write,
    // and scalar reference identities remain in their original binding columns.
    let mut aliases = BTreeSet::new();
    let names: Vec<_> = scope
        .references
        .iter()
        .filter(|(_, b)| b.parts.iter().any(|p| p.graph == graph))
        .map(|(name, _)| name.clone())
        .collect();
    for name in names {
        let old = scope.references.remove(&name).expect("existing reference");
        aliases.extend(
            old.parts
                .iter()
                .map(|p| TableReference::bare(p.alias.clone())),
        );
    }
    let columns: Vec<_> = plan
        .schema()
        .columns()
        .into_iter()
        .filter(|c| !c.relation.as_ref().is_some_and(|r| aliases.contains(r)))
        .map(Expr::Column)
        .collect();
    plan = plan.project(columns)?;
    resolve(plan, scope)
}

pub(super) fn scalar(scope: &Bindings, name: &str, schema: &DFSchema) -> Result<Option<Expr>> {
    let Some(column) = scope.scalars.get(name) else {
        return Ok(None);
    };
    let value = Expr::Column(column.clone());
    let ty = value.get_type(schema)?;
    if ty == DataType::Null {
        return Ok(Some(datafusion::logical_expr::cast(
            value,
            path_values::data_type(),
        )));
    }
    if ty != path_values::data_type() {
        return Err(Error::InvalidQuery(format!(
            "{name} is not an element reference"
        )));
    }
    Ok(Some(value))
}
