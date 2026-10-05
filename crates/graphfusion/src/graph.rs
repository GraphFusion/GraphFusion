//! Validated immutable Arrow tables for a property graph.
use crate::{
    rows::{DeleteVector, ResidentRows, ResidentTable},
    Error, Result,
};
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
    /// Immutable Parquet object ID, shared across versions with different row masks.
    pub file_id: Option<u64>,
    /// Stable WAL identity of a resident fragment across append/mask operations.
    pub memory_id: Option<u64>,
    pub deleted_ids: BTreeSet<u64>,
    /// Row removals from this transaction's resident base, replayed before new batches.
    pub memory_deleted_ids: BTreeSet<u64>,
    pub(crate) resident: ResidentRows,
    pub(crate) file_rows: Option<Arc<crate::parquet::FileRows>>,
    pub(crate) file_deleted: Arc<DeleteVector>,
    max_id: Option<u64>,
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
        Self::try_new_with_deletions(labels, schema, batches, reserved, vec![])
    }
    pub(crate) fn try_new_with_deletions(
        labels: Vec<String>,
        schema: SchemaRef,
        batches: Vec<RecordBatch>,
        reserved: &[&str],
        deleted: Vec<Arc<DeleteVector>>,
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
        let resident = ResidentRows::new(&batches, deleted);
        let max_id = batches
            .iter()
            .flat_map(|b| crate::rows::column(b, ID).values().iter().copied())
            .max();
        let provider: Arc<dyn TableProvider> = if resident.deleted.iter().any(|d| d.len() != 0) {
            Arc::new(ResidentTable {
                schema: schema.clone(),
                batches: batches.clone(),
                deleted: resident.deleted.clone(),
            })
        } else {
            Arc::new(MemTable::try_new(schema.clone(), vec![batches.clone()])?)
        };
        Ok(Self {
            labels: label_set,
            schema,
            row_count: batches
                .iter()
                .zip(&resident.deleted)
                .map(|(b, d)| b.num_rows() - d.len())
                .sum(),
            batches,
            provider,
            file_id: None,
            memory_id: None,
            deleted_ids: BTreeSet::new(),
            memory_deleted_ids: BTreeSet::new(),
            resident,
            file_rows: None,
            file_deleted: Arc::default(),
            max_id,
        })
    }

    pub(crate) fn append_rows(&mut self, extra: Table) -> Result<()> {
        if self.file_id.is_some() {
            return Err(Error::Corrupt(
                "cannot append to an unloaded graph table".into(),
            ));
        }
        if self.labels != extra.labels || self.schema.as_ref() != extra.schema.as_ref() {
            return Err(Error::InvalidDefinition(
                "incompatible graph table append".into(),
            ));
        }
        for (batch, deleted) in extra.batches.into_iter().zip(extra.resident.deleted) {
            self.resident.append(&batch, deleted);
            self.batches.push(batch);
        }
        self.memory_deleted_ids.extend(extra.memory_deleted_ids);
        self.row_count += extra.row_count;
        self.max_id = self.max_id.max(extra.max_id);
        self.refresh_resident_provider()?;
        Ok(())
    }
    /// Mark physical row positions without changing shared Arrow/Parquet data.
    pub(crate) fn mask_rows(&mut self, ids: &BTreeSet<u64>) -> Result<BTreeSet<u64>> {
        let mut removed = BTreeSet::new();
        if ids.is_empty() {
            return Ok(removed);
        }
        if let Some(file) = &self.file_rows {
            for (id, row) in file.locate(ids)? {
                if Arc::make_mut(&mut self.file_deleted).insert(row) {
                    self.deleted_ids.insert(id);
                    removed.insert(id);
                }
            }
            if !removed.is_empty() {
                self.provider = file.provider(self.file_deleted.clone(), None);
            }
        } else {
            for id in ids {
                if self.resident.remove(*id) {
                    self.memory_deleted_ids.insert(*id);
                    removed.insert(*id);
                }
            }
            if !removed.is_empty() {
                self.refresh_resident_provider()?;
            }
        }
        self.row_count = self
            .row_count
            .checked_sub(removed.len())
            .ok_or_else(|| Error::Corrupt("too many masked rows".into()))?;
        Ok(removed)
    }
    fn refresh_resident_provider(&mut self) -> Result<()> {
        self.provider = if self.resident.deleted.iter().any(|d| d.len() != 0) {
            Arc::new(ResidentTable {
                schema: self.schema.clone(),
                batches: self.batches.clone(),
                deleted: self.resident.deleted.clone(),
            })
        } else {
            Arc::new(MemTable::try_new(
                self.schema.clone(),
                vec![self.batches.clone()],
            )?)
        };
        Ok(())
    }
    pub(crate) fn prune_deleted_batches(&mut self) -> Result<()> {
        self.resident.prune(&mut self.batches);
        self.refresh_resident_provider()
    }
    pub(crate) fn visible_batches(&self) -> Result<Vec<RecordBatch>> {
        self.batches
            .iter()
            .zip(&self.resident.deleted)
            .map(|(batch, deleted)| deleted.filter(batch).map_err(Error::from))
            .collect()
    }
    /// Only selected rows are gathered; unaffected property buffers stay shared.
    pub(crate) fn select_ids(&self, ids: &BTreeSet<u64>) -> Result<Option<Arc<dyn TableProvider>>> {
        if let Some(file) = &self.file_rows {
            let mut positions: Vec<_> = file
                .locate(ids)?
                .into_iter()
                .map(|(_, row)| row)
                .filter(|row| !self.file_deleted.contains(*row))
                .collect();
            if positions.is_empty() {
                return Ok(None);
            }
            positions.sort_unstable();
            return Ok(Some(
                file.provider(self.file_deleted.clone(), Some(positions)),
            ));
        }
        use datafusion::arrow::compute::take;
        let mut positions: BTreeMap<usize, Vec<u64>> = BTreeMap::new();
        for id in ids {
            if let Some(location) = self.resident.index.get(*id) {
                positions
                    .entry(location.batch)
                    .or_default()
                    .push(location.row as u64);
            }
        }
        if positions.is_empty() {
            return Ok(None);
        }
        let batches = positions
            .into_iter()
            .map(|(batch, rows)| {
                let indices = UInt64Array::from(rows);
                let columns = self.batches[batch]
                    .columns()
                    .iter()
                    .map(|column| take(column, &indices, None))
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                RecordBatch::try_new(self.schema.clone(), columns)
            })
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(datafusion::error::DataFusionError::from)?;
        Ok(Some(Arc::new(MemTable::try_new(
            self.schema.clone(),
            vec![batches],
        )?)))
    }
    pub(crate) fn incident_ids(&self, nodes: &BTreeSet<u64>) -> Result<BTreeSet<u64>> {
        match &self.file_rows {
            Some(file) => file.incident_ids(nodes, &self.file_deleted),
            None => Ok(self.resident.incident_ids(nodes)),
        }
    }
    pub fn ids<'a>(&'a self, column: &'a str) -> impl Iterator<Item = u64> + 'a {
        self.batches
            .iter()
            .zip(&self.resident.deleted)
            .flat_map(move |(batch, deleted)| {
                crate::rows::column(batch, column)
                    .values()
                    .iter()
                    .copied()
                    .enumerate()
                    .filter_map(move |(row, id)| (!deleted.contains(row)).then_some(id))
            })
    }
}

