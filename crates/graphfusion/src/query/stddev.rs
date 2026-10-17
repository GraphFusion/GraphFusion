//! Integer standard deviation from the exact sum and sum of squares.
//!
//! DataFusion accumulates these moments in Float64, so integers past 2^53 lose
//! low bits before the square root. The population and sample results here keep
//! the moments exact and round the square root once.
use datafusion::{
    arrow::{
        array::{Array, ArrayRef, AsArray, Decimal128Array, Decimal256Array},
        datatypes::{i256, DataType, Field, Int64Type},
    },
    common::ScalarValue,
    error::{DataFusionError, Result},
    logical_expr::{
        function::{AccumulatorArgs, StateFieldsArgs},
        utils::format_state_name,
        Accumulator, AggregateUDF, AggregateUDFImpl, Expr, Signature, Volatility,
    },
};
use std::{cmp::Ordering, collections::HashSet, mem::size_of, sync::Arc};

pub(super) fn integer_stddev(value: Expr, population: bool) -> Expr {
    AggregateUDF::new_from_impl(IntegerStddev::new(population)).call(vec![value])
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct IntegerStddev {
    population: bool,
    signature: Signature,
}

impl IntegerStddev {
    fn new(population: bool) -> Self {
        Self {
            population,
            signature: Signature::exact(vec![DataType::Int64], Volatility::Immutable),
        }
    }
}

impl AggregateUDFImpl for IntegerStddev {
    fn name(&self) -> &str {
        if self.population {
            "gql_stddev_pop_i64"
        } else {
            "gql_stddev_samp_i64"
        }
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> Result<DataType> {
        Ok(DataType::Float64)
    }

    fn accumulator(&self, acc_args: AccumulatorArgs) -> Result<Box<dyn Accumulator>> {
        Ok(Box::new(StddevAccumulator {
            population: self.population,
            distinct: acc_args.is_distinct,
            count: 0,
            sum: 0,
            sum_squares: i256::ZERO,
            values: HashSet::new(),
        }))
    }

    fn state_fields(&self, args: StateFieldsArgs) -> Result<Vec<Arc<Field>>> {
        if args.is_distinct {
            return Ok(vec![Arc::new(Field::new(
                format_state_name(args.name, "values"),
                DataType::List(Arc::new(Field::new("item", DataType::Int64, true))),
                true,
            ))]);
        }
        Ok(vec![
            Arc::new(Field::new(
                format_state_name(args.name, "count"),
                DataType::Int64,
                true,
            )),
            Arc::new(Field::new(
                format_state_name(args.name, "sum"),
                DataType::Decimal128(38, 0),
                true,
            )),
            Arc::new(Field::new(
                format_state_name(args.name, "sumsq"),
                DataType::Decimal256(76, 0),
                true,
            )),
        ])
    }
}

#[derive(Clone, Debug)]
struct StddevAccumulator {
    population: bool,
    distinct: bool,
    count: i64,
    sum: i128,
    sum_squares: i256,
    values: HashSet<i64>,
}

impl StddevAccumulator {
    fn absorb(&mut self, value: i64) -> Result<()> {
        self.count = self.count.checked_add(1).ok_or_else(overflow)?;
        self.sum = self
            .sum
            .checked_add(i128::from(value))
            .ok_or_else(overflow)?;
        let wide = i256::from(value);
        let square = wide.checked_mul(wide).ok_or_else(overflow)?;
        self.sum_squares = self.sum_squares.checked_add(square).ok_or_else(overflow)?;
        Ok(())
    }

    fn moments(&self) -> Result<(i64, i128, i256)> {
        if !self.distinct {
            return Ok((self.count, self.sum, self.sum_squares));
        }
        let count = i64::try_from(self.values.len()).map_err(|_| overflow())?;
        let mut sum = 0i128;
        let mut sum_squares = i256::ZERO;
        for &value in &self.values {
            sum = sum.checked_add(i128::from(value)).ok_or_else(overflow)?;
            let wide = i256::from(value);
            let square = wide.checked_mul(wide).ok_or_else(overflow)?;
            sum_squares = sum_squares.checked_add(square).ok_or_else(overflow)?;
        }
        Ok((count, sum, sum_squares))
    }
}

impl Accumulator for StddevAccumulator {
    fn update_batch(&mut self, values: &[ArrayRef]) -> Result<()> {
        let values = values[0].as_primitive::<Int64Type>();
        for index in 0..values.len() {
            if values.is_null(index) {
                continue;
            }
            let value = values.value(index);
            if self.distinct {
                self.values.insert(value);
            } else {
                self.absorb(value)?;
            }
        }
        Ok(())
    }

    fn evaluate(&mut self) -> Result<ScalarValue> {
        let (count, sum, sum_squares) = self.moments()?;
        Ok(ScalarValue::Float64(stddev_from_moments(
            count,
            sum,
            sum_squares,
            self.population,
        )?))
    }

    fn size(&self) -> usize {
        size_of::<Self>() + self.values.capacity() * size_of::<i64>()
    }

    fn state(&mut self) -> Result<Vec<ScalarValue>> {
        if self.distinct {
            let keys: Vec<_> = self
                .values
                .iter()
                .copied()
                .map(|value| ScalarValue::Int64(Some(value)))
                .collect();
            return Ok(vec![ScalarValue::List(ScalarValue::new_list_nullable(
                &keys,
                &DataType::Int64,
            ))]);
        }
        Ok(vec![
            ScalarValue::Int64(Some(self.count)),
            ScalarValue::Decimal128(Some(self.sum), 38, 0),
            ScalarValue::Decimal256(Some(self.sum_squares), 76, 0),
        ])
    }

    fn merge_batch(&mut self, states: &[ArrayRef]) -> Result<()> {
        if self.distinct {
            let lists = states[0].as_list::<i32>();
            for list in lists.iter().flatten() {
                let values = list.as_primitive::<Int64Type>();
                for index in 0..values.len() {
                    if !values.is_null(index) {
                        self.values.insert(values.value(index));
                    }
                }
            }
            return Ok(());
        }
        let counts = states[0].as_primitive::<Int64Type>();
        let sums = states[1]
            .as_any()
            .downcast_ref::<Decimal128Array>()
            .ok_or_else(|| DataFusionError::Internal("expected decimal sum".into()))?;
        let squares = states[2]
            .as_any()
            .downcast_ref::<Decimal256Array>()
            .ok_or_else(|| DataFusionError::Internal("expected decimal sum of squares".into()))?;
        for index in 0..counts.len() {
            if counts.is_null(index) {
                continue;
            }
            self.count = self
                .count
                .checked_add(counts.value(index))
                .ok_or_else(overflow)?;
            if !sums.is_null(index) {
                self.sum = self
                    .sum
                    .checked_add(sums.value(index))
                    .ok_or_else(overflow)?;
            }
            if !squares.is_null(index) {
                self.sum_squares = self
                    .sum_squares
                    .checked_add(squares.value(index))
                    .ok_or_else(overflow)?;
            }
        }
        Ok(())
    }
}

fn overflow() -> DataFusionError {
    DataFusionError::Execution("integer standard deviation exceeds the exact range".into())
}

fn stddev_from_moments(
    count: i64,
    sum: i128,
    sum_squares: i256,
    population: bool,
) -> Result<Option<f64>> {
    if count <= 0 || (!population && count < 2) {
        return Ok(None);
    }
    if count == 1 {
        return Ok(Some(0.0));
    }
    let count_wide = i256::from(count);
    let numer = count_wide
        .checked_mul(sum_squares)
        .ok_or_else(overflow)?
        .checked_sub(
            i256::from(sum)
                .checked_mul(i256::from(sum))
                .ok_or_else(overflow)?,
        )
        .ok_or_else(overflow)?;
    if numer.is_negative() {
        return Err(overflow());
    }
    if numer == i256::ZERO {
        return Ok(Some(0.0));
    }
    let count = u128::try_from(count).map_err(|_| overflow())?;
    let denom = if population {
        count.checked_mul(count)
    } else {
        count.checked_mul(count - 1)
    }
    .ok_or_else(overflow)?;
    Ok(Some(round_sqrt_ratio(&unsigned_from_i256(numer)?, denom)?))
}

fn unsigned_from_i256(value: i256) -> Result<Magnitude> {
    if value.is_negative() {
        return Err(overflow());
    }
    let bytes = value.to_le_bytes();
    let mut limbs = [0u64; 4];
    for (index, limb) in limbs.iter_mut().enumerate() {
        let start = index * 8;
        let mut buf = [0u8; 8];
        buf.copy_from_slice(&bytes[start..start + 8]);
        *limb = u64::from_le_bytes(buf);
    }
    Ok(Magnitude::from_limbs(&limbs))
}

/// Round `sqrt(numer / denom)` to the nearest Float64, ties to even.
fn round_sqrt_ratio(numer: &Magnitude, denom: u128) -> Result<f64> {
    if numer.is_zero() || denom == 0 {
        return Ok(0.0);
    }
    let mut best = approximate_sqrt(numer, denom);
    if !best.is_finite() || best <= 0.0 {
        return Err(overflow());
    }
    for _ in 0..8 {
        let mut moved = false;
        let up = best.next_up();
        if up.is_finite() && prefers(up, best, numer, denom) {
            best = up;
            moved = true;
        }
        let down = best.next_down();
        if down > 0.0 && prefers(down, best, numer, denom) {
            best = down;
            moved = true;
        }
        if !moved {
            return Ok(best);
        }
    }
    Err(overflow())
}

fn prefers(candidate: f64, current: f64, numer: &Magnitude, denom: u128) -> bool {
    match scaled_distance(candidate, numer, denom)
        .cmp_magnitude(&scaled_distance(current, numer, denom))
    {
        Ordering::Less => true,
        Ordering::Greater => false,
        Ordering::Equal => significand_even(candidate) && !significand_even(current),
    }
}

fn significand_even(value: f64) -> bool {
    value.to_bits() & 1 == 0
}

fn approximate_sqrt(numer: &Magnitude, denom: u128) -> f64 {
    let (numer_sig, numer_pow) = leading_window(numer);
    let (denom_sig, denom_pow) = leading_window(&Magnitude::from_u128(denom));
    let mut exponent = numer_pow - denom_pow;
    let mut fraction = numer_sig / denom_sig;
    if exponent % 2 != 0 {
        if exponent > 0 {
            fraction *= 2.0;
            exponent -= 1;
        } else {
            fraction *= 0.5;
            exponent += 1;
        }
    }
    fraction.sqrt() * 2f64.powi(exponent / 2)
}

/// Leading 53 bits as a Float64 in `[2^52, 2^53)` and the power of two below them.
fn leading_window(magnitude: &Magnitude) -> (f64, i32) {
    let highest = magnitude.highest_bit().unwrap_or(0) as i32;
    let mut significand = 0u64;
    for offset in 0..53 {
        let index = highest - offset;
        if index >= 0 && magnitude.bit(index as u32) {
            significand |= 1u64 << (52 - offset);
        }
    }
    (significand as f64, highest - 52)
}

fn scaled_distance(value: f64, numer: &Magnitude, denom: u128) -> Magnitude {
    let Some((mantissa, power)) = positive_dyadic(value) else {
        return Magnitude::from_u128(u128::MAX).shift_left(128);
    };
    let product = mul_u128(u128::from(mantissa) * u128::from(mantissa), denom);
    let shift = i64::from(power) * 2;
    if shift >= 0 {
        abs_diff(
            &product.shift_left(u32::try_from(shift).unwrap_or(u32::MAX)),
            numer,
        )
    } else {
        let Ok(bits) = u32::try_from(-shift) else {
            return Magnitude::from_u128(u128::MAX).shift_left(128);
        };
        abs_diff(&product, &numer.shift_left(bits))
    }
}

fn positive_dyadic(value: f64) -> Option<(u64, i32)> {
    if !(value > 0.0 && value.is_finite()) {
        return None;
    }
    let bits = value.to_bits();
    let exponent = ((bits >> 52) & 0x7ff) as i32;
    let fraction = bits & ((1u64 << 52) - 1);
    if exponent == 0 {
        Some((fraction, -1074))
    } else {
        Some(((1u64 << 52) | fraction, exponent - 1023 - 52))
    }
}

fn mul_u128(left: u128, right: u128) -> Magnitude {
    let left_low = left as u64;
    let left_high = (left >> 64) as u64;
    let right_low = right as u64;
    let right_high = (right >> 64) as u64;
    let low = u128::from(left_low) * u128::from(right_low);
    let mid_left = u128::from(left_low) * u128::from(right_high);
    let mid_right = u128::from(left_high) * u128::from(right_low);
    let high = u128::from(left_high) * u128::from(right_high);
    let mut limbs = vec![
        low as u64,
        (low >> 64) as u64,
        high as u64,
        (high >> 64) as u64,
    ];
    add_shifted(&mut limbs, 1, mid_left);
    add_shifted(&mut limbs, 1, mid_right);
    Magnitude::normalized(limbs)
}

fn add_shifted(limbs: &mut Vec<u64>, offset: usize, value: u128) {
    let mut carry = value;
    let mut index = offset;
    while carry != 0 {
        if index == limbs.len() {
            limbs.push(0);
        }
        let sum = u128::from(limbs[index]) + (carry & 0xffff_ffff_ffff_ffff);
        limbs[index] = sum as u64;
        carry = (carry >> 64) + (sum >> 64);
        index += 1;
    }
}

fn abs_diff(left: &Magnitude, right: &Magnitude) -> Magnitude {
    match left.cmp_magnitude(right) {
        Ordering::Less => right.saturating_sub(left),
        _ => left.saturating_sub(right),
    }
}

/// Little-endian unsigned magnitude. Empty means zero.
#[derive(Clone, Debug)]
struct Magnitude(Vec<u64>);

impl Magnitude {
    fn from_u128(value: u128) -> Self {
        Self::from_limbs(&[value as u64, (value >> 64) as u64])
    }

    fn from_limbs(limbs: &[u64]) -> Self {
        Self::normalized(limbs.to_vec())
    }

    fn normalized(mut limbs: Vec<u64>) -> Self {
        while limbs.last() == Some(&0) {
            limbs.pop();
        }
        Self(limbs)
    }

    fn is_zero(&self) -> bool {
        self.0.is_empty()
    }

    fn shift_left(&self, bits: u32) -> Self {
        if self.is_zero() || bits == 0 {
            return self.clone();
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

    fn saturating_sub(&self, other: &Self) -> Self {
        let mut limbs = Vec::with_capacity(self.0.len());
        let mut borrow = 0u64;
        for index in 0..self.0.len() {
            let left = self.0[index];
            let right = other.0.get(index).copied().unwrap_or(0);
            let (difference, overflow) = left.overflowing_sub(right);
            let (difference, overflow_borrow) = difference.overflowing_sub(borrow);
            limbs.push(difference);
            borrow = u64::from(overflow || overflow_borrow);
        }
        Self::normalized(limbs)
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

    fn highest_bit(&self) -> Option<u32> {
        let (index, limb) = self.0.iter().enumerate().rfind(|(_, limb)| **limb != 0)?;
        Some(index as u32 * 64 + 63 - limb.leading_zeros())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn population(values: &[i64]) -> f64 {
        deviation(values, true).unwrap()
    }

    fn sample(values: &[i64]) -> f64 {
        deviation(values, false).unwrap()
    }

    fn deviation(values: &[i64], population: bool) -> Option<f64> {
        let mut accumulator = StddevAccumulator {
            population,
            distinct: false,
            count: 0,
            sum: 0,
            sum_squares: i256::ZERO,
            values: HashSet::new(),
        };
        for &value in values {
            accumulator.absorb(value).unwrap();
        }
        match accumulator.evaluate().unwrap() {
            ScalarValue::Float64(value) => value,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn integer_stddev_rounds_the_exact_square_root() {
        assert_eq!(population(&[9007199254740993, 1]), 4503599627370496.0);
        assert_eq!(sample(&[9007199254740993, 1]).to_bits(), 0x4336a09e667f3bcd);
        assert_eq!(
            population(&[9007199254740993, 9007199254740993, 1]).to_bits(),
            0x432e2b7dddfefa66
        );
        assert_eq!(population(&[1, 2, 3, 4]).to_bits(), 0x3ff1e3779b97f4a8);
        assert_eq!(sample(&[1, 2, 3, 4]).to_bits(), 0x3ff4a7e9cb8a3491);
        assert_eq!(population(&[10, 20, 30]).to_bits(), 0x4020547666079ba7);
        assert_eq!(sample(&[10, 20, 30]), 10.0);
        assert_eq!(population(&[1]), 0.0);
        assert_eq!(deviation(&[1], false), None);
        assert_eq!(deviation(&[], true), None);
        assert_eq!(population(&[-2, 2]), 2.0);
        assert_eq!(population(&[4, 4, 4]), 0.0);
        assert!(sample(&[i64::MIN, i64::MAX]).is_finite());
    }

    #[test]
    fn partial_moments_merge_before_the_square_root() {
        let mut left = StddevAccumulator {
            population: true,
            distinct: false,
            count: 0,
            sum: 0,
            sum_squares: i256::ZERO,
            values: HashSet::new(),
        };
        let mut right = left.clone();
        left.absorb(9007199254740993).unwrap();
        right.absorb(1).unwrap();
        let left_state = left.state().unwrap();
        let right_state = right.state().unwrap();
        let states = (0..left_state.len())
            .map(|index| {
                ScalarValue::iter_to_array([left_state[index].clone(), right_state[index].clone()])
                    .unwrap()
            })
            .collect::<Vec<_>>();
        let mut merged = StddevAccumulator {
            population: true,
            distinct: false,
            count: 0,
            sum: 0,
            sum_squares: i256::ZERO,
            values: HashSet::new(),
        };
        merged.merge_batch(&states).unwrap();
        assert_eq!(
            merged.evaluate().unwrap(),
            ScalarValue::Float64(Some(4503599627370496.0))
        );
    }
}
