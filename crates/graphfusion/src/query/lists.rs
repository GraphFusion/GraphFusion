//! Three-valued list comparisons. Element nulls stay unknown unless an earlier
//! element, or the list lengths after an equal prefix, already decides the result.
use datafusion::{
    arrow::{
        array::{Array, ArrayRef, BooleanBuilder},
        datatypes::DataType,
    },
    common::ScalarValue,
    error::DataFusionError,
    logical_expr::{create_udf, ColumnarValue, Operator, Volatility},
};
use std::{cmp::Ordering, sync::Arc};

pub(super) fn is_list(data_type: &DataType) -> bool {
    matches!(data_type, DataType::List(_) | DataType::LargeList(_))
}

pub(super) fn compare(
    left: datafusion::logical_expr::Expr,
    right: datafusion::logical_expr::Expr,
    left_type: &DataType,
    right_type: &DataType,
    op: Operator,
) -> crate::Result<datafusion::logical_expr::Expr> {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    format!("{op:?}:{left_type:?}:{right_type:?}").hash(&mut hasher);
    let name = format!("gql_list_{}_{:x}", op_name(op), hasher.finish());
    Ok(create_udf(
        &name,
        vec![left_type.clone(), right_type.clone()],
        DataType::Boolean,
        Volatility::Immutable,
        Arc::new(move |args| evaluate(args, op)),
    )
    .call(vec![left, right]))
}

fn op_name(op: Operator) -> &'static str {
    match op {
        Operator::Eq => "eq",
        Operator::NotEq => "neq",
        Operator::Lt => "lt",
        Operator::LtEq => "le",
        Operator::Gt => "gt",
        Operator::GtEq => "ge",
        _ => "cmp",
    }
}

