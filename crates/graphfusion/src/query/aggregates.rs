use super::{expressions::Binder, graph::Bindings};
use crate::{gql as ast, Error, Result, SessionState};
use datafusion::{
    arrow::datatypes::DataType,
    common::{
        tree_node::{Transformed, TreeNode},
        Column, DFSchema, ScalarValue,
    },
    functions_aggregate as agg,
    logical_expr::{
        cast, lit, utils::find_aggregate_exprs, Expr, ExprSchemable, LogicalPlanBuilder,
    },
};
use std::collections::BTreeMap;

pub(super) fn call(
    name: &str,
    quantifier: Option<ast::SetQuantifier>,
    mut args: Vec<Expr>,
    star: bool,
    schema: &DFSchema,
) -> Result<Expr> {
    if !find_aggregate_exprs(&args).is_empty() {
        return Err(Error::InvalidQuery("nested aggregate functions".into()));
    }
    let name = name.to_ascii_uppercase();
    let distinct = quantifier == Some(ast::SetQuantifier::Distinct);
    if star {
        if name != "COUNT" || distinct {
            return Err(Error::InvalidQuery(
                "only COUNT(*) accepts an asterisk".into(),
            ));
        }
        return Ok(agg::count::count_all());
    }
    let expected = if matches!(name.as_str(), "PERCENTILE_CONT" | "PERCENTILE_DISC") {
        2
    } else {
        1
    };
    if args.len() != expected {
        return Err(Error::InvalidQuery(format!(
            "{name} requires {expected} arguments"
        )));
    }
    let mut value = args.remove(0);
    let mut data_type = value.get_type(schema)?;
    if matches!(
        name.as_str(),
        "SUM" | "AVG" | "STDDEV_POP" | "STDDEV_SAMP" | "PERCENTILE_CONT" | "PERCENTILE_DISC"
    ) {
        if data_type == DataType::Null {
            value = cast(value, DataType::Int64);
            data_type = DataType::Int64;
        }
        if !data_type.is_numeric() {
            return Err(Error::InvalidQuery(format!(
                "{name} requires a numeric argument"
            )));
        }
    }
    let mut sum_integer = false;
    let mut collect = false;
    let mut expression = match name.as_str() {
        "COUNT" => agg::expr_fn::count(value),
        "SUM" => {
            // Widen accumulation, then use a strict cast to detect an Int64 result overflow.
            sum_integer = data_type == DataType::Int64;
            agg::expr_fn::sum(if sum_integer {
                cast(value, DataType::Decimal128(38, 0))
            } else {
                value
            })
        }
        "AVG" => agg::expr_fn::avg(value),
        "MIN" => agg::expr_fn::min(value),
        "MAX" => agg::expr_fn::max(value),
        "STDDEV_POP" => agg::expr_fn::stddev_pop(value),
        "STDDEV_SAMP" => agg::expr_fn::stddev(value),
        "COLLECT_LIST" => {
            collect = true;
            let mut expression = agg::expr_fn::array_agg(value.clone());
            if let Expr::AggregateFunction(function) = &mut expression {
                function.params.filter = Some(Box::new(value.is_not_null()));
            }
            expression
        }
        "PERCENTILE_CONT" | "PERCENTILE_DISC" => {
            let fraction = args.remove(0);
            if !fraction.column_refs().is_empty() {
                return Err(Error::InvalidQuery(
                    "percentile must be independent of input rows".into(),
                ));
            }
            let fraction_type = fraction.get_type(schema)?;
            if !fraction_type.is_numeric() && fraction_type != DataType::Null {
                return Err(Error::InvalidQuery(
                    "percentile requires a numeric fraction".into(),
                ));
            }
            percentile(value, fraction, distinct, name == "PERCENTILE_CONT")?
        }

        _ => return Err(super::unsupported(&format!("aggregate function {name}"))),
    };
    if let Expr::AggregateFunction(function) = &mut expression {
        function.params.distinct = distinct;
    }
    if sum_integer {
        expression = cast(expression, DataType::Int64);
    }
    if collect {
        let empty = ScalarValue::List(ScalarValue::new_list(&[], &data_type, true));
        expression = datafusion::functions::core::expr_fn::coalesce(vec![expression, lit(empty)]);
    }
    if expression.get_type(schema)? == DataType::Float64 {
        expression = super::arithmetic::finite(expression);
    }
    Ok(expression)
}

