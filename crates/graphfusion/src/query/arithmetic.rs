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
}
