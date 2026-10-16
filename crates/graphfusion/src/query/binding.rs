//! Binding-table inputs: session parameters and nested queries.
use super::{execution, plan, unsupported, WorkingTable};
use crate::{gql as ast, transaction::StatementTxn, Error, Result, SessionState, Value};
use datafusion::{
    arrow::{
        array::{
            ArrayRef, BinaryBuilder, BooleanArray, Float64Array, Int64Array, NullArray, StringArray,
        },
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    execution::context::SessionContext,
    logical_expr::{LogicalPlan, LogicalPlanBuilder},
};
use std::{collections::BTreeMap, sync::Arc};

#[allow(clippy::too_many_arguments)]
pub(super) async fn source(
    expression: &ast::BindingTableExpression,
    session: &SessionState,
    graph: Option<crate::catalog::ObjectId>,
    tx: &mut StatementTxn,
    ctx: &SessionContext,
    writes: &mut super::mutations::Writes,
    trace: &mut execution::Trace,
    correlated: bool,
) -> Result<LogicalPlan> {
    match expression {
        ast::BindingTableExpression::NestedQuery(query) => {
            if correlated {
                return Err(unsupported("correlated binding table"));
            }
            let mut nested = session.clone();
            nested.current_graph = graph;
            let (plan, _) = Box::pin(plan(
                query,
                &nested,
                tx,
                ctx,
                writes,
                trace,
                false,
                None::<WorkingTable>,
            ))
            .await?;
            Ok(plan)
        }
        ast::BindingTableExpression::Parameter(name) => parameter_table(session, name),
        ast::BindingTableExpression::Variable(expression) => match expression.as_ref() {
            ast::Expr::Parameter(name) | ast::Expr::Identifier(ast::Identifier { value: name }) => {
                parameter_table(session, name)
            }
            _ => Err(unsupported("binding-table value expression")),
        },
        ast::BindingTableExpression::Name(name)
            if name.parts.len() == 1
                && !name.absolute
                && !name.current_schema
                && !name.home_schema
                && name.parent_levels == 0 =>
        {
            parameter_table(session, &name.parts[0].value)
        }
        ast::BindingTableExpression::Name(_) => Err(unsupported("catalog binding-table reference")),
    }
}

fn parameter_table(session: &SessionState, name: &str) -> Result<LogicalPlan> {
    let parameter = session
        .parameters
        .get(name)
        .ok_or_else(|| Error::NotFound(format!("parameter {name}")))?;
    let Value::BindingTable(rows) = &parameter.value else {
        return Err(Error::InvalidQuery(format!(
            "parameter {name} is not a binding table"
        )));
    };
    rows_to_plan(rows)
}

fn rows_to_plan(rows: &[BTreeMap<String, Value>]) -> Result<LogicalPlan> {
    if rows.is_empty() {
        return Ok(LogicalPlanBuilder::empty(false).build()?);
    }
    let mut names = std::collections::BTreeSet::new();
    for row in rows {
        names.extend(row.keys().cloned());
    }
    let mut fields = Vec::new();
    let mut columns = Vec::new();
    for name in &names {
        let mut unified = None;
        for row in rows {
            let Some(value) = row.get(name) else {
                continue;
            };
            if matches!(value, Value::Null) {
                continue;
            }
            let next = scalar_type(value)?;
            unified = Some(match unified {
                None => next,
                Some(previous) => {
                    crate::graph::unify_property_type(&previous, &next).ok_or_else(|| {
                        Error::InvalidQuery(format!("mixed types for binding-table column {name}"))
                    })?
                }
            });
        }
        // An all-null column has no scalar type. Utf8 would make `p.id = id`
        // a string/integer comparison and reject a typed graph lookup.
        let data_type = unified.unwrap_or(DataType::Null);
        columns.push(column_array(name, &data_type, rows)?);
        fields.push(Field::new(name, data_type, true));
    }
    let schema = Arc::new(Schema::new(fields));
    let batch = RecordBatch::try_new(schema.clone(), columns)
        .map_err(datafusion::error::DataFusionError::from)?;
    Ok(execution::memory_named("__gf_binding", schema, vec![batch])?.build()?)
}

fn scalar_type(value: &Value) -> Result<DataType> {
    Ok(match value {
        Value::Boolean(_) => DataType::Boolean,
        Value::Integer(_) => DataType::Int64,
        Value::Float(_) => DataType::Float64,
        Value::String(_) => DataType::Utf8,
        Value::Bytes(_) => DataType::Binary,
        Value::Null => DataType::Utf8,
        _ => return Err(unsupported("binding-table column must be a scalar value")),
    })
}

fn column_array(
    name: &str,
    data_type: &DataType,
    rows: &[BTreeMap<String, Value>],
) -> Result<ArrayRef> {
    let values = rows.iter().map(|row| row.get(name));
    let array: ArrayRef = match data_type {
        DataType::Null => Arc::new(NullArray::new(rows.len())),
        DataType::Boolean => Arc::new(BooleanArray::from(
            values
                .map(|value| match value {
                    Some(Value::Boolean(value)) => Some(*value),
                    Some(Value::Null) | None => None,
                    _ => None,
                })
                .collect::<Vec<_>>(),
        )),
        DataType::Int64 => Arc::new(Int64Array::from(
            values
                .map(|value| match value {
                    Some(Value::Integer(value)) => Some(*value),
                    Some(Value::Null) | None => None,
                    _ => None,
                })
                .collect::<Vec<_>>(),
        )),
        DataType::Float64 => Arc::new(Float64Array::from(
            values
                .map(|value| match value {
                    Some(Value::Float(value)) => Some(*value),
                    Some(Value::Integer(value)) => Some(*value as f64),
                    Some(Value::Null) | None => None,
                    _ => None,
                })
                .collect::<Vec<_>>(),
        )),
        DataType::Utf8 => Arc::new(StringArray::from(
            values
                .map(|value| match value {
                    Some(Value::String(value)) => Some(value.clone()),
                    Some(Value::Boolean(value)) => Some(value.to_string()),
                    Some(Value::Integer(value)) => Some(value.to_string()),
                    Some(Value::Float(value)) => Some(value.to_string()),
                    Some(Value::Null) | None => None,
                    _ => None,
                })
                .collect::<Vec<_>>(),
        )),
        DataType::Binary => {
            let mut builder = BinaryBuilder::new();
            for value in values {
                match value {
                    Some(Value::Bytes(value)) => builder.append_value(value),
                    _ => builder.append_null(),
                }
            }
            Arc::new(builder.finish())
        }
        _ => return Err(unsupported("binding-table column type")),
    };
    Ok(array)
}
