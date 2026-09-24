//! Arrow encodings of graph element references carried in paths and group lists.
//! These scalar functions only encode values; traversal stays in DataFusion joins.
use crate::{Error, Result};
use datafusion::{
    arrow::{
        array::{
            Array, ArrayRef, ListArray, ListBuilder, StringBuilder, StructBuilder, UInt64Array,
            UInt64Builder,
        },
        datatypes::{DataType, Field, Fields},
    },
    common::{DFSchema, ScalarValue},
    error::DataFusionError,
    functions::core::expr_fn::get_field,
    logical_expr::{cast, create_udf, lit, ColumnarValue, Expr, ExprSchemable, Volatility},
};
use std::sync::Arc;

const GRAPH: &str = "__gql_element_graph";
const KIND: &str = "__gql_element_kind";
const ID: &str = "__gql_element_id";
fn fields() -> Fields {
    vec![
        Field::new(GRAPH, DataType::UInt64, false),
        Field::new(KIND, DataType::Utf8, false),
        Field::new(ID, DataType::UInt64, false),
    ]
    .into()
}
fn data_type() -> DataType {
    DataType::Struct(fields())
}
pub(super) fn null_list() -> Expr {
    lit(ScalarValue::new_null_list(data_type(), true, 1))
}
fn append(builder: &mut StructBuilder, graph: u64, kind: &str, id: u64) {
    builder
        .field_builder::<UInt64Builder>(0)
        .expect("graph builder")
        .append_value(graph);
    builder
        .field_builder::<StringBuilder>(1)
        .expect("kind builder")
        .append_value(kind);
    builder
        .field_builder::<UInt64Builder>(2)
        .expect("id builder")
        .append_value(id);
    builder.append(true);
}

pub(super) fn identity(value: Expr, schema: &DFSchema) -> Result<Expr> {
    if value.get_type(schema)? == DataType::Null {
        return Ok(lit(ScalarValue::Utf8(None)));
    }
    if value.get_type(schema)? != data_type() {
        return Err(Error::InvalidQuery(
            "ELEMENT_ID requires an element reference".into(),
        ));
    }
    // concat ignores null arguments, so guard the null reference explicitly.
    let id = datafusion::functions::string::expr_fn::concat(vec![
        lit("g"),
        cast(get_field(value.clone(), GRAPH), DataType::Utf8),
        lit(":"),
        get_field(value.clone(), KIND),
        lit(":"),
        cast(get_field(value.clone(), ID), DataType::Utf8),
    ]);
    Ok(Expr::Case(datafusion::logical_expr::expr::Case::new(
        None,
        vec![(
            Box::new(value.is_null()),
            Box::new(lit(ScalarValue::Utf8(None))),
        )],
        Some(Box::new(id)),
    )))
}

/// `nodes` and `edges` are ordered ID lists. When both are supplied, interleave
/// them as node, edge, node; otherwise encode one element-kind group list.
pub(super) fn references(graph: Expr, nodes: Option<Expr>, edges: Option<Expr>) -> Expr {
    let path = nodes.is_some() && edges.is_some();
    let node_group = edges.is_none();
    let name = if path {
        "gql_path_elements"
    } else if node_group {
        "gql_node_group"
    } else {
        "gql_edge_group"
    };
    let list_type = DataType::new_list(DataType::UInt64, true);
    let mut input = vec![DataType::UInt64, list_type.clone()];
    if path {
        input.push(list_type);
    }
    let mut arguments = vec![graph];
    arguments.extend(nodes);
    arguments.extend(edges);
    create_udf(
        name,
        input,
        DataType::new_list(data_type(), true),
        Volatility::Immutable,
        Arc::new(move |args| {
            let arrays = ColumnarValue::values_to_arrays(args)?;
            let graphs = arrays[0]
                .as_any()
                .downcast_ref::<UInt64Array>()
                .ok_or_else(|| DataFusionError::Internal("element graph type".into()))?;
            let ids = arrays[1]
                .as_any()
                .downcast_ref::<ListArray>()
                .ok_or_else(|| DataFusionError::Internal("element list type".into()))?;
            let edge_ids = if path {
                Some(
                    arrays[2]
                        .as_any()
                        .downcast_ref::<ListArray>()
                        .ok_or_else(|| DataFusionError::Internal("path edge list type".into()))?,
                )
            } else {
                None
            };
            let mut builder = ListBuilder::new(StructBuilder::from_fields(fields(), 0));
            for row in 0..ids.len() {
                if graphs.is_null(row)
                    || ids.is_null(row)
                    || edge_ids.is_some_and(|a| a.is_null(row))
                {
                    builder.append(false);
                    continue;
                }
                let values = ids.value(row);
                let values = values
                    .as_any()
                    .downcast_ref::<UInt64Array>()
                    .ok_or_else(|| DataFusionError::Internal("element ID type".into()))?;
                let edges = edge_ids.map(|a| a.value(row));
                let edges = edges.as_ref().map(|a| {
                    a.as_any()
                        .downcast_ref::<UInt64Array>()
                        .expect("UInt64 edge IDs")
                });
                if edges.is_some_and(|e| e.len() + 1 != values.len()) {
                    return Err(DataFusionError::Internal(
                        "invalid path value lengths".into(),
                    ));
                }
                for (index, id) in values.iter().enumerate() {
                    let Some(id) = id else {
                        if path {
                            return Err(DataFusionError::Internal("null path element ID".into()));
                        }
                        let value = builder.values();
                        value
                            .field_builder::<UInt64Builder>(0)
                            .expect("graph builder")
                            .append_null();
                        value
                            .field_builder::<StringBuilder>(1)
                            .expect("kind builder")
                            .append_null();
                        value
                            .field_builder::<UInt64Builder>(2)
                            .expect("id builder")
                            .append_null();
                        value.append(false);
                        continue;
                    };
                    append(
                        builder.values(),
                        graphs.value(row),
                        if path || node_group { "n" } else { "e" },
                        id,
                    );
                    if let Some(edges) = edges {
                        if index < edges.len() {
                            append(builder.values(), graphs.value(row), "e", edges.value(index));
                        }
                    }
                }
                builder.append(true);
            }
            let output: ArrayRef = Arc::new(builder.finish());
            if args.iter().all(|a| matches!(a, ColumnarValue::Scalar(_))) {
                Ok(ColumnarValue::Scalar(ScalarValue::try_from_array(
                    &output, 0,
                )?))
            } else {
                Ok(ColumnarValue::Array(output))
            }
        }),
    )
    .call(arguments)
}