/// Percentiles use DataFusion's array aggregate/sort/extract kernels, including
/// DISTINCT before interpolation and exact input values for the discrete case.
fn percentile(value: Expr, fraction: Expr, distinct: bool, continuous: bool) -> Result<Expr> {
    use datafusion::{
        functions::math::expr_fn::{ceil, floor},
        functions_nested::expr_fn::{array_element, array_length, array_sort},
    };
    let mut values = agg::expr_fn::array_agg(value.clone());
    if let Expr::AggregateFunction(function) = &mut values {
        function.params.distinct = distinct;
        function.params.filter = Some(Box::new(value.is_not_null()));
    }
    let values = array_sort(values, lit("ASC"), lit("NULLS LAST"));
    let count = cast(array_length(values.clone()), DataType::Float64);
    let fraction = percentile_fraction(cast(fraction, DataType::Float64));
    let result = if continuous {
        let position = fraction.clone() * (count - lit(1.0_f64));
        let low = floor(position.clone());
        let high = ceil(position.clone());
        let weight = position - low.clone();
        let a = cast(
            array_element(values.clone(), cast(low + lit(1.0_f64), DataType::Int64)),
            DataType::Float64,
        );
        let b = cast(
            array_element(values, cast(high + lit(1.0_f64), DataType::Int64)),
            DataType::Float64,
        );
        (lit(1.0_f64) - weight.clone()) * a + weight * b
    } else {
        let rank = datafusion::functions::core::expr_fn::greatest(vec![
            ceil(fraction.clone() * count),
            lit(1.0_f64),
        ]);
        array_element(values, cast(rank, DataType::Int64))
    };
    Ok(Expr::Case(datafusion::logical_expr::expr::Case::new(
        None,
        vec![(
            Box::new(fraction.is_null()),
            Box::new(lit(ScalarValue::Null)),
        )],
        Some(Box::new(result)),
    )))
}

fn percentile_fraction(value: Expr) -> Expr {
    use datafusion::{
        arrow::array::Float64Array,
        error::DataFusionError,
        logical_expr::{create_udf, ColumnarValue, Volatility},
    };
    create_udf(
        "gql_percentile_fraction",
        vec![DataType::Float64],
        DataType::Float64,
        Volatility::Immutable,
        std::sync::Arc::new(|args| {
            let arrays = ColumnarValue::values_to_arrays(args)?;
            let values = arrays[0]
                .as_any()
                .downcast_ref::<Float64Array>()
                .ok_or_else(|| {
                    DataFusionError::Internal("expected Float64 percentile fraction".into())
                })?;
            if values.iter().flatten().any(|v| !(0.0..=1.0).contains(&v)) {
                return Err(DataFusionError::Execution(
                    "percentile must be between 0 and 1".into(),
                ));
            }
            Ok(args[0].clone())
        }),
    )
    .call(vec![value])
}

