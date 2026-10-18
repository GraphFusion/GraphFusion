//! Checked Arrow arithmetic invoked by DataFusion in both constant folding and execution.
use datafusion::{
    arrow::{
        array::{Array, Float64Array},
        compute::kernels::numeric,
        datatypes::DataType,
    },
    common::ScalarValue,
    error::DataFusionError,
    logical_expr::{create_udf, ColumnarValue, Expr, Operator, Volatility},
};
use std::sync::Arc;

pub(super) fn finite(value: Expr) -> Expr {
    create_udf(
        "gql_finite_float",
        vec![DataType::Float64],
        DataType::Float64,
        Volatility::Immutable,
        Arc::new(|args| {
            let arrays = ColumnarValue::values_to_arrays(args)?;
            let values = arrays[0]
                .as_any()
                .downcast_ref::<Float64Array>()
                .ok_or_else(|| {
                    DataFusionError::Internal("expected Float64 aggregate result".into())
                })?;
            if values.iter().flatten().any(|n| !n.is_finite()) {
                return Err(DataFusionError::Execution(
                    "non-finite aggregate result".into(),
                ));
            }
            Ok(args[0].clone())
        }),
    )
    .call(vec![value])
}

pub(super) fn checked(left: Expr, op: Operator, right: Expr, data_type: DataType) -> Expr {
    let name = format!("gql_checked_{op:?}_{data_type:?}").to_lowercase();
    create_udf(
        &name,
        vec![data_type.clone(), data_type.clone()],
        // The numeric input family is chosen by the GQL binder, not SQL coercion.
        data_type,
        Volatility::Immutable,
        Arc::new(move |args| {
            let arrays = ColumnarValue::values_to_arrays(args)?;
            if op == Operator::Divide {
                if let (Some(left), Some(right)) = (
                    arrays[0].as_any().downcast_ref::<Float64Array>(),
                    arrays[1].as_any().downcast_ref::<Float64Array>(),
                ) {
                    if left
                        .iter()
                        .zip(right.iter())
                        .any(|(a, b)| a.is_some() && b == Some(0.0))
                    {
                        return Err(DataFusionError::Execution("division by zero".into()));
                    }
                }
            }
            let left = &arrays[0].as_ref();
            let right = &arrays[1].as_ref();
            let output = match op {
                Operator::Plus => numeric::add(left, right),
                Operator::Minus => numeric::sub(left, right),
                Operator::Multiply => numeric::mul(left, right),
                Operator::Divide => numeric::div(left, right),
                _ => unreachable!("only arithmetic operators are bound to this UDF"),
            }?;
            if let Some(values) = output.as_any().downcast_ref::<Float64Array>() {
                if values.iter().flatten().any(|n| !n.is_finite()) {
                    return Err(DataFusionError::Execution(
                        "non-finite arithmetic result".into(),
                    ));
                }
            }
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
    .call(vec![left, right])
}

/// `NULLIF` for an Int64/Float64 pair. Equality uses the integer's exact value,
/// and the result keeps the left operand's original type.
pub(super) fn exact_nullif(
    left: Expr,
    right: Expr,
    left_type: &DataType,
    right_type: &DataType,
) -> Option<Expr> {
    let return_type = match (left_type, right_type) {
        (DataType::Int64, DataType::Float64) | (DataType::Float64, DataType::Int64) => {
            left_type.clone()
        }
        _ => return None,
    };
    let integer_on_left = return_type == DataType::Int64;
    let name = if integer_on_left {
        "gql_exact_nullif_int_float"
    } else {
        "gql_exact_nullif_float_int"
    };
    Some(
        create_udf(
            name,
            vec![left_type.clone(), right_type.clone()],
            return_type,
            Volatility::Immutable,
            Arc::new(move |args| {
                use datafusion::arrow::array::ArrayRef;
                let arrays = ColumnarValue::values_to_arrays(args)?;
                let output: ArrayRef = if integer_on_left {
                    let integers = int64_array(&arrays[0])?;
                    let floats = float64_array(&arrays[1])?;
                    Arc::new(nullif_int_float(integers, floats))
                } else {
                    let floats = float64_array(&arrays[0])?;
                    let integers = int64_array(&arrays[1])?;
                    Arc::new(nullif_float_int(floats, integers))
                };
                columnar_result(args, output)
            }),
        )
        .call(vec![left, right]),
    )
}

/// Mixed Int64/Float64 `COALESCE`. The result is Float64, and a selected integer
/// that is not exactly representable is an error instead of a rounded value.
pub(super) fn exact_coalesce(args: Vec<Expr>, types: &[DataType]) -> Option<Expr> {
    if !(types.contains(&DataType::Int64) && types.contains(&DataType::Float64)) {
        return None;
    }
    if types
        .iter()
        .any(|ty| !matches!(ty, DataType::Int64 | DataType::Float64 | DataType::Null))
    {
        return None;
    }
    let input_types: Vec<_> = types
        .iter()
        .map(|ty| {
            if *ty == DataType::Null {
                DataType::Float64
            } else {
                ty.clone()
            }
        })
        .collect();
    let signature: String = input_types
        .iter()
        .map(|ty| if *ty == DataType::Int64 { 'i' } else { 'f' })
        .collect();
    let args = args
        .into_iter()
        .zip(types)
        .map(|(expr, ty)| {
            if *ty == DataType::Null {
                datafusion::logical_expr::cast(expr, DataType::Float64)
            } else {
                expr
            }
        })
        .collect();
    Some(
        create_udf(
            &format!("gql_exact_coalesce_{signature}"),
            input_types,
            DataType::Float64,
            Volatility::Immutable,
            Arc::new(|args| {
                use datafusion::arrow::array::{Array, ArrayRef, Float64Array};
                let arrays = ColumnarValue::values_to_arrays(args)?;
                let len = arrays.first().map(|array| array.len()).unwrap_or(0);
                let mut values = Vec::with_capacity(len);
                for row in 0..len {
                    let mut selected = None;
                    for array in &arrays {
                        if array.is_null(row) {
                            continue;
                        }
                        selected = Some(selected_f64(array, row)?);
                        break;
                    }
                    values.push(selected);
                }
                columnar_result(args, Arc::new(Float64Array::from(values)) as ArrayRef)
            }),
        )
        .call(args),
    )
}

fn nullif_int_float(
    integers: &datafusion::arrow::array::Int64Array,
    floats: &datafusion::arrow::array::Float64Array,
) -> datafusion::arrow::array::Int64Array {
    (0..integers.len())
        .map(|row| {
            if integers.is_null(row) {
                return None;
            }
            let value = integers.value(row);
            if !floats.is_null(row) && i64_eq_f64(value, floats.value(row)) {
                None
            } else {
                Some(value)
            }
        })
        .collect()
}

fn nullif_float_int(
    floats: &datafusion::arrow::array::Float64Array,
    integers: &datafusion::arrow::array::Int64Array,
) -> datafusion::arrow::array::Float64Array {
    (0..floats.len())
        .map(|row| {
            if floats.is_null(row) {
                return None;
            }
            let value = floats.value(row);
            if !integers.is_null(row) && i64_eq_f64(integers.value(row), value) {
                None
            } else {
                Some(value)
            }
        })
        .collect()
}

fn selected_f64(
    array: &datafusion::arrow::array::ArrayRef,
    row: usize,
) -> Result<f64, DataFusionError> {
    use datafusion::arrow::array::{Float64Array, Int64Array};
    if let Some(values) = array.as_any().downcast_ref::<Int64Array>() {
        let integer = values.value(row);
        exact_i64_f64(integer).ok_or_else(|| {
            DataFusionError::Execution(format!(
                "integer {integer} is not exactly representable as float"
            ))
        })
    } else if let Some(values) = array.as_any().downcast_ref::<Float64Array>() {
        Ok(values.value(row))
    } else {
        Err(DataFusionError::Internal(
            "expected an integer or float coalesce argument".into(),
        ))
    }
}

fn int64_array(
    array: &datafusion::arrow::array::ArrayRef,
) -> Result<&datafusion::arrow::array::Int64Array, DataFusionError> {
    array
        .as_any()
        .downcast_ref()
        .ok_or_else(|| DataFusionError::Internal("expected Int64 NULLIF operand".into()))
}

fn float64_array(
    array: &datafusion::arrow::array::ArrayRef,
) -> Result<&datafusion::arrow::array::Float64Array, DataFusionError> {
    array
        .as_any()
        .downcast_ref()
        .ok_or_else(|| DataFusionError::Internal("expected Float64 NULLIF operand".into()))
}

fn columnar_result(
    args: &[ColumnarValue],
    output: datafusion::arrow::array::ArrayRef,
) -> Result<ColumnarValue, DataFusionError> {
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
}

/// `Some` only when `integer` is an exact `f64`. `i64::MIN` is exact; `i64::MAX` is not.
fn exact_i64_f64(integer: i64) -> Option<f64> {
    let as_float = integer as f64;
    // Values that round to 2^63 saturate when cast back to i64, so a round trip
    // alone would accept integers that float cannot store.
    if as_float < 9223372036854775808.0 && as_float as i64 == integer {
        Some(as_float)
    } else {
        None
    }
}

fn i64_eq_f64(integer: i64, float: f64) -> bool {
    exact_i64_f64(integer) == Some(float)
}

#[cfg(test)]
mod tests {
    use super::*;
    use datafusion::{
        arrow::{
            array::{ArrayRef, Int64Array},
            record_batch::RecordBatch,
        },
        logical_expr::col,
        prelude::SessionContext,
    };

    #[tokio::test]
    async fn checked_arithmetic_executes_on_arrow_columns_with_nulls() {
        let batch = RecordBatch::try_from_iter([
            (
                "a",
                Arc::new(Int64Array::from(vec![Some(10), None, Some(-4)])) as ArrayRef,
            ),
            (
                "b",
                Arc::new(Int64Array::from(vec![Some(2), Some(0), Some(2)])) as ArrayRef,
            ),
        ])
        .unwrap();
        let data = SessionContext::new().read_batch(batch).unwrap();
        let result = data
            .select(vec![checked(
                col("a"),
                Operator::Divide,
                col("b"),
                DataType::Int64,
            )])
            .unwrap()
            .collect()
            .await
            .unwrap();
        let values = result[0]
            .column(0)
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap();
        assert_eq!(
            values.iter().collect::<Vec<_>>(),
            vec![Some(5), None, Some(-2)]
        );

        let batch = RecordBatch::try_from_iter([
            (
                "a",
                Arc::new(Int64Array::from(vec![1, i64::MAX])) as ArrayRef,
            ),
            ("b", Arc::new(Int64Array::from(vec![1, 1])) as ArrayRef),
        ])
        .unwrap();
        let data = SessionContext::new().read_batch(batch).unwrap();
        assert!(data
            .select(vec![checked(
                col("a"),
                Operator::Plus,
                col("b"),
                DataType::Int64
            )])
            .unwrap()
            .collect()
            .await
            .is_err());
    }

    #[tokio::test]
    async fn float_zero_division_checks_only_nonnull_pairs() {
        let batch = RecordBatch::try_from_iter([
            (
                "a",
                Arc::new(Float64Array::from(vec![None, Some(4.0)])) as ArrayRef,
            ),
            (
                "b",
                Arc::new(Float64Array::from(vec![Some(0.0), Some(2.0)])) as ArrayRef,
            ),
        ])
        .unwrap();
        let data = SessionContext::new().read_batch(batch).unwrap();
        let result = data
            .select(vec![checked(
                col("a"),
                Operator::Divide,
                col("b"),
                DataType::Float64,
            )])
            .unwrap()
            .collect()
            .await
            .unwrap();
        let values = result[0]
            .column(0)
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        assert_eq!(values.iter().collect::<Vec<_>>(), vec![None, Some(2.0)]);
    }
}