#[derive(Clone, Debug, Default)]
pub struct GraphData {
    pub(crate) nodes: Vec<NodeTable>,
    pub(crate) edges: Vec<EdgeTable>,
}
impl GraphData {
    pub(crate) fn next_element_id(&self) -> Option<u64> {
        self.nodes
            .iter()
            .map(|t| t.0.max_id)
            .chain(self.edges.iter().map(|t| t.table.max_id))
            .flatten()
            .max()
            .map_or(Some(0), |id| id.checked_add(1))
    }

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
        Self::from_mutation(nodes, edges)
    }
    /// Mutations preserve IDs/endpoints from a validated snapshot. Only property
    /// layouts and newly grouped resident fragments need revalidation here.
    pub(crate) fn from_mutation(nodes: Vec<NodeTable>, edges: Vec<EdgeTable>) -> Result<Self> {
        property_types(nodes.iter().map(|t| &t.0))?;
        property_types(edges.iter().map(|t| &t.table))?;
        // Keep one resident fragment per layout, even after repeated row updates.
        let mut compact_nodes: Vec<NodeTable> = Vec::new();
        for node in nodes {
            if node.0.file_id.is_none() {
                if let Some(existing) = compact_nodes.iter_mut().find(|t| {
                    t.0.file_id.is_none()
                        && t.0.labels == node.0.labels
                        && t.0.schema == node.0.schema
                }) {
                    existing.0.append_rows(node.0)?;
                    continue;
                }
            }
            compact_nodes.push(node);
        }
        let mut compact_edges: Vec<EdgeTable> = Vec::new();
        for edge in edges {
            if edge.table.file_id.is_none() {
                if let Some(existing) = compact_edges.iter_mut().find(|e| {
                    e.table.file_id.is_none()
                        && e.directed == edge.directed
                        && e.table.labels == edge.table.labels
                        && e.table.schema == edge.table.schema
                }) {
                    existing.table.append_rows(edge.table)?;
                    continue;
                }
            }
            compact_edges.push(edge);
        }
        Ok(Self {
            nodes: compact_nodes,
            edges: compact_edges,
        })
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