fn evaluate(args: &[ColumnarValue], op: Operator) -> Result<ColumnarValue, DataFusionError> {
    let arrays = ColumnarValue::values_to_arrays(args)?;
    let left = ScalarValue::convert_array_to_scalar_vec(&arrays[0])?;
    let right = ScalarValue::convert_array_to_scalar_vec(&arrays[1])?;
    if left.len() != right.len() {
        return Err(DataFusionError::Internal(
            "list comparison inputs have different lengths".into(),
        ));
    }
    let mut builder = BooleanBuilder::with_capacity(left.len());
    for (left, right) in left.into_iter().zip(right) {
        match (left, right) {
            (Some(left), Some(right)) => match satisfies(rel_sequence(&left, &right)?, op)? {
                Some(value) => builder.append_value(value),
                None => builder.append_null(),
            },
            _ => builder.append_null(),
        }
    }
    let output: ArrayRef = Arc::new(builder.finish());
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum Rel {
    Lt,
    Eq,
    Gt,
    Unknown,
}

fn satisfies(rel: Rel, op: Operator) -> Result<Option<bool>, DataFusionError> {
    if rel == Rel::Unknown {
        return Ok(None);
    }
    let value = match op {
        Operator::Eq => rel == Rel::Eq,
        Operator::NotEq => rel != Rel::Eq,
        Operator::Lt => rel == Rel::Lt,
        Operator::Gt => rel == Rel::Gt,
        Operator::LtEq => rel == Rel::Lt || rel == Rel::Eq,
        Operator::GtEq => rel == Rel::Gt || rel == Rel::Eq,
        _ => return Err(DataFusionError::Internal("list comparison operator".into())),
    };
    Ok(Some(value))
}

/// Equal prefix then length. An unknown element stops the walk: a later element
/// cannot override a comparison that might already have been decided.
fn rel_sequence(left: &[ScalarValue], right: &[ScalarValue]) -> Result<Rel, DataFusionError> {
    let shared = left.len().min(right.len());
    for index in 0..shared {
        match rel_value(&left[index], &right[index])? {
            Rel::Eq => {}
            decided => return Ok(decided),
        }
    }
    Ok(from_ord(left.len().cmp(&right.len())))
}

fn rel_value(left: &ScalarValue, right: &ScalarValue) -> Result<Rel, DataFusionError> {
    if left.is_null() || right.is_null() {
        return Ok(Rel::Unknown);
    }
    if let Some(rel) = primitive_rel(left, right) {
        return Ok(rel);
    }
    if let (Some(left), Some(right)) = (list_elements(left)?, list_elements(right)?) {
        return rel_sequence(&left, &right);
    }
    if let (Some(left), Some(right)) = (struct_fields(left)?, struct_fields(right)?) {
        return rel_sequence(&left, &right);
    }
    Err(DataFusionError::Execution(
        "incomparable list elements".into(),
    ))
}

fn from_ord(order: Ordering) -> Rel {
    match order {
        Ordering::Less => Rel::Lt,
        Ordering::Equal => Rel::Eq,
        Ordering::Greater => Rel::Gt,
    }
}

fn primitive_rel(left: &ScalarValue, right: &ScalarValue) -> Option<Rel> {
    if let (Some(left), Some(right)) = (boolean(left), boolean(right)) {
        return Some(from_ord(left.cmp(&right)));
    }
    if let (Some(left), Some(right)) = (integer(left), integer(right)) {
        return Some(from_ord(left.cmp(&right)));
    }
    if let (Some(left), Some(right)) = (number(left), number(right)) {
        return Some(match left.partial_cmp(&right) {
            Some(order) => from_ord(order),
            None => Rel::Unknown,
        });
    }
    if let (Some(left), Some(right)) = (text(left), text(right)) {
        return Some(from_ord(left.cmp(right)));
    }
    if let (Some(left), Some(right)) = (bytes(left), bytes(right)) {
        return Some(from_ord(left.cmp(right)));
    }
    None
}

fn boolean(value: &ScalarValue) -> Option<bool> {
    match value {
        ScalarValue::Boolean(Some(value)) => Some(*value),
        _ => None,
    }
}

fn integer(value: &ScalarValue) -> Option<i128> {
    match value {
        ScalarValue::Int8(Some(value)) => Some(i128::from(*value)),
        ScalarValue::Int16(Some(value)) => Some(i128::from(*value)),
        ScalarValue::Int32(Some(value)) => Some(i128::from(*value)),
        ScalarValue::Int64(Some(value)) => Some(i128::from(*value)),
        ScalarValue::UInt8(Some(value)) => Some(i128::from(*value)),
        ScalarValue::UInt16(Some(value)) => Some(i128::from(*value)),
        ScalarValue::UInt32(Some(value)) => Some(i128::from(*value)),
        ScalarValue::UInt64(Some(value)) => Some(i128::from(*value)),
        _ => None,
    }
}

fn number(value: &ScalarValue) -> Option<f64> {
    match value {
        ScalarValue::Float32(Some(value)) => Some(f64::from(*value)),
        ScalarValue::Float64(Some(value)) => Some(*value),
        ScalarValue::Int8(Some(value)) => Some(f64::from(*value)),
        ScalarValue::Int16(Some(value)) => Some(f64::from(*value)),
        ScalarValue::Int32(Some(value)) => Some(f64::from(*value)),
        ScalarValue::Int64(Some(value)) => Some(*value as f64),
        ScalarValue::UInt8(Some(value)) => Some(f64::from(*value)),
        ScalarValue::UInt16(Some(value)) => Some(f64::from(*value)),
        ScalarValue::UInt32(Some(value)) => Some(f64::from(*value)),
        ScalarValue::UInt64(Some(value)) => Some(*value as f64),
        _ => None,
    }
}

fn text(value: &ScalarValue) -> Option<&str> {
    match value {
        ScalarValue::Utf8(Some(value))
        | ScalarValue::LargeUtf8(Some(value))
        | ScalarValue::Utf8View(Some(value)) => Some(value),
        _ => None,
    }
}

fn bytes(value: &ScalarValue) -> Option<&[u8]> {
    match value {
        ScalarValue::Binary(Some(value))
        | ScalarValue::LargeBinary(Some(value))
        | ScalarValue::BinaryView(Some(value))
        | ScalarValue::FixedSizeBinary(_, Some(value)) => Some(value),
        _ => None,
    }
}

fn list_elements(value: &ScalarValue) -> Result<Option<Vec<ScalarValue>>, DataFusionError> {
    let values = match value {
        ScalarValue::List(array) => {
            if array.len() != 1 {
                return Err(DataFusionError::Internal("list scalar length".into()));
            }
            if array.is_null(0) {
                return Ok(None);
            }
            array.value(0)
        }
        ScalarValue::LargeList(array) => {
            if array.len() != 1 {
                return Err(DataFusionError::Internal("list scalar length".into()));
            }
            if array.is_null(0) {
                return Ok(None);
            }
            array.value(0)
        }
        _ => return Ok(None),
    };
    (0..values.len())
        .map(|index| ScalarValue::try_from_array(values.as_ref(), index))
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

fn struct_fields(value: &ScalarValue) -> Result<Option<Vec<ScalarValue>>, DataFusionError> {
    let ScalarValue::Struct(array) = value else {
        return Ok(None);
    };
    if array.len() != 1 {
        return Err(DataFusionError::Internal("struct scalar length".into()));
    }
    if array.is_null(0) {
        return Ok(None);
    }
    array
        .columns()
        .iter()
        .map(|column| ScalarValue::try_from_array(column.as_ref(), 0))
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}
