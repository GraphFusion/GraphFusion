//! Checked Arrow arithmetic invoked by DataFusion in both constant folding and execution.
//! Integer percentile weights are exact dyadic rationals and round once to Float64.
use datafusion::{
    arrow::{
        array::{Float64Array, Float64Builder, Int64Array},
        compute::kernels::numeric,
        datatypes::DataType,
    },
    common::ScalarValue,
    error::DataFusionError,
    logical_expr::{create_udf, ColumnarValue, Expr, Operator, Volatility},
};
use std::{cmp::Ordering, sync::Arc};

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
            };
            // Integer kernels report these as Arrow failures. Surface the same
            // execution errors as the float checks above, including MIN / -1.
            let output = match output {
                Ok(output) => output,
                Err(datafusion::arrow::error::ArrowError::DivideByZero) => {
                    return Err(DataFusionError::Execution("division by zero".into()));
                }
                Err(datafusion::arrow::error::ArrowError::ArithmeticOverflow(_)) => {
                    return Err(DataFusionError::Execution("integer overflow".into()));
                }
                Err(error) => return Err(DataFusionError::from(error)),
            };
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

/// `left + (right - left) * weight`, with integer endpoints kept exact.
///
/// `weight` is the Float64 fractional position already computed for the
/// percentile. It is read as a dyadic rational, applied to the exact integer
/// span, and rounded once. Casting both endpoints first drops bits above 2^53.
pub(super) fn exact_lerp(left: Expr, right: Expr, weight: Expr) -> Expr {
    create_udf(
        "gql_exact_lerp",
        vec![DataType::Int64, DataType::Int64, DataType::Float64],
        DataType::Float64,
        Volatility::Immutable,
        Arc::new(|args| {
            use datafusion::arrow::array::Array;
            let arrays = ColumnarValue::values_to_arrays(args)?;
            let endpoints = [
                arrays[0]
                    .as_any()
                    .downcast_ref::<Int64Array>()
                    .ok_or_else(|| {
                        DataFusionError::Internal("expected Int64 percentile endpoint".into())
                    })?,
                arrays[1]
                    .as_any()
                    .downcast_ref::<Int64Array>()
                    .ok_or_else(|| {
                        DataFusionError::Internal("expected Int64 percentile endpoint".into())
                    })?,
            ];
            let weights = arrays[2]
                .as_any()
                .downcast_ref::<Float64Array>()
                .ok_or_else(|| {
                    DataFusionError::Internal("expected Float64 percentile weight".into())
                })?;
            if endpoints[1].len() != endpoints[0].len() || weights.len() != endpoints[0].len() {
                return Err(DataFusionError::Internal(
                    "percentile endpoints have different lengths".into(),
                ));
            }
            let mut output = Float64Builder::with_capacity(endpoints[0].len());
            for i in 0..endpoints[0].len() {
                if endpoints[0].is_null(i) || endpoints[1].is_null(i) || weights.is_null(i) {
                    output.append_null();
                } else {
                    output.append_value(lerp_i64(
                        endpoints[0].value(i),
                        endpoints[1].value(i),
                        weights.value(i),
                    ));
                }
            }
            Ok(ColumnarValue::Array(Arc::new(output.finish())))
        }),
    )
    .call(vec![left, right, weight])
}

fn lerp_i64(left: i64, right: i64, weight: f64) -> f64 {
    if !weight.is_finite() {
        return f64::NAN;
    }
    if weight <= 0.0 {
        return left as f64;
    }
    if weight >= 1.0 {
        return right as f64;
    }
    let (mantissa, power) = f64_dyadic(weight);
    let Ok(denominator_pow) = u32::try_from(-power) else {
        return float_lerp(left, right, weight);
    };
    let diff = i128::from(right) - i128::from(left);
    let Some(product) = diff.checked_mul(i128::from(mantissa)) else {
        return float_lerp(left, right, weight);
    };
    let scaled = Magnitude::from_u128(u128::from(left.unsigned_abs())).shift_left(denominator_pow);
    let (negative, magnitude) = signed_sum(
        left < 0,
        scaled,
        product < 0,
        Magnitude::from_u128(product.unsigned_abs()),
    );
    f64_from_dyadic(negative, &magnitude, denominator_pow)
}

