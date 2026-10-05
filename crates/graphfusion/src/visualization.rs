//! Bounded, snapshot-consistent graph projections for graphical query clients.
//! Only returned elements (and the endpoints of returned edges) are included.
use crate::{
    catalog::ObjectId,
    graph::{GraphData, Table, DESTINATION, ID, SOURCE},
    Result,
};
use datafusion::{
    arrow::{
        array::{Array, ArrayRef, FixedSizeListArray, LargeListArray, ListArray, StructArray},
        datatypes::DataType,
        record_batch::RecordBatch,
        util::display::array_value_to_string,
    },
    execution::context::SessionContext,
};
use serde::Serialize;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

pub const DISPLAY_ROWS: usize = 1000;
const MAX_REFERENCES: usize = 500;
type Reference = (ObjectId, char, u64);

#[derive(Debug, Default, Serialize)]
pub struct GraphProjection {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    /// Graph references are capped at 500; edge endpoints are included additionally.
    pub truncated: bool,
    /// References to elements no longer present in the result snapshot.
    pub unresolved: usize,
}
#[derive(Debug, Serialize)]
pub struct GraphNode {
    pub id: String,
    pub labels: Vec<String>,
    pub properties: BTreeMap<String, Value>,
}
#[derive(Debug, Serialize)]
pub struct GraphEdge {
    pub id: String,
    pub source: String,
    pub target: String,
    pub directed: bool,
    pub labels: Vec<String>,
    pub properties: BTreeMap<String, Value>,
}
fn identity(graph: ObjectId, kind: char, id: u64) -> String {
    format!("g{graph}:{kind}:{id}")
}

/// JSON cells preserve graph references and nested lists/records. Integers outside
/// JavaScript's exact range are strings; temporal/decimal/binary values use Arrow's
/// display representation rather than a lossy floating-point conversion.
pub fn cell(array: &ArrayRef, row: usize) -> Result<Value> {
    value(array, row, &mut BTreeSet::new())
}
fn value(array: &ArrayRef, row: usize, refs: &mut BTreeSet<Reference>) -> Result<Value> {
    if array.is_null(row) {
        return Ok(Value::Null);
    }
    if let Some(record) = array.as_any().downcast_ref::<StructArray>() {
        let fields = record.fields();
        let names: Vec<_> = fields.iter().map(|f| f.name().as_str()).collect();
        if names == ["__gql_path_graph", "__gql_path_nodes", "__gql_path_edges"] {
            if let (Some(graphs), Some(nodes), Some(edges)) = (
                record
                    .column(0)
                    .as_any()
                    .downcast_ref::<datafusion::arrow::array::UInt64Array>(),
                record.column(1).as_any().downcast_ref::<ListArray>(),
                record.column(2).as_any().downcast_ref::<ListArray>(),
            ) {
                if !graphs.is_null(row) && !nodes.is_null(row) && !edges.is_null(row) {
                    let graph = graphs.value(row);
                    let nodes = nodes.value(row);
                    let edges = edges.value(row);
                    if let (Some(nodes), Some(edges)) = (
                        nodes
                            .as_any()
                            .downcast_ref::<datafusion::arrow::array::UInt64Array>(),
                        edges
                            .as_any()
                            .downcast_ref::<datafusion::arrow::array::UInt64Array>(),
                    ) {
                        let mut elements = Vec::new();
                        for (i, id) in nodes.values().iter().copied().enumerate() {
                            refs.insert((graph, 'n', id));
                            elements.push(json!({"element":"node", "id":identity(graph,'n',id)}));
                            if i < edges.len() {
                                let edge = edges.value(i);
                                refs.insert((graph, 'e', edge));
                                elements
                                    .push(json!({"element":"edge", "id":identity(graph,'e',edge)}));
                            }
                        }
                        return Ok(json!({"path":elements}));
                    }
                }
            }
        }
        if names
            == [
                "__gql_element_graph",
                "__gql_element_kind",
                "__gql_element_id",
            ]
            && fields[0].data_type() == &DataType::UInt64
            && fields[1].data_type() == &DataType::Utf8
            && fields[2].data_type() == &DataType::UInt64
        {
            let graph = display(record.column(0), row)?.parse::<u64>();
            let kind = display(record.column(1), row)?;
            let id = display(record.column(2), row)?.parse::<u64>();
            if let (Ok(graph), Ok(id), Some(kind)) = (graph, id, kind.chars().next()) {
                if matches!(kind, 'n' | 'e') {
                    refs.insert((graph, kind, id));
                    return Ok(json!({"element": if kind == 'n' {"node"} else {"edge"},
                        "id": identity(graph, kind, id)}));
                }
            }
        }
        let mut object = serde_json::Map::new();
        for (field, column) in fields.iter().zip(record.columns()) {
            object.insert(field.name().clone(), value(column, row, refs)?);
        }
        return Ok(Value::Object(object));
    }
    let list = if let Some(a) = array.as_any().downcast_ref::<ListArray>() {
        Some(a.value(row))
    } else if let Some(a) = array.as_any().downcast_ref::<LargeListArray>() {
        Some(a.value(row))
    } else {
        array
            .as_any()
            .downcast_ref::<FixedSizeListArray>()
            .map(|a| a.value(row))
    };
    if let Some(list) = list {
        return (0..list.len())
            .map(|i| value(&list, i, refs))
            .collect::<Result<Vec<_>>>()
            .map(Value::Array);
    }
    let text = display(array, row)?;
    Ok(match array.data_type() {
        DataType::Boolean => Value::Bool(text == "true"),
        DataType::Int8 | DataType::Int16 | DataType::Int32 | DataType::Int64 => text
            .parse::<i64>()
            .ok()
            .filter(|n| n.unsigned_abs() <= 9_007_199_254_740_991)
            .map_or(Value::String(text), |n| json!(n)),
        DataType::UInt8 | DataType::UInt16 | DataType::UInt32 | DataType::UInt64 => text
            .parse::<u64>()
            .ok()
            .filter(|n| *n <= 9_007_199_254_740_991)
            .map_or(Value::String(text), |n| json!(n)),
        DataType::Float16 | DataType::Float32 | DataType::Float64 => text
            .parse::<f64>()
            .ok()
            .filter(|n| n.is_finite())
            .map_or(Value::String(text), |n| json!(n)),
        _ => Value::String(text),
    })
}

