//! Checked Arrow arithmetic invoked by DataFusion in both constant folding and execution.
//! Integer percentile weights are exact dyadic rationals and round once to Float64.
use datafusion::{
    arrow::{
        array::{Array, Float64Array, Float64Builder, Int64Array},
        compute::kernels::numeric,
        datatypes::DataType,
    },
    common::ScalarValue,
    error::DataFusionError,
    logical_expr::{create_udf, ColumnarValue, Expr, Operator, ScalarUDF, Volatility},
};
use std::{cmp::Ordering, sync::Arc};

pub(super) fn finite(value: Expr) -> Expr {
    // One function value for every finite check. A fresh closure would not
    // compare equal, so DISTINCT could not recognize the same float aggregate
    // in ORDER BY.
    finite_udf().call(vec![value])
}

fn finite_udf() -> ScalarUDF {
    use std::sync::OnceLock;
    static UDF: OnceLock<ScalarUDF> = OnceLock::new();
    UDF.get_or_init(finite_udf_new).clone()
}

fn finite_udf_new() -> ScalarUDF {
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
}

/// Divide an exact decimal sum by an integer count and round to Float64.
pub(super) fn exact_ratio(sum: Expr, sum_type: DataType, count: Expr) -> Expr {
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<std::collections::HashMap<DataType, ScalarUDF>>> = OnceLock::new();
    let mut cache = CACHE
        .get_or_init(|| Mutex::new(std::collections::HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    cache
        .entry(sum_type.clone())
        .or_insert_with(|| exact_ratio_udf(sum_type))
        .clone()
        .call(vec![sum, count])
}

fn exact_ratio_udf(sum_type: DataType) -> ScalarUDF {
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
}

fn scale_factor(scale: i8) -> i128 {
    if scale <= 0 {
        1
    } else {
        10i128.saturating_pow(u32::from(scale as u8))
    }
}

/// Round the exact rational once, using a binary guard bit and remainder.
fn ratio_to_f64(numer: i128, denom: i128) -> f64 {
    if denom == 0 {
        return f64::NAN;
    }
    let negative = (numer < 0) ^ (denom < 0);
    let numer = numer.unsigned_abs();
    let denom = denom.unsigned_abs();
    if numer == 0 {
        return if negative { -0.0 } else { 0.0 };
    }
    let whole = numer / denom;
    let mut remainder = numer % denom;
    // A remainder is below |denom| <= 2^127, so doubling cannot overflow u128.
    let next_bit = |remainder: &mut u128| {
        *remainder *= 2;
        let bit = *remainder >= denom;
        if bit {
            *remainder -= denom;
        }
        u64::from(bit)
    };
    let mut exponent;
    let mut mantissa;
    let round;
    let sticky;
    if whole != 0 {
        exponent = i64::from(127 - whole.leading_zeros());
        if exponent > 52 {
            let shift = (exponent - 52) as u32;
            mantissa = (whole >> shift) as u64;
            round = (whole >> (shift - 1)) & 1 != 0;
            sticky = whole & ((1u128 << (shift - 1)) - 1) != 0 || remainder != 0;
        } else {
            mantissa = whole as u64;
            for _ in 0..52 - exponent {
                mantissa = (mantissa << 1) | next_bit(&mut remainder);
            }
            round = next_bit(&mut remainder) != 0;
            sticky = remainder != 0;
        }
    } else {
        exponent = -1;
        while next_bit(&mut remainder) == 0 {
            exponent -= 1;
        }
        mantissa = 1;
        for _ in 0..52 {
            mantissa = (mantissa << 1) | next_bit(&mut remainder);
        }
        round = next_bit(&mut remainder) != 0;
        sticky = remainder != 0;
    }
    if round_ties_to_even(mantissa, round, sticky) {
        mantissa += 1;
        if mantissa == 1u64 << 53 {
            mantissa >>= 1;
            exponent += 1;
        }
    }
    pack_normal(negative, exponent, mantissa)
}

pub(super) fn checked(left: Expr, op: Operator, right: Expr, data_type: DataType) -> Expr {
    // One function value per operator and type. A fresh closure would not compare
    // equal, so DISTINCT and GROUP BY could not recognize the same arithmetic
    // expression in ORDER BY.
    checked_udf(op, &data_type).call(vec![left, right])
}

fn checked_udf(op: Operator, data_type: &DataType) -> ScalarUDF {
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<std::collections::HashMap<(u8, DataType), ScalarUDF>>> =
        OnceLock::new();
    let key = (
        match op {
            Operator::Plus => 0,
            Operator::Minus => 1,
            Operator::Multiply => 2,
            Operator::Divide => 3,
            _ => unreachable!("only arithmetic operators are bound to this UDF"),
        },
        data_type.clone(),
    );
    let cache = CACHE.get_or_init(|| Mutex::new(std::collections::HashMap::new()));
    let mut cache = cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    cache
        .entry(key)
        .or_insert_with(|| checked_udf_new(op, data_type.clone()))
        .clone()
}

fn checked_udf_new(op: Operator, data_type: DataType) -> ScalarUDF {
    let name = format!("gql_checked_{op:?}_{data_type:?}").to_lowercase();
    create_udf(
        &name,
        vec![data_type.clone(), data_type.clone()],
        // The numeric input family is chosen by the GQL binder, not SQL coercion.
        data_type,
        Volatility::Immutable,
        Arc::new(move |args| {
            let arrays = ColumnarValue::values_to_arrays(args)?;
            columnar_result(args, checked_arrays(op, &arrays)?)
        }),
    )
}

fn checked_arrays(
    op: Operator,
    arrays: &[datafusion::arrow::array::ArrayRef],
) -> Result<datafusion::arrow::array::ArrayRef, DataFusionError> {
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
    Ok(output)
}

/// Check the pair's null mask before converting an integer to Float64. An
/// inexact integer is an error only on rows that perform mixed arithmetic.
pub(super) fn checked_mixed(left: Expr, op: Operator, right: Expr, integer_on_left: bool) -> Expr {
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<std::collections::HashMap<(u8, bool), ScalarUDF>>> =
        OnceLock::new();
    let key = (
        match op {
            Operator::Plus => 0,
            Operator::Minus => 1,
            Operator::Multiply => 2,
            Operator::Divide => 3,
            _ => unreachable!(),
        },
        integer_on_left,
    );
    let mut cache = CACHE
        .get_or_init(|| Mutex::new(std::collections::HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    cache
        .entry(key)
        .or_insert_with(|| checked_mixed_new(op, integer_on_left))
        .clone()
        .call(vec![left, right])
}
fn checked_mixed_new(op: Operator, integer_on_left: bool) -> ScalarUDF {
    let types = if integer_on_left {
        vec![DataType::Int64, DataType::Float64]
    } else {
        vec![DataType::Float64, DataType::Int64]
    };
    create_udf(
        &format!("gql_checked_mixed_{op:?}_{integer_on_left}"),
        types,
        DataType::Float64,
        Volatility::Immutable,
        Arc::new(move |args| {
            let arrays = ColumnarValue::values_to_arrays(args)?;
            let index = usize::from(!integer_on_left);
            let integers = int64_array(&arrays[index])?;
            let floats = float64_array(&arrays[1 - index])?;
            let mut converted = Float64Builder::with_capacity(integers.len());
            for row in 0..integers.len() {
                if integers.is_null(row) || floats.is_null(row) {
                    converted.append_null();
                    continue;
                }
                let integer = integers.value(row);
                let float = exact_i64_f64(integer).ok_or_else(|| {
                    DataFusionError::Execution(format!(
                        "integer {integer} is not exactly representable as float"
                    ))
                })?;
                converted.append_value(float);
            }
            let mut numeric = arrays;
            numeric[index] = Arc::new(converted.finish());
            columnar_result(args, checked_arrays(op, &numeric)?)
        }),
    )
}

/// `left + (right - left) * weight`, with integer endpoints kept exact.
///
/// `weight` is the Float64 fractional position already computed for the
/// percentile. It is read as a dyadic rational, applied to the exact integer
/// span, and rounded once. Casting both endpoints first drops bits above 2^53.
pub(super) fn exact_lerp(left: Expr, right: Expr, weight: Expr) -> Expr {
    exact_lerp_udf().call(vec![left, right, weight])
}

fn exact_lerp_udf() -> ScalarUDF {
    use std::sync::OnceLock;
    static UDF: OnceLock<ScalarUDF> = OnceLock::new();
    UDF.get_or_init(|| {
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
    })
    .clone()
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

    #[test]
    fn identical_checked_expressions_compare_equal() {
        use datafusion::logical_expr::lit;
        let left = checked(col("n"), Operator::Plus, lit(1_i64), DataType::Int64);
        let right = checked(col("n"), Operator::Plus, lit(1_i64), DataType::Int64);
        assert_eq!(left, right);
        assert_ne!(
            left,
            checked(col("n"), Operator::Minus, lit(1_i64), DataType::Int64)
        );
    }

    #[test]
    fn identical_finite_checks_compare_equal() {
        assert_eq!(finite(col("n")), finite(col("n")));
    }

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

#[cfg(test)]
mod exact_average_tests {
    use super::ratio_to_f64;

    #[test]
    fn ratio_rounds_exact_binary_midpoints_and_integer_extremes() {
        for (n, d, expected) in [
            (9007199254740994, 2, 4503599627370497.0),
            (9007199254740995, 2199023255552, 4096.000000000002),
            (-9007199254740995, 2199023255552, -4096.000000000002),
            (9007199254741003, 2, 4503599627370502.0),
            (1, i128::MIN, -2.0_f64.powi(-127)),
            (i128::MIN, 1, -2.0_f64.powi(127)),
            (i128::MAX, i128::MAX - 1, 1.0),
            (4, 3, 4.0 / 3.0),
            (-3, 2, -1.5),
            (0, 5, 0.0),
        ] {
            assert_eq!(ratio_to_f64(n, d).to_bits(), expected.to_bits(), "{n}/{d}");
        }
    }
}

/// Integer SUM has one checked conversion function per accumulator type, so
/// repeated aggregates remain equal during grouping and DISTINCT rebasing.
pub(super) fn checked_int_sum(value: Expr, data_type: DataType) -> Result<Expr, DataFusionError> {
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<std::collections::HashMap<DataType, ScalarUDF>>> = OnceLock::new();
    let mut cache = CACHE
        .get_or_init(|| Mutex::new(std::collections::HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let function = if let Some(function) = cache.get(&data_type) {
        function.clone()
    } else {
        let function = checked_int_sum_new(data_type.clone())?;
        cache.insert(data_type, function.clone());
        function
    };
    Ok(function.call(vec![value]))
}

fn checked_int_sum_new(data_type: DataType) -> Result<ScalarUDF, DataFusionError> {
    let DataType::Decimal128(_, scale) = data_type.clone() else {
        return Err(DataFusionError::Internal(format!(
            "integer sum accumulator has type {data_type}"
        )));
    };
    if scale < 0 {
        return Err(DataFusionError::Internal(format!(
            "integer sum accumulator has scale {scale}"
        )));
    }
    let scale = scale as u32;
    Ok(create_udf(
        "gql_checked_int_sum",
        vec![data_type],
        DataType::Int64,
        Volatility::Immutable,
        Arc::new(move |args| {
            use datafusion::arrow::array::{Decimal128Array, Int64Builder};
            let arrays = ColumnarValue::values_to_arrays(args)?;
            let values = arrays[0]
                .as_any()
                .downcast_ref::<Decimal128Array>()
                .ok_or_else(|| {
                    DataFusionError::Internal("expected Decimal128 integer sum".into())
                })?;
            let factor = 10i128
                .checked_pow(scale)
                .ok_or_else(|| DataFusionError::Execution("integer overflow".into()))?;
            let mut output = Int64Builder::with_capacity(values.len());
            for value in values.iter() {
                let Some(value) = value else {
                    output.append_null();
                    continue;
                };
                if value % factor != 0 {
                    return Err(DataFusionError::Execution("integer overflow".into()));
                }
                let value = value / factor;
                let value = i64::try_from(value)
                    .map_err(|_| DataFusionError::Execution("integer overflow".into()))?;
                output.append_value(value);
            }
            let output: datafusion::arrow::array::ArrayRef = Arc::new(output.finish());
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
    ))
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
    Some(exact_compare_udf(op).call(vec![integer, float]))
}

fn exact_compare_udf(op: Operator) -> ScalarUDF {
    use std::sync::OnceLock;
    static FUNCTIONS: OnceLock<[ScalarUDF; 6]> = OnceLock::new();
    let operators = [
        Operator::Eq,
        Operator::NotEq,
        Operator::Lt,
        Operator::LtEq,
        Operator::Gt,
        Operator::GtEq,
    ];
    let index = operators
        .iter()
        .position(|candidate| *candidate == op)
        .expect("comparison operator");
    FUNCTIONS.get_or_init(|| operators.map(exact_compare_udf_new))[index].clone()
}
fn exact_compare_udf_new(op: Operator) -> ScalarUDF {
    let name = format!("gql_exact_int_float_{op:?}").to_lowercase();
    create_udf(
        &name,
        vec![DataType::Int64, DataType::Float64],
        DataType::Boolean,
        Volatility::Immutable,
        Arc::new(move |args| {
            use datafusion::arrow::array::{BooleanArray, Float64Array, Int64Array};
            let arrays = ColumnarValue::values_to_arrays(args)?;
            let integers = arrays[0]
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
    Some(exact_nullif_udf(integer_on_left).call(vec![left, right]))
}
fn exact_nullif_udf(integer_on_left: bool) -> ScalarUDF {
    use std::sync::OnceLock;
    static FUNCTIONS: OnceLock<[ScalarUDF; 2]> = OnceLock::new();
    FUNCTIONS.get_or_init(|| [exact_nullif_udf_new(false), exact_nullif_udf_new(true)])
        [usize::from(integer_on_left)]
    .clone()
}
fn exact_nullif_udf_new(integer_on_left: bool) -> ScalarUDF {
    let (left_type, right_type) = if integer_on_left {
        (DataType::Int64, DataType::Float64)
    } else {
        (DataType::Float64, DataType::Int64)
    };
    create_udf(
        if integer_on_left {
            "gql_exact_nullif_int_float"
        } else {
            "gql_exact_nullif_float_int"
        },
        vec![left_type.clone(), right_type],
        left_type,
        Volatility::Immutable,
        Arc::new(move |args| {
            let arrays = ColumnarValue::values_to_arrays(args)?;
            let output: datafusion::arrow::array::ArrayRef = if integer_on_left {
                Arc::new(nullif_int_float(
                    int64_array(&arrays[0])?,
                    float64_array(&arrays[1])?,
                ))
            } else {
                Arc::new(nullif_float_int(
                    float64_array(&arrays[0])?,
                    int64_array(&arrays[1])?,
                ))
            };
            columnar_result(args, output)
        }),
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
    cmp_i64_f64(integer, float) == Some(Ordering::Equal)
}

pub(super) fn exact_float(value: Expr) -> Expr {
    use std::sync::OnceLock;
    static FUNCTION: OnceLock<ScalarUDF> = OnceLock::new();
    FUNCTION
        .get_or_init(|| {
            create_udf(
                "gql_exact_int_to_float",
                vec![DataType::Int64],
                DataType::Float64,
                Volatility::Immutable,
                Arc::new(|args| {
                    let arrays = ColumnarValue::values_to_arrays(args)?;
                    let values = int64_array(&arrays[0])?
                        .iter()
                        .map(|value| {
                            value
                                .map(|value| {
                                    exact_i64_f64(value).ok_or_else(|| {
                                        DataFusionError::Execution(format!(
                                            "integer {value} is not exactly representable as float"
                                        ))
                                    })
                                })
                                .transpose()
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    columnar_result(args, Arc::new(Float64Array::from(values)))
                }),
            )
        })
        .call(vec![value])
}

/// CASE evaluates each fallback only on the rows that still need it. Integer
/// conversion belongs inside the selected arm, preserving exactness and laziness.
pub(super) fn lazy_coalesce(
    args: Vec<Expr>,
    types: &[DataType],
    return_type: DataType,
) -> Result<Expr, DataFusionError> {
    use datafusion::logical_expr::{cast, expr::Case, lit};
    let boolean = return_type == DataType::Boolean;
    let flag_type = if boolean {
        DataType::Int64
    } else {
        return_type.clone()
    };
    let mut result = lit(ScalarValue::try_from(&flag_type)?);
    for (arg, ty) in args.into_iter().zip(types).rev() {
        let value = if return_type == DataType::Float64 && *ty == DataType::Int64 {
            exact_float(arg.clone())
        } else if *ty != return_type {
            cast(arg.clone(), return_type.clone())
        } else {
            arg.clone()
        };
        let value = if boolean {
            cast(value, DataType::Int64)
        } else {
            value
        };
        result = Expr::Case(Case::new(
            None,
            vec![(Box::new(arg.is_not_null()), Box::new(value))],
            Some(Box::new(result)),
        ));
    }
    Ok(if boolean {
        super::expressions::boolean_value(result)
    } else {
        result
    })
}

pub(super) fn exact_coalesce(
    args: Vec<Expr>,
    types: &[DataType],
) -> Result<Option<Expr>, DataFusionError> {
    if types.contains(&DataType::Int64)
        && types.contains(&DataType::Float64)
        && types
            .iter()
            .all(|ty| matches!(ty, DataType::Int64 | DataType::Float64 | DataType::Null))
    {
        Ok(Some(lazy_coalesce(args, types, DataType::Float64)?))
    } else {
        Ok(None)
    }
}