fn float_lerp(left: i64, right: i64, weight: f64) -> f64 {
    left as f64 + (right as f64 - left as f64) * weight
}

/// `weight = mantissa * 2^power`, with the implicit bit folded into `power`.
fn f64_dyadic(weight: f64) -> (u64, i32) {
    let bits = weight.to_bits();
    let exponent = ((bits >> 52) & 0x7ff) as i32;
    let fraction = bits & ((1u64 << 52) - 1);
    if exponent == 0 {
        (fraction, -1074)
    } else {
        ((1u64 << 52) | fraction, exponent - 1023 - 52)
    }
}

fn f64_from_dyadic(negative: bool, magnitude: &Magnitude, denominator_pow: u32) -> f64 {
    let Some(highest) = magnitude.highest_bit() else {
        return 0.0;
    };
    let exponent = i64::from(highest) - i64::from(denominator_pow);
    if exponent > 1023 {
        return if negative {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        };
    }
    if exponent >= -1022 {
        normal_from_dyadic(negative, magnitude, highest, exponent)
    } else {
        subnormal_from_dyadic(negative, magnitude, denominator_pow)
    }
}

fn normal_from_dyadic(
    negative: bool,
    magnitude: &Magnitude,
    highest: u32,
    mut exponent: i64,
) -> f64 {
    let mut mantissa = 0u64;
    for offset in 0..52u32 {
        let index = i64::from(highest) - 1 - i64::from(offset);
        if index >= 0 && magnitude.bit(index as u32) {
            mantissa |= 1u64 << (51 - offset);
        }
    }
    let (round, sticky) = magnitude.round_info(i64::from(highest) - 53);
    if round_ties_to_even(mantissa, round, sticky) {
        mantissa += 1;
        if mantissa >> 52 != 0 {
            mantissa = 0;
            exponent += 1;
        }
    }
    pack_normal(negative, exponent, mantissa)
}

fn subnormal_from_dyadic(negative: bool, magnitude: &Magnitude, denominator_pow: u32) -> f64 {
    // Subnormals are integer multiples of 2^-1074. `shift` is exact when the
    // weight itself is a Float64, whose shortest power is 2^-1074.
    let shift = i64::from(denominator_pow) - 1074;
    let (mut mantissa, round, sticky) = if shift <= 0 {
        let shifted = magnitude.shift_left(u32::try_from(-shift).unwrap_or(0));
        (shifted.0.first().copied().unwrap_or(0), false, false)
    } else {
        let shift = u32::try_from(shift).unwrap_or(u32::MAX);
        let kept = magnitude.low_bits_from(shift);
        let (round, sticky) = magnitude.round_info(i64::from(shift) - 1);
        (kept, round, sticky)
    };
    if round_ties_to_even(mantissa, round, sticky) {
        mantissa = mantissa.saturating_add(1);
    }
    if mantissa == 0 {
        return if negative { -0.0 } else { 0.0 };
    }
    if mantissa >> 52 != 0 {
        return pack_normal(negative, -1022, 0);
    }
    f64::from_bits((u64::from(negative) << 63) | mantissa)
}

fn round_ties_to_even(mantissa: u64, round: bool, sticky: bool) -> bool {
    round && (sticky || mantissa & 1 == 1)
}

fn pack_normal(negative: bool, exponent: i64, mantissa: u64) -> f64 {
    if exponent > 1023 {
        return if negative {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        };
    }
    let field = (exponent + 1023) as u64;
    f64::from_bits((u64::from(negative) << 63) | (field << 52) | (mantissa & ((1u64 << 52) - 1)))
}

fn signed_sum(
    left_negative: bool,
    left: Magnitude,
    right_negative: bool,
    right: Magnitude,
) -> (bool, Magnitude) {
    if left_negative == right_negative {
        let magnitude = left.add(right);
        return (left_negative && !magnitude.is_zero(), magnitude);
    }
    match left.cmp_magnitude(&right) {
        Ordering::Greater => (left_negative, left.sub(right)),
        Ordering::Less => (right_negative, right.sub(left)),
        Ordering::Equal => (false, Magnitude::zero()),
    }
}

/// Little-endian unsigned magnitude. Empty means zero.
struct Magnitude(Vec<u64>);