pub(crate) async fn project(
    batches: &[RecordBatch],
    sources: &BTreeMap<ObjectId, Arc<GraphData>>,
    ctx: &SessionContext,
) -> Result<GraphProjection> {
    let mut refs = BTreeSet::new();
    let mut rows = 0;
    let total: usize = batches.iter().map(RecordBatch::num_rows).sum();
    for batch in batches {
        for row in 0..batch.num_rows().min(DISPLAY_ROWS.saturating_sub(rows)) {
            for column in batch.columns() {
                value(column, row, &mut refs)?;
            }
            rows += 1;
        }
    }
    let mut result = GraphProjection {
        truncated: total > DISPLAY_ROWS || refs.len() > MAX_REFERENCES,
        ..Default::default()
    };
    let refs: BTreeSet<_> = refs.into_iter().take(MAX_REFERENCES).collect();
    let mut resolved = BTreeSet::new();
    for (graph, data) in sources {
        let mut nodes: BTreeSet<_> = refs
            .iter()
            .filter(|(g, k, _)| g == graph && *k == 'n')
            .map(|(_, _, id)| *id)
            .collect();
        let edges: BTreeSet<_> = refs
            .iter()
            .filter(|(g, k, _)| g == graph && *k == 'e')
            .map(|(_, _, id)| *id)
            .collect();
        for edge in &data.edges {
            for batch in selected(&edge.table, &edges, ctx).await? {
                for row in 0..batch.num_rows() {
                    let id = unsigned(&batch, ID, row)?;
                    let source = unsigned(&batch, SOURCE, row)?;
                    let target = unsigned(&batch, DESTINATION, row)?;
                    nodes.extend([source, target]);
                    resolved.insert((*graph, 'e', id));
                    result.edges.push(GraphEdge {
                        id: identity(*graph, 'e', id),
                        source: identity(*graph, 'n', source),
                        target: identity(*graph, 'n', target),
                        directed: edge.directed,
                        labels: edge.table.labels.iter().cloned().collect(),
                        properties: properties(&batch, row)?,
                    });
                }
            }
        }
        for node in &data.nodes {
            for batch in selected(&node.0, &nodes, ctx).await? {
                for row in 0..batch.num_rows() {
                    let id = unsigned(&batch, ID, row)?;
                    resolved.insert((*graph, 'n', id));
                    result.nodes.push(GraphNode {
                        id: identity(*graph, 'n', id),
                        labels: node.0.labels.iter().cloned().collect(),
                        properties: properties(&batch, row)?,
                    });
                }
            }
        }
    }
    result.unresolved = refs.difference(&resolved).count();
    Ok(result)
}
async fn selected(
    table: &Table,
    ids: &BTreeSet<u64>,
    ctx: &SessionContext,
) -> Result<Vec<RecordBatch>> {
    match table.select_ids(ids)? {
        Some(provider) => Ok(ctx.read_table(provider)?.collect().await?),
        None => Ok(vec![]),
    }
}
fn unsigned(batch: &RecordBatch, name: &str, row: usize) -> Result<u64> {
    let column = batch.column_by_name(name).expect("validated graph column");
    Ok(column
        .as_any()
        .downcast_ref::<datafusion::arrow::array::UInt64Array>()
        .expect("validated graph ID")
        .value(row))
}
fn properties(batch: &RecordBatch, row: usize) -> Result<BTreeMap<String, Value>> {
    batch
        .schema()
        .fields()
        .iter()
        .zip(batch.columns())
        .filter(|(f, _)| ![ID, SOURCE, DESTINATION].contains(&f.name().as_str()))
        .map(|(f, c)| Ok((f.name().clone(), cell(c, row)?)))
        .collect()
}

fn display(array: &ArrayRef, row: usize) -> Result<String> {
    Ok(array_value_to_string(array, row).map_err(datafusion::error::DataFusionError::from)?)
}
