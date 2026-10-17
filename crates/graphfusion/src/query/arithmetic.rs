//! Checked Arrow arithmetic invoked by DataFusion in both constant folding and execution.
use datafusion::{
    arrow::{array::Float64Array, compute::kernels::numeric, datatypes::DataType},
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

/// Divide an exact decimal sum by an integer count and round to Float64.
pub(super) fn exact_ratio(sum: Expr, sum_type: DataType, count: Expr) -> Expr {
    let scale = match &sum_type {
        DataType::Decimal128(_, scale) => *scale,
        _ => 0,
    };
    let name = format!("gql_exact_ratio_{sum_type:?}").to_lowercase();
    create_udf(
        &name,
        vec![sum_type, DataType::Int64],
        DataType::Float64,
        Volatility::Immutable,
        Arc::new(move |args| {
            use datafusion::arrow::array::Array;
            let arrays = ColumnarValue::values_to_arrays(args)?;
            let sums = arrays[0]
                .as_any()
                .downcast_ref::<datafusion::arrow::array::Decimal128Array>()
                .ok_or_else(|| DataFusionError::Internal("expected decimal sum".into()))?;
            let counts = arrays[1]
                .as_any()
                .downcast_ref::<datafusion::arrow::array::Int64Array>()
                .ok_or_else(|| DataFusionError::Internal("expected integer count".into()))?;
            let mut builder = datafusion::arrow::array::Float64Builder::with_capacity(sums.len());
            for row in 0..sums.len() {
                if sums.is_null(row) || counts.is_null(row) || counts.value(row) == 0 {
                    builder.append_null();
                    continue;
                }
                let Some(denom) = i128::from(counts.value(row)).checked_mul(scale_factor(scale))
                else {
                    return Err(DataFusionError::Execution(
                        "integer average exceeds decimal range".into(),
                    ));
                };
                builder.append_value(ratio_to_f64(sums.value(row), denom));
            }
            let output: datafusion::arrow::array::ArrayRef = Arc::new(builder.finish());
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
    .call(vec![sum, count])
}

fn scale_factor(scale: i8) -> i128 {
    if scale <= 0 {
        1
    } else {
        10i128.saturating_pow(u32::from(scale as u8))
    }
}

/// Correctly rounded `numer / denom`. Forty truncated decimal digits sit well
/// below a Float64 midpoint, and Rust parses decimal text with that rounding.
fn ratio_to_f64(numer: i128, denom: i128) -> f64 {
    if denom == 0 {
        return f64::NAN;
    }
    let negative = (numer < 0) ^ (denom < 0);
    let numer = numer.unsigned_abs();
    let denom = denom.unsigned_abs();
    let mut digits = (numer / denom).to_string();
    digits.push('.');
    let mut remainder = numer % denom;
    for _ in 0..40 {
        remainder *= 10;
        let digit = remainder / denom;
        digits.push((b'0' + u8::try_from(digit).unwrap_or(0)) as char);
        remainder %= denom;
    }
    let value = digits.parse::<f64>().unwrap_or(f64::NAN);
    if negative {
        -value
    } else {
        value
    }
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

    #[test]
    fn ratio_rounds_the_exact_quotient() {
        assert_eq!(ratio_to_f64(9007199254740994, 2), 4503599627370497.0);
        assert_eq!(ratio_to_f64(4, 3), 4.0 / 3.0);
        assert_eq!(ratio_to_f64(-3, 2), -1.5);
        assert_eq!(ratio_to_f64(0, 5), 0.0);
        assert_eq!(ratio_to_f64(9007199254741003, 2), 4503599627370502.0);
        assert_eq!(ratio_to_f64(-9007199254741003, 2), -4503599627370502.0);
    }
}
