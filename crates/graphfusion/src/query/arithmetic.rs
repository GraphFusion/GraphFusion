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

/// Compare an integer with a float by the integer's exact value.
///
/// Casting either side to Float64 rounds integers past 2^53, so values such as
/// `9007199254740993` and `9007199254740992.0` would compare equal. Returns
/// `None` when the operands are not an Int64/Float64 pair.
pub(super) fn exact_numeric_compare(
    left: Expr,
    op: Operator,
    right: Expr,
    left_type: &DataType,
    right_type: &DataType,
) -> Option<Expr> {
    let (integer, float, op) = match (left_type, right_type) {
        (DataType::Int64, DataType::Float64) => (left, right, op),
        (DataType::Float64, DataType::Int64) => (right, left, flip_compare(op)),
        _ => return None,
    };
    let name = format!("gql_exact_int_float_{op:?}").to_lowercase();
    Some(
        create_udf(
            &name,
            vec![DataType::Int64, DataType::Float64],
            DataType::Boolean,
            Volatility::Immutable,
            Arc::new(move |args| {
                use datafusion::arrow::array::{BooleanArray, Float64Array, Int64Array};
                let arrays = ColumnarValue::values_to_arrays(args)?;
                let integers =
                    arrays[0]
                        .as_any()
                        .downcast_ref::<Int64Array>()
                        .ok_or_else(|| {
                            DataFusionError::Internal("expected Int64 comparison operand".into())
                        })?;
                let floats = arrays[1]
                    .as_any()
                    .downcast_ref::<Float64Array>()
                    .ok_or_else(|| {
                        DataFusionError::Internal("expected Float64 comparison operand".into())
                    })?;
                let compared: BooleanArray = (0..integers.len())
                    .map(|row| {
                        if integers.is_null(row) || floats.is_null(row) {
                            None
                        } else {
                            Some(apply_order(
                                op,
                                cmp_i64_f64(integers.value(row), floats.value(row)),
                            ))
                        }
                    })
                    .collect();
                if args
                    .iter()
                    .all(|arg| matches!(arg, ColumnarValue::Scalar(_)))
                {
                    Ok(ColumnarValue::Scalar(ScalarValue::try_from_array(
                        &compared, 0,
                    )?))
                } else {
                    Ok(ColumnarValue::Array(std::sync::Arc::new(compared)))
                }
            }),
        )
        .call(vec![integer, float]),
    )
}

fn flip_compare(op: Operator) -> Operator {
    match op {
        Operator::Lt => Operator::Gt,
        Operator::Gt => Operator::Lt,
        Operator::LtEq => Operator::GtEq,
        Operator::GtEq => Operator::LtEq,
        other => other,
    }
}

/// `None` means the float is NaN. Every other finite, infinite, or out-of-range
/// float has a total order against the integer.
fn cmp_i64_f64(integer: i64, float: f64) -> Option<std::cmp::Ordering> {
    use std::cmp::Ordering;
    if float.is_nan() {
        return None;
    }
    if float == f64::INFINITY {
        return Some(Ordering::Less);
    }
    if float == f64::NEG_INFINITY {
        return Some(Ordering::Greater);
    }
    let minimum = i64::MIN as f64;
    // 2^63 is the first magnitude that no longer fits in i64.
    let exclusive_max = 9223372036854775808.0;
    if float >= exclusive_max {
        return Some(Ordering::Less);
    }
    if float < minimum {
        return Some(Ordering::Greater);
    }
    let truncated = float.trunc();
    let truncated_int = if truncated == minimum {
        i64::MIN
    } else {
        let value = truncated as i64;
        debug_assert_eq!(value as f64, truncated);
        value
    };
    Some(match integer.cmp(&truncated_int) {
        Ordering::Equal if float == truncated => Ordering::Equal,
        Ordering::Equal if float > truncated => Ordering::Less,
        Ordering::Equal => Ordering::Greater,
        other => other,
    })
}

fn apply_order(op: Operator, order: Option<std::cmp::Ordering>) -> bool {
    use std::cmp::Ordering::{Equal, Greater, Less};
    match op {
        // NaN equals nothing, so `<>` is true and the ordered comparisons are false.
        Operator::Eq => order == Some(Equal),
        Operator::NotEq => order != Some(Equal),
        Operator::Lt => order == Some(Less),
        Operator::LtEq => matches!(order, Some(Less | Equal)),
        Operator::Gt => order == Some(Greater),
        Operator::GtEq => matches!(order, Some(Greater | Equal)),
        _ => false,
    }
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
