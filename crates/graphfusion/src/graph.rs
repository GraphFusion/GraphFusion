//! Validated immutable Arrow tables for a property graph.
use crate::{Error, Result};
use datafusion::{
    arrow::{
        array::UInt64Array,
        datatypes::{DataType, SchemaRef},
        record_batch::RecordBatch,
    },
    datasource::{MemTable, TableProvider},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    path::Path,
    sync::Arc,
};

pub const ID: &str = "__gf_id";
pub const SOURCE: &str = "__gf_source";
pub const DESTINATION: &str = "__gf_destination";
pub(crate) const FROM: &str = "__gf_from";
pub(crate) const TO: &str = "__gf_to";

#[derive(Clone, Debug)]
pub struct NodeTable(pub(crate) Table);
#[derive(Clone, Debug)]
pub struct EdgeTable {
    pub(crate) table: Table,
    pub(crate) directed: bool,
}
#[derive(Clone, Debug)]
pub(crate) struct Table {
    pub labels: BTreeSet<String>,
    pub schema: SchemaRef,
    pub batches: Vec<RecordBatch>,
    pub provider: Arc<dyn TableProvider>,
    pub row_count: usize,
}
impl NodeTable {
    /// Reads an external Parquet table and validates it for a subsequent graph import.
    pub fn read_parquet(labels: Vec<String>, path: impl AsRef<Path>) -> Result<Self> {
        let (schema, batches) = read_parquet(path.as_ref())?;
        Self::try_new(labels, schema, batches)
    }
    /// Rows share a label set and property layout. IDs must be non-null UInt64.
    pub fn try_new(
        labels: Vec<String>,
        schema: SchemaRef,
        batches: Vec<RecordBatch>,
    ) -> Result<Self> {
        Ok(Self(Table::try_new(labels, schema, batches, &[ID])?))
    }
}
impl EdgeTable {
    pub fn read_parquet(
        labels: Vec<String>,
        directed: bool,
        path: impl AsRef<Path>,
    ) -> Result<Self> {
        let (schema, batches) = read_parquet(path.as_ref())?;
        Self::try_new(labels, directed, schema, batches)
    }
    /// Undirected edges use the same endpoints; their ordering is immaterial.
    pub fn try_new(
        labels: Vec<String>,
        directed: bool,
        schema: SchemaRef,
        batches: Vec<RecordBatch>,
    ) -> Result<Self> {
        Ok(Self {
            table: Table::try_new(labels, schema, batches, &[ID, SOURCE, DESTINATION])?,
            directed,
        })
    }
}