impl Magnitude {
    fn zero() -> Self {
        Self(Vec::new())
    }

    fn is_zero(&self) -> bool {
        self.0.is_empty()
    }

    fn from_u128(value: u128) -> Self {
        let low = value as u64;
        let high = (value >> 64) as u64;
        if high != 0 {
            Self(vec![low, high])
        } else if low != 0 {
            Self(vec![low])
        } else {
            Self::zero()
        }
    }

    fn shift_left(&self, bits: u32) -> Self {
        if self.is_zero() || bits == 0 {
            return Self(self.0.clone());
        }
        let skip = (bits / 64) as usize;
        let remainder = bits % 64;
        let mut limbs = vec![0; skip];
        if remainder == 0 {
            limbs.extend_from_slice(&self.0);
            return Self(limbs);
        }
        let mut carry = 0u64;
        for &limb in &self.0 {
            limbs.push((limb << remainder) | carry);
            carry = limb >> (64 - remainder);
        }
        if carry != 0 {
            limbs.push(carry);
        }
        Self(limbs)
    }

    fn add(self, other: Self) -> Self {
        let width = self.0.len().max(other.0.len());
        let mut limbs = Vec::with_capacity(width + 1);
        let mut carry = 0u64;
        for i in 0..width {
            let left = self.0.get(i).copied().unwrap_or(0);
            let right = other.0.get(i).copied().unwrap_or(0);
            let (sum, overflow) = left.overflowing_add(right);
            let (sum, overflow_carry) = sum.overflowing_add(carry);
            limbs.push(sum);
            carry = u64::from(overflow || overflow_carry);
        }
        if carry != 0 {
            limbs.push(carry);
        }
        Self(limbs)
    }

    fn sub(self, other: Self) -> Self {
        let mut limbs = Vec::with_capacity(self.0.len());
        let mut borrow = 0u64;
        for i in 0..self.0.len() {
            let left = self.0[i];
            let right = other.0.get(i).copied().unwrap_or(0);
            let (difference, overflow) = left.overflowing_sub(right);
            let (difference, overflow_borrow) = difference.overflowing_sub(borrow);
            limbs.push(difference);
            borrow = u64::from(overflow || overflow_borrow);
        }
        while limbs.last() == Some(&0) {
            limbs.pop();
        }
        Self(limbs)
    }

    fn cmp_magnitude(&self, other: &Self) -> Ordering {
        match self.0.len().cmp(&other.0.len()) {
            Ordering::Equal => self.0.iter().rev().cmp(other.0.iter().rev()),
            order => order,
        }
    }

    fn bit(&self, index: u32) -> bool {
        let limb = (index / 64) as usize;
        self.0
            .get(limb)
            .is_some_and(|value| value & (1u64 << (index % 64)) != 0)
    }

    fn any_below(&self, index: u32) -> bool {
        let limbs = (index / 64) as usize;
        let remainder = index % 64;
        if self.0.iter().take(limbs).any(|limb| *limb != 0) {
            return true;
        }
        remainder > 0
            && self
                .0
                .get(limbs)
                .is_some_and(|limb| limb & ((1u64 << remainder) - 1) != 0)
    }

    fn highest_bit(&self) -> Option<u32> {
        let (index, limb) = self.0.iter().enumerate().rfind(|(_, limb)| **limb != 0)?;
        Some(index as u32 * 64 + 63 - limb.leading_zeros())
    }

    fn round_info(&self, index: i64) -> (bool, bool) {
        let Ok(index) = u32::try_from(index) else {
            return (false, false);
        };
        (self.bit(index), index > 0 && self.any_below(index))
    }