pub(super) fn result(
    mut plan: LogicalPlanBuilder,
    session: &SessionState,
    scope: &Bindings,
    body: &ast::QueryBody,
) -> Result<LogicalPlanBuilder> {
    let binder = Binder::with_bindings(session, plan.schema(), scope).aggregating();
    let mut aliases = BTreeMap::new();
    let mut projections = Vec::new();
    for (i, item) in body.result_clause.items.iter().enumerate() {
        if matches!(item.expr, ast::Expr::Wildcard) {
            if item.alias.is_some() {
                return Err(Error::InvalidQuery("wildcard cannot have an alias".into()));
            }
            for name in &scope.order {
                let expr = binder.bind(&ast::Expr::Identifier(ast::Identifier::new(name)))?;
                if aliases.insert(name.clone(), expr.clone()).is_some() {
                    return Err(Error::InvalidQuery("duplicate result column".into()));
                }
                projections.push((name.clone(), expr));
            }
        } else {
            let name = item
                .alias
                .as_ref()
                .map(|name| name.value.clone())
                .unwrap_or_else(|| match &item.expr {
                    ast::Expr::Identifier(name) => name.value.clone(),
                    _ => format!("column_{}", i + 1),
                });
            let expr = binder.bind(&item.expr)?;
            if aliases.insert(name.clone(), expr.clone()).is_some() {
                return Err(Error::InvalidQuery(format!(
                    "duplicate result column {name}"
                )));
            }
            projections.push((name, expr));
        }
    }
    if projections.is_empty() {
        return Err(Error::InvalidQuery("result has no bound columns".into()));
    }
    let binder = Binder::with_bindings(session, plan.schema(), scope)
        .aggregating()
        .aliases(&aliases);
    let having = body
        .having
        .as_ref()
        .map(|p| binder.predicate(p))
        .transpose()?;
    let ordering = body
        .order_by
        .iter()
        .map(|o| binder.bind(&o.expr))
        .collect::<Result<Vec<_>>>()?;
    let mut all_expressions: Vec<_> = projections.iter().map(|(_, e)| e).collect();
    all_expressions.extend(having.iter());
    all_expressions.extend(&ordering);
    let mut aggregates = find_aggregate_exprs(all_expressions);
    let explicit = !body.group_by.is_empty() || body.empty_grouping_set;
    let grouped = explicit || !aggregates.is_empty() || having.is_some();
    let mut groups = Vec::new();
    if explicit {
        for expr in &body.group_by {
            if let ast::Expr::Identifier(name) = expr {
                if !aliases.contains_key(&name.value) {
                    if let Some(binding) = scope.elements.get(&name.value) {
                        groups.extend(
                            plan.schema()
                                .columns()
                                .into_iter()
                                .filter(|c| {
                                    c.relation
                                        .as_ref()
                                        .is_some_and(|r| r.table() == binding.alias)
                                })
                                .map(Expr::Column),
                        );
                        continue;
                    }
                }
            }
            let group = binder.bind(expr)?;
            if !find_aggregate_exprs([&group]).is_empty() {
                return Err(Error::InvalidQuery("aggregate in GROUP BY".into()));
            }
            if !groups.contains(&group) {
                groups.push(group);
            }
        }
    } else if grouped {
        for (_, expr) in &projections {
            if find_aggregate_exprs([expr]).is_empty()
                && !expr.column_refs().is_empty()
                && !groups.contains(expr)
            {
                groups.push(expr.clone());
            }
        }
    }
    let mut mapping = Vec::new();
    if grouped {
        if groups.is_empty() && aggregates.is_empty() {
            aggregates.push(agg::count::count_all());
        }
        let group_exprs: Vec<_> = groups
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let name = format!("__gf_group_{i}");
                mapping.push((e.clone(), Column::new_unqualified(&name)));
                e.clone().alias(name)
            })
            .collect();
        let agg_exprs: Vec<_> = aggregates
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let name = format!("__gf_aggregate_{i}");
                mapping.push((e.clone(), Column::new_unqualified(&name)));
                e.clone().alias(name)
            })
            .collect();
        plan = plan.aggregate(group_exprs, agg_exprs)?;
    }
    let rewrite = |expr: Expr| -> Result<Expr> {
        if !grouped {
            Ok(expr)
        } else {
            rebase(expr, &mapping)
        }
    };
    if let Some(having) = having {
        plan = plan.filter(rewrite(having)?)?;
    }
    let projected = projections
        .into_iter()
        .map(|(name, expr)| Ok(rewrite(expr)?.alias(name)))
        .collect::<Result<Vec<_>>>()?;
    let ordering = if body.result_clause.distinct {
        plan = plan.project(projected.clone())?.distinct()?;
        let output = aliases
            .into_iter()
            .map(|(name, expr)| (expr, Column::new_unqualified(name)))
            .collect::<Vec<_>>();
        ordering
            .into_iter()
            .map(|e| rebase(e, &output))
            .collect::<Result<Vec<_>>>()?
    } else {
        ordering
            .into_iter()
            .map(rewrite)
            .collect::<Result<Vec<_>>>()?
    };
    if !ordering.is_empty() {
        let ordering = ordering
            .into_iter()
            .zip(&body.order_by)
            .map(|(expr, o)| {
                let asc = o.direction != Some(ast::SortDirection::Desc);
                let nulls_first = match o.null_ordering {
                    Some(ast::NullOrdering::First) => true,
                    Some(ast::NullOrdering::Last) => false,
                    None => !asc,
                };
                Ok(expr.sort(asc, nulls_first))
            })
            .collect::<Result<Vec<_>>>()?;
        plan = plan.sort(ordering)?;
    }
    super::order_and_page(
        if body.result_clause.distinct {
            plan
        } else {
            plan.project(projected)?
        },
        session,
        &[],
        body.offset.as_ref(),
        body.limit.as_ref(),
        None,
    )
}

fn rebase(expr: Expr, mapping: &[(Expr, Column)]) -> Result<Expr> {
    let transformed = expr.transform_down(|expr| {
        if let Some((_, column)) = mapping.iter().find(|(source, _)| source == &expr) {
            return Ok(Transformed::yes(Expr::Column(column.clone())));
        }
        if matches!(expr, Expr::Column(_)) {
            return Err(datafusion::error::DataFusionError::Plan(format!(
                "expression is not grouped: {expr}"
            )));
        }
        Ok(Transformed::no(expr))
    })?;
    Ok(transformed.data)
}