fn read_parquet(path: &Path) -> Result<(SchemaRef, Vec<RecordBatch>)> {
    use datafusion::parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
    let reader = ParquetRecordBatchReaderBuilder::try_new(File::open(path)?)
        .map_err(datafusion::error::DataFusionError::from)?;
    let schema = reader.schema().clone();
    let batches = reader
        .build()
        .map_err(datafusion::error::DataFusionError::from)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(datafusion::error::DataFusionError::from)?;
    Ok((schema, batches))
}
impl Table {
    pub(crate) fn try_new(
        labels: Vec<String>,
        schema: SchemaRef,
        batches: Vec<RecordBatch>,
        reserved: &[&str],
    ) -> Result<Self> {
        let label_set: BTreeSet<_> = labels.iter().cloned().collect();
        if label_set.len() != labels.len() || labels.iter().any(String::is_empty) {
            return Err(Error::InvalidDefinition(
                "duplicate or empty element label".into(),
            ));
        }
        let mut names = BTreeSet::new();
        for field in schema.fields() {
            if !names.insert(field.name()) || field.name().is_empty() {
                return Err(Error::InvalidDefinition(
                    "duplicate or empty table column".into(),
                ));
            }
            if field.name().starts_with("__gf_") && !reserved.contains(&field.name().as_str()) {
                return Err(Error::InvalidDefinition(
                    "reserved graph storage column".into(),
                ));
            }
            if !reserved.contains(&field.name().as_str())
                && !matches!(
                    field.data_type(),
                    DataType::Boolean
                        | DataType::Int64
                        | DataType::Float64
                        | DataType::Utf8
                        | DataType::Binary
                )
            {
                return Err(Error::UnsupportedFeature(format!(
                    "graph property type {}",
                    field.data_type()
                )));
            }
        }
        for name in reserved {
            let field = schema
                .field_with_name(name)
                .map_err(|_| Error::InvalidDefinition(format!("missing graph column {name}")))?;
            if field.data_type() != &DataType::UInt64 || field.is_nullable() {
                return Err(Error::InvalidDefinition(format!(
                    "{name} must be non-null UInt64"
                )));
            }
        }
        for batch in &batches {
            if batch.schema().as_ref() != schema.as_ref() {
                return Err(Error::InvalidDefinition(
                    "graph batch schema differs from table schema".into(),
                ));
            }
            for (field, column) in schema.fields().iter().zip(batch.columns()) {
                if !field.is_nullable() && column.null_count() != 0 {
                    return Err(Error::InvalidDefinition(format!(
                        "null in required column {}",
                        field.name()
                    )));
                }
            }
        }
        let provider = Arc::new(MemTable::try_new(schema.clone(), vec![batches.clone()])?);
        Ok(Self {
            labels: label_set,
            schema,
            row_count: batches.iter().map(RecordBatch::num_rows).sum(),
            batches,
            provider,
        })
    }
    pub fn ids<'a>(&'a self, column: &'a str) -> impl Iterator<Item = u64> + 'a {
        self.batches.iter().flat_map(move |batch| {
            batch
                .column_by_name(column)
                .expect("validated graph schema")
                .as_any()
                .downcast_ref::<UInt64Array>()
                .expect("validated graph ID type")
                .values()
                .iter()
                .copied()
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct GraphData {
    pub(crate) nodes: Vec<NodeTable>,
    pub(crate) edges: Vec<EdgeTable>,
}
impl GraphData {
    /// Validates identity and endpoints across all tables. Each node is stored once.
    pub fn try_new(nodes: Vec<NodeTable>, edges: Vec<EdgeTable>) -> Result<Self> {
        let mut node_ids = BTreeSet::new();
        for table in &nodes {
            for id in table.0.ids(ID) {
                if !node_ids.insert(id) {
                    return Err(Error::InvalidDefinition(format!("duplicate node ID {id}")));
                }
            }
        }
        let mut edge_ids = BTreeSet::new();
        for table in &edges {
            for id in table.table.ids(ID) {
                if !edge_ids.insert(id) {
                    return Err(Error::InvalidDefinition(format!("duplicate edge ID {id}")));
                }
            }
            for endpoint in [SOURCE, DESTINATION] {
                for id in table.table.ids(endpoint) {
                    if !node_ids.contains(&id) {
                        return Err(Error::InvalidDefinition(format!(
                            "edge endpoint {id} does not exist"
                        )));
                    }
                }
            }
        }
        property_types(nodes.iter().map(|t| &t.0))?;
        property_types(edges.iter().map(|t| &t.table))?;
        Ok(Self { nodes, edges })
    }
    pub fn node_count(&self) -> usize {
        self.nodes.iter().map(|t| t.0.row_count).sum()
    }
    pub fn edge_count(&self) -> usize {
        self.edges.iter().map(|t| t.table.row_count).sum()
    }
}
pub(crate) fn property_types<'a>(
    tables: impl Iterator<Item = &'a Table>,
) -> Result<BTreeMap<String, DataType>> {
    let mut types = BTreeMap::new();
    for table in tables {
        for field in table.schema.fields() {
            if field.name().starts_with("__gf_") {
                continue;
            }
            if let Some(previous) = types.insert(field.name().clone(), field.data_type().clone()) {
                if previous != *field.data_type() {
                    return Err(Error::UnsupportedFeature(format!(
                        "mixed types for graph property {}",
                        field.name()
                    )));
                }
            }
        }
    }
    Ok(types)
}
