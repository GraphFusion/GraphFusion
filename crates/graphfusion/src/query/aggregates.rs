use super::casefold::CaseFoldMap;
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
        "AVG" if data_type == DataType::Int64 => {
            // Sum in decimal, then divide. Float addition drops bits above 2^53
            // before the division, so the rounded quotient can be wrong.
            let mut sum = agg::expr_fn::sum(cast(value.clone(), DataType::Decimal128(38, 0)));
            let mut count = agg::expr_fn::count(value);
            if distinct {
                for aggregate in [&mut sum, &mut count] {
                    if let Expr::AggregateFunction(function) = aggregate {
                        function.params.distinct = true;
                    }
                }
            }
            let sum_type = sum.get_type(schema)?;
            let average = super::arithmetic::exact_ratio(sum, sum_type, count.clone());
            Expr::Case(datafusion::logical_expr::expr::Case::new(
                None,
                vec![(
                    Box::new(count.eq(lit(0_i64))),
                    Box::new(lit(ScalarValue::Float64(None))),
                )],
                Some(Box::new(average)),
            ))
        }
        "AVG" => agg::expr_fn::avg(value),
        "MIN" => agg::expr_fn::min(value),
        "MAX" => agg::expr_fn::max(value),
        "STDDEV_POP" | "STDDEV_SAMP" => {
            if data_type == DataType::Int64 {
                super::stddev::integer_stddev(value, name == "STDDEV_POP")
            } else if distinct {
                distinct_stddev(value, name == "STDDEV_SAMP")?
            } else if name == "STDDEV_POP" {
                agg::expr_fn::stddev_pop(value)
            } else {
                agg::expr_fn::stddev(value)
            }
        }
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
            percentile(
                value,
                fraction,
                distinct,
                name == "PERCENTILE_CONT",
                data_type == DataType::Int64,
            )?
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
/// Integer continuous percentiles interpolate those exact endpoints.
fn percentile(
    value: Expr,
    fraction: Expr,
    distinct: bool,
    continuous: bool,
    integer: bool,
) -> Result<Expr> {
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
        let position = percentile_product(fraction.clone(), count - lit(1.0_f64));
        let low = floor(position.clone());
        let high = ceil(position.clone());
        let weight = position - low.clone();
        let a = array_element(values.clone(), cast(low + lit(1.0_f64), DataType::Int64));
        let b = array_element(values, cast(high + lit(1.0_f64), DataType::Int64));
        if integer {
            super::arithmetic::exact_lerp(a, b, weight)
        } else {
            let a = cast(a, DataType::Float64);
            let b = cast(b, DataType::Float64);
            (lit(1.0_f64) - weight.clone()) * a + weight * b
        }
    } else {
        let rank = datafusion::functions::core::expr_fn::greatest(vec![
            ceil(percentile_product(fraction.clone(), count)),
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

/// `STDDEV_*(DISTINCT)` aggregates the distinct numbers, then takes the deviation.
/// Values are sorted first so the one-pass update does not depend on group order.
fn distinct_stddev(value: Expr, sample: bool) -> Result<Expr> {
    let mut values = agg::expr_fn::array_agg(value.clone());
    if let Expr::AggregateFunction(function) = &mut values {
        function.params.distinct = true;
        function.params.filter = Some(Box::new(value.is_not_null()));
    }
    Ok(stddev_values(values, sample))
}

fn stddev_values(values: Expr, sample: bool) -> Expr {
    use datafusion::logical_expr::ScalarUDF;
    static POP: std::sync::OnceLock<ScalarUDF> = std::sync::OnceLock::new();
    static SAMP: std::sync::OnceLock<ScalarUDF> = std::sync::OnceLock::new();
    let udf = if sample { &SAMP } else { &POP };
    udf.get_or_init(|| stddev_values_udf(sample))
        .call(vec![values])
}

fn stddev_values_udf(sample: bool) -> datafusion::logical_expr::ScalarUDF {
    use datafusion::{
        arrow::array::{Array, Float64Array, ListArray},
        error::DataFusionError,
        logical_expr::{create_udf, ColumnarValue, Volatility},
    };
    create_udf(
        if sample {
            "gql_stddev_samp_values"
        } else {
            "gql_stddev_pop_values"
        },
        vec![DataType::new_list(DataType::Float64, true)],
        DataType::Float64,
        Volatility::Immutable,
        std::sync::Arc::new(move |args| {
            let arrays = ColumnarValue::values_to_arrays(args)?;
            let lists = arrays[0]
                .as_any()
                .downcast_ref::<ListArray>()
                .ok_or_else(|| {
                    DataFusionError::Internal("expected Float64 deviation list".into())
                })?;
            let mut output = Vec::with_capacity(lists.len());
            for row in 0..lists.len() {
                let deviation = if lists.is_null(row) {
                    None
                } else {
                    let child = lists.value(row);
                    let numbers =
                        child
                            .as_any()
                            .downcast_ref::<Float64Array>()
                            .ok_or_else(|| {
                                DataFusionError::Internal(
                                    "expected Float64 deviation values".into(),
                                )
                            })?;
                    deviation_of(numbers.iter().flatten(), sample)
                };
                output.push(deviation);
            }
            Ok(ColumnarValue::Array(std::sync::Arc::new(
                Float64Array::from(output),
            )))
        }),
    )
}

fn deviation_of(values: impl Iterator<Item = f64>, sample: bool) -> Option<f64> {
    let mut numbers: Vec<f64> = values.collect();
    if numbers.is_empty() || (sample && numbers.len() < 2) {
        return None;
    }
    if numbers.iter().any(|value| !value.is_finite()) {
        return Some(f64::NAN);
    }
    numbers.sort_by(f64::total_cmp);
    // Recenter before scaling, retaining small differences between nearby large
    // values while avoiding overflow for inputs with opposite signs.
    let center = numbers[0] * 0.5 + numbers[numbers.len() - 1] * 0.5;
    let scale = numbers
        .iter()
        .map(|value| (value - center).abs())
        .fold(0.0_f64, f64::max);
    if scale == 0.0 {
        return Some(0.0);
    }
    let mut count = 0_u64;
    let mut mean = 0.0_f64;
    let mut square = 0.0_f64;
    for value in numbers {
        let value = (value - center) / scale;
        count += 1;
        let delta = value - mean;
        mean += delta / count as f64;
        square += delta * (value - mean);
    }
    let denominator = if sample { count - 1 } else { count };
    Some((square / denominator as f64).sqrt() * scale)
}

fn percentile_fraction(value: Expr) -> Expr {
    use datafusion::{
        arrow::array::Float64Array,
        error::DataFusionError,
        logical_expr::{create_udf, ColumnarValue, ScalarUDF, Volatility},
    };
    use std::sync::OnceLock;
    // One range check for every percentile. A fresh closure would not compare
    // equal, so DISTINCT could not recognize the same percentile in ORDER BY.
    fn udf() -> ScalarUDF {
        static UDF: OnceLock<ScalarUDF> = OnceLock::new();
        UDF.get_or_init(|| {
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
        })
        .clone()
    }
    udf().call(vec![value])
}

/// Correct products that are exactly integer ranks in the fraction's shortest
/// decimal representation (for example 0.28 * 25). Nearby non-integer ranks
/// retain their weight instead of being absorbed by an epsilon tolerance.
fn percentile_product(fraction: Expr, count: Expr) -> Expr {
    use datafusion::{
        arrow::array::Float64Array,
        error::DataFusionError,
        logical_expr::{create_udf, ColumnarValue, ScalarUDF, Volatility},
    };
    static UDF: std::sync::OnceLock<ScalarUDF> = std::sync::OnceLock::new();
    UDF.get_or_init(|| {
        create_udf(
            "gql_percentile_product",
            vec![DataType::Float64; 2],
            DataType::Float64,
            Volatility::Immutable,
            std::sync::Arc::new(|args| {
                let arrays = ColumnarValue::values_to_arrays(args)?;
                let fractions = arrays[0]
                    .as_any()
                    .downcast_ref::<Float64Array>()
                    .ok_or_else(|| {
                        DataFusionError::Internal("expected Float64 percentile fraction".into())
                    })?;
                let counts = arrays[1]
                    .as_any()
                    .downcast_ref::<Float64Array>()
                    .ok_or_else(|| {
                        DataFusionError::Internal("expected Float64 percentile count".into())
                    })?;
                let products: Float64Array = fractions
                    .iter()
                    .zip(counts.iter())
                    .map(|(fraction, count)| {
                        fraction
                            .zip(count)
                            .map(|(fraction, count)| percentile_product_value(fraction, count))
                    })
                    .collect();
                let output: datafusion::arrow::array::ArrayRef = std::sync::Arc::new(products);
                if args
                    .iter()
                    .all(|arg| matches!(arg, ColumnarValue::Scalar(_)))
                {
                    Ok(ColumnarValue::Scalar(ScalarValue::try_from_array(
                        &output, 0,
                    )?))
                } else {
                    Ok(ColumnarValue::Array(output))
                }
            }),
        )
    })
    .call(vec![fraction, count])
}

fn percentile_product_value(fraction: f64, count: f64) -> f64 {
    exact_decimal_integer_product(fraction, count).unwrap_or(fraction * count)
}

fn exact_decimal_integer_product(fraction: f64, count: f64) -> Option<f64> {
    if !fraction.is_finite()
        || !(0.0..=1.0).contains(&fraction)
        || !count.is_finite()
        || count < 0.0
        || count.fract() != 0.0
        || count >= u64::MAX as f64
    {
        return None;
    }
    let text = fraction.to_string();
    let (mantissa, exponent) = text.split_once(['e', 'E']).map_or(
        Some((text.as_str(), 0_i32)),
        |(mantissa, exponent)| {
            exponent
                .parse::<i32>()
                .ok()
                .map(|exponent| (mantissa, exponent))
        },
    )?;
    let decimals = mantissa
        .split_once('.')
        .map_or(0, |(_, digits)| digits.len() as i32);
    let scale = decimals - exponent;
    let numerator = mantissa
        .bytes()
        .filter(|byte| *byte != b'.')
        .try_fold(0_u128, |n, byte| {
            n.checked_mul(10)?
                .checked_add(u128::from(byte.checked_sub(b'0')?))
        })?
        .checked_mul(count as u128)?;
    if scale < 0 {
        return Some(numerator.checked_mul(10_u128.checked_pow((-scale) as u32)?)? as f64);
    }
    let denominator = 10_u128.checked_pow(scale as u32)?;
    (numerator % denominator == 0).then(|| (numerator / denominator) as f64)
}

pub(super) fn result(
    mut plan: LogicalPlanBuilder,
    session: &SessionState,
    outer: &mut Bindings,
    body: &ast::QueryBody,
    input_order: Option<Expr>,
) -> Result<LogicalPlanBuilder> {
    let mut aliases = CaseFoldMap::new();
    let mut result_scope = outer.clone();
    let mut projections = Vec::new();
    let mut stored = Vec::new();
    for (i, item) in body.result_clause.items.iter().enumerate() {
        if matches!(item.expr, ast::Expr::Wildcard) {
            if item.alias.is_some() {
                return Err(Error::InvalidQuery("wildcard cannot have an alias".into()));
            }
            let binder = Binder::with_bindings(session, plan.schema(), outer).aggregating();
            for name in &outer.order {
                let expr = binder.bind(&ast::Expr::Identifier(ast::Identifier::new(name)))?;
                if aliases.insert(name.clone(), expr.clone()).is_some() {
                    return Err(Error::InvalidQuery("duplicate result column".into()));
                }
                projections.push((name.clone(), expr, false));
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
            // Result items are simultaneous: an earlier output name is not
            // visible to a later projection.
            let (next, expr, dependent) =
                bind_result_expr(plan, outer, session, &CaseFoldMap::new(), &item.expr, false)?;
            plan = next;
            if let Some(dependent) = dependent {
                stored.push(dependent);
            }
            if aliases.insert(name.clone(), expr.clone()).is_some() {
                return Err(Error::InvalidQuery(format!(
                    "duplicate result column {name}"
                )));
            }
            // Output names shadow input names in grouping, HAVING and ordering.
            // Carry the source's lookup metadata along with its value expression,
            // always resolving against the input scope (aliases are simultaneous).
            result_scope.scalars.remove(&name);
            result_scope.elements.remove(&name);
            result_scope.references.remove(&name);
            let source = match &item.expr {
                ast::Expr::Identifier(source) => Some(&source.value),
                _ => None,
            };
            if let Some(column) = source.and_then(|source| outer.scalars.get(source)) {
                result_scope.scalars.insert(name.clone(), column.clone());
            }
            if let Some(element) = source.and_then(|source| outer.elements.get(source)) {
                result_scope.elements.insert(name.clone(), element.clone());
            }
            if let Some(reference) = source.and_then(|source| outer.references.get(source)) {
                result_scope
                    .references
                    .insert(name.clone(), reference.clone());
            }
            // A missing property binds as a null literal. Remember that the
            // source expression still reads an element, so implicit grouping
            // does not treat it as a query constant.
            projections.push((name, expr, reads_property(&item.expr)));
        }
    }
    if projections.is_empty() {
        return Err(Error::InvalidQuery("result has no bound columns".into()));
    }
    // Projection comparisons allocate names on the input scope. Later clauses
    // resolve against the result scope, so keep its name counter ahead.
    result_scope.advance(outer);
    result_scope.fragment_cache = outer.fragment_cache.clone();
    let mut having = None;
    if let Some(predicate) = &body.having {
        let (next, expr, dependent) =
            bind_result_expr(plan, &mut result_scope, session, &aliases, predicate, true)?;
        plan = next;
        if let Some(dependent) = dependent {
            stored.push(dependent);
        }
        having = Some(expr);
    }
    let mut ordering = Vec::new();
    for item in &body.order_by {
        let (next, expr, dependent) = bind_result_expr(
            plan,
            &mut result_scope,
            session,
            &aliases,
            &item.expr,
            false,
        )?;
        plan = next;
        if let Some(dependent) = dependent {
            stored.push(dependent);
        }
        ordering.push(expr);
    }
    outer.advance(&result_scope);
    let scope = &result_scope;
    let binder = Binder::with_bindings(session, plan.schema(), scope)
        .aggregating()
        .aliases(&aliases);
    let mut all_expressions: Vec<_> = projections.iter().map(|(_, e, _)| e).collect();
    all_expressions.extend(having.iter());
    all_expressions.extend(&ordering);
    let mut aggregates = find_aggregate_exprs(all_expressions);
    let explicit = !body.group_by.is_empty() || body.empty_grouping_set;
    let grouped = explicit || !aggregates.is_empty() || having.is_some();
    let mut groups = Vec::new();
    if explicit {
        for expr in &body.group_by {
            if let ast::Expr::Identifier(name) = expr {
                if let Some(reference) = scope.references.get(&name.value) {
                    let scalar = scope.scalars.get(&name.value).expect("scalar reference");
                    groups.push(Expr::Column(scalar.clone()));
                    groups.extend(
                        plan.schema()
                            .columns()
                            .into_iter()
                            .filter(|c| {
                                c.relation.as_ref().is_some_and(|r| {
                                    reference.parts.iter().any(|p| r.table() == p.alias)
                                        || scope
                                            .elements
                                            .get(&name.value)
                                            .is_some_and(|p| r.table() == p.alias)
                                })
                            })
                            .map(Expr::Column),
                    );
                    continue;
                }
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
            let group = binder.bind(expr)?;
            if !find_aggregate_exprs([&group]).is_empty() {
                return Err(Error::InvalidQuery("aggregate in GROUP BY".into()));
            }
            if !groups.contains(&group) {
                groups.push(group);
            }
        }
    } else if grouped {
        for (_, expr, property) in &projections {
            // Property reads stay row-dependent even when every selected
            // fragment lacks the column and the binder folds them to null.
            // Grouping by that null keeps an empty match from inventing a
            // (NULL, 0) group, while a written NULL or numeric constant does not.
            if find_aggregate_exprs([expr]).is_empty()
                && (!expr.column_refs().is_empty() || *property)
                && !groups.contains(expr)
            {
                groups.push(expr.clone());
            }
        }
    }
    let dependencies = super::native::grouping_columns(
        scope,
        &groups,
        &stored.iter().map(|value| &value.expr).collect::<Vec<_>>(),
    );
    for dependency in dependencies {
        if !groups.contains(&dependency) {
            groups.push(dependency);
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
        .map(|(name, expr, _)| Ok(rewrite(expr)?.alias(name)))
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
    if ordering.is_empty() && !grouped && !body.result_clause.distinct {
        if let Some(order) = input_order {
            plan = plan.sort([order.sort(true, false)])?;
        }
    }
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

fn reads_property(expr: &ast::Expr) -> bool {
    use ast::Expr as E;
    match expr {
        E::Property { .. } => true,
        E::Unary { expr, .. }
        | E::IsNull { expr, .. }
        | E::IsUnknown { expr, .. }
        | E::IsTruth { expr, .. }
        | E::Elements { path: expr } => reads_property(expr),
        E::Binary { left, right, .. } | E::NullIf { left, right } => {
            reads_property(left) || reads_property(right)
        }
        E::Coalesce(args)
        | E::List(args)
        | E::NumericFunction { args, .. }
        | E::Function { args, .. } => args.iter().any(reads_property),
        _ => false,
    }
}

struct StoredComparison {
    expr: Expr,
}

fn bind_result_expr(
    plan: LogicalPlanBuilder,
    scope: &mut Bindings,
    session: &SessionState,
    aliases: &CaseFoldMap<Expr>,
    expr: &ast::Expr,
    predicate: bool,
) -> Result<(LogicalPlanBuilder, Expr, Option<StoredComparison>)> {
    if let ast::Expr::Identifier(name) = expr {
        if let Some(alias) = aliases.get(&name.value) {
            return Ok((plan, alias.clone(), None));
        }
    }
    let (plan, mixed) = super::native::fragment_result_value(plan, scope, session, aliases, expr)?;
    if let Some(mixed) = mixed {
        return Ok((
            plan,
            mixed.expr.clone(),
            Some(StoredComparison { expr: mixed.expr }),
        ));
    }
    let binder = Binder::with_bindings(session, plan.schema(), scope)
        .aggregating()
        .aliases(aliases);
    let bound = if predicate {
        binder.predicate(expr)?
    } else {
        binder.bind(expr)?
    };
    Ok((plan, bound, None))
}

fn rebase(expr: Expr, mapping: &[(Expr, Column)]) -> Result<Expr> {
    // An ungrouped column is an invalid query. Reporting it as a DataFusion
    // planning error would surface internal `__gf_*` names as an execution failure.
    let transformed = expr
        .transform_down(|expr| {
            if let Some((_, column)) = mapping.iter().find(|(source, _)| source == &expr) {
                return Ok(Transformed::yes(Expr::Column(column.clone())));
            }
            if matches!(expr, Expr::Column(_)) {
                return Err(datafusion::error::DataFusionError::Plan(
                    "expression is not grouped or aggregated".into(),
                ));
            }
            Ok(Transformed::no(expr))
        })
        .map_err(|error| match error {
            datafusion::error::DataFusionError::Plan(message)
                if message == "expression is not grouped or aggregated" =>
            {
                Error::InvalidQuery(message)
            }
            other => other.into(),
        })?;
    Ok(transformed.data)
}

#[cfg(test)]
mod tests {
    use super::percentile_product_value;
    #[test]
    fn percentile_products_correct_only_exact_decimal_integer_ranks() {
        assert_eq!(percentile_product_value(0.28, 25.0), 7.0);
        assert_eq!(percentile_product_value(0.56, 25.0), 14.0);
        for (fraction, count) in [
            (1e-16, 1.0),
            (f64::from_bits(1), 1.0),
            (0.5000000000000001, 2.0),
            (0.2800000000000001, 25.0),
            (0.29, 25.0),
            (15.0 / 22.0, 22.0),
        ] {
            assert_eq!(
                percentile_product_value(fraction, count).to_bits(),
                (fraction * count).to_bits()
            );
        }
        assert!(percentile_product_value(f64::NAN, 2.0).is_nan());
    }
}

#[cfg(test)]
mod distinct_deviation_tests {
    use super::deviation_of;
    #[test]
    fn floating_deviation_keeps_large_and_tiny_differences() {
        assert_eq!(
            deviation_of([1e308, -1e308].into_iter(), false),
            Some(1e308)
        );
        assert_eq!(
            deviation_of([1e16, 1e16 + 2.0].into_iter(), false),
            Some(1.0)
        );
        assert_eq!(
            deviation_of([1e-300, -1e-300].into_iter(), false),
            Some(1e-300)
        );
    }
}