    fn low_bits_from(&self, shift: u32) -> u64 {
        if self
            .highest_bit()
            .is_some_and(|bit| bit >= shift.saturating_add(52))
        {
            return 1u64 << 52;
        }
        let mut value = 0u64;
        for offset in 0..52u32 {
            let Some(index) = shift.checked_add(offset) else {
                break;
            };
            if self.bit(index) {
                value |= 1u64 << offset;
            }
        }
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use datafusion::{
        arrow::{
            array::{ArrayRef, Float64Array, Int64Array},
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
        let overflow = data
            .select(vec![checked(
                col("a"),
                Operator::Plus,
                col("b"),
                DataType::Int64,
            )])
            .unwrap()
            .collect()
            .await
            .unwrap_err();
        assert!(
            overflow.to_string().contains("integer overflow"),
            "{overflow}"
        );

        let batch = RecordBatch::try_from_iter([
            ("a", Arc::new(Int64Array::from(vec![1, 4])) as ArrayRef),
            ("b", Arc::new(Int64Array::from(vec![0, 2])) as ArrayRef),
        ])
        .unwrap();
        let data = SessionContext::new().read_batch(batch).unwrap();
        let division = data
            .select(vec![checked(
                col("a"),
                Operator::Divide,
                col("b"),
                DataType::Int64,
            )])
            .unwrap()
            .collect()
            .await
            .unwrap_err();
        assert!(
            division.to_string().contains("division by zero"),
            "{division}"
        );
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
    fn lerp_rounds_the_exact_integer_blend() {
        for weight in [
            0.0,
            0.5,
            0.25,
            0.1,
            0.75,
            1.0,
            1e-10,
            1e-320,
            f64::from_bits(1),
            f64::from_bits(1.0f64.to_bits() - 1),
        ] {
            assert_eq!(
                lerp_i64(0, 1, weight).to_bits(),
                weight.to_bits(),
                "{weight}"
            );
        }
        for (value, weight) in [
            (9007199254740993, 0.3),
            (9007199254740994, 0.5),
            (i64::MIN, 0.25),
            (i64::MAX, 0.75),
        ] {
            assert_eq!(
                lerp_i64(value, value, weight).to_bits(),
                (value as f64).to_bits(),
                "{value}"
            );
        }
        assert_eq!(lerp_i64(9007199254740993, 1, 0.5), 4503599627370497.0);
        assert_eq!(
            lerp_i64(9007199254740993, 9007199254740995, 0.5),
            9007199254740994.0
        );
        assert_eq!(lerp_i64(-9007199254740993, -1, 0.5), -4503599627370497.0);
        assert_eq!(lerp_i64(i64::MAX, i64::MIN, 0.5), -0.5);
        assert_eq!(lerp_i64(0, 9007199254741003, 0.5), 4503599627370502.0);
        assert_eq!(lerp_i64(0, -9007199254741003, 0.5), -4503599627370502.0);
        assert_eq!(lerp_i64(1, 3, 0.5), 2.0);
        assert_eq!(lerp_i64(1, 2, 0.5), 1.5);
        assert_eq!(lerp_i64(1, 4, 0.75), 3.25);
        assert_eq!(lerp_i64(-5, -1, 0.5), -3.0);
        assert_eq!(lerp_i64(-1, 5, 0.5), 2.0);
        assert_eq!(lerp_i64(-3, 7, 0.25), -0.5);
        assert_eq!(lerp_i64(1, 2, 0.1).to_bits(), 1.1f64.to_bits());
        assert_eq!(lerp_i64(-5, 9, 0.1).to_bits(), (-3.6f64).to_bits());
        assert_eq!(lerp_i64(0, 7, 0.1).to_bits(), (7.0_f64 * 0.1).to_bits());
        assert!(lerp_i64(1, 2, f64::NAN).is_nan());
    }

    #[tokio::test]
    async fn exact_lerp_preserves_null_endpoints() {
        let batch = RecordBatch::try_from_iter([
            (
                "a",
                Arc::new(Int64Array::from(vec![Some(1), None, Some(4)])) as ArrayRef,
            ),
            (
                "b",
                Arc::new(Int64Array::from(vec![Some(3), Some(5), None])) as ArrayRef,
            ),
            (
                "w",
                Arc::new(Float64Array::from(vec![Some(0.5), Some(0.5), Some(0.25)])) as ArrayRef,
            ),
        ])
        .unwrap();
        let data = SessionContext::new().read_batch(batch).unwrap();
        let result = data
            .select(vec![exact_lerp(col("a"), col("b"), col("w"))])
            .unwrap()
            .collect()
            .await
            .unwrap();
        let values = result[0]
            .column(0)
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        assert_eq!(
            values.iter().collect::<Vec<_>>(),
            vec![Some(2.0), None, None]
        );
    }
}
