//! Immutable managed Parquet files. Only the coordinator publishes their manifests.
use crate::{
    catalog::ObjectId,
    graph::{EdgeTable, GraphData, NodeTable, Table, DESTINATION, ID, SOURCE},
    persistence::{failpoint, sync_directory, Disk},
    transaction::PublishedState,
    Error, Result,
};
use datafusion::{
    arrow::datatypes::SchemaRef,
    datasource::{
        file_format::parquet::ParquetFormat,
        listing::{ListingOptions, ListingTable, ListingTableConfig, ListingTableUrl},
    },
    parquet::arrow::{arrow_reader::ParquetRecordBatchReaderBuilder, ArrowWriter},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    sync::Arc,
};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct GraphManifest {
    pub nodes: Vec<TableManifest>,
    pub edges: Vec<EdgeManifest>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct TableManifest {
    pub id: ObjectId,
    pub labels: BTreeSet<String>,
    pub schema: SchemaRef,
    pub rows: usize,
    pub bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct EdgeManifest {
    pub table: TableManifest,
    pub directed: bool,
}

impl TableManifest {
    fn name(&self) -> String {
        format!("graph-{}.parquet", self.id)
    }
    fn empty_table(&self, edge: bool) -> Result<Table> {
        Table::try_new(
            self.labels.iter().cloned().collect(),
            self.schema.clone(),
            vec![],
            if edge {
                &[ID, SOURCE, DESTINATION]
            } else {
                &[ID]
            },
        )
        .map_err(|e| Error::Corrupt(format!("invalid Parquet graph schema: {e}")))
    }
    fn open(&self, disk: &Disk, edge: bool) -> Result<Table> {
        let mut table = self.empty_table(edge)?;
        let path = disk.directory.join(self.name());
        let path = path
            .to_str()
            .ok_or_else(|| Error::UnsupportedFeature("non-UTF-8 database path".into()))?;
        table.provider = Arc::new(ListingTable::try_new(
            ListingTableConfig::new(ListingTableUrl::parse(path)?)
                .with_listing_options(ListingOptions::new(Arc::new(ParquetFormat::default())))
                .with_schema(self.schema.clone()),
        )?);
        table.row_count = self.rows;
        Ok(table)
    }
    fn check_file(&self, disk: &Disk) -> Result<()> {
        let path = disk.directory.join(self.name());
        let metadata = fs::symlink_metadata(&path)
            .map_err(|e| Error::Corrupt(format!("missing graph file {}: {e}", self.name())))?;
        if !metadata.is_file() || metadata.len() != self.bytes {
            return Err(Error::Corrupt(format!(
                "graph file changed: {}",
                self.name()
            )));
        }
        let reader = ParquetRecordBatchReaderBuilder::try_new(File::open(path)?)
            .map_err(|e| Error::Corrupt(format!("invalid graph file {}: {e}", self.name())))?;
        if reader.schema().as_ref() != self.schema.as_ref()
            || u64::try_from(reader.metadata().file_metadata().num_rows()).ok()
                != Some(self.rows as u64)
        {
            return Err(Error::Corrupt(format!(
                "graph file metadata differs: {}",
                self.name()
            )));
        }
        Ok(())
    }
}

impl GraphManifest {
    pub fn tables(&self) -> impl Iterator<Item = &TableManifest> {
        self.nodes.iter().chain(self.edges.iter().map(|e| &e.table))
    }
    pub fn validate(&self, next_id: ObjectId, files: &mut BTreeSet<ObjectId>) -> Result<()> {
        for table in self.tables() {
            if table.id < 3 || table.id >= next_id || table.bytes == 0 || !files.insert(table.id) {
                return Err(Error::Corrupt("invalid or shared Parquet file ID".into()));
            }
        }
        let nodes = self
            .nodes
            .iter()
            .map(|t| t.empty_table(false))
            .collect::<Result<Vec<_>>>()?;
        let edges = self
            .edges
            .iter()
            .map(|e| e.table.empty_table(true))
            .collect::<Result<Vec<_>>>()?;
        crate::graph::property_types(nodes.iter())?;
        crate::graph::property_types(edges.iter())?;
        Ok(())
    }
    pub fn open(&self, disk: &Disk) -> Result<Arc<GraphData>> {
        Ok(Arc::new(GraphData {
            nodes: self
                .nodes
                .iter()
                .map(|t| Ok(NodeTable(t.open(disk, false)?)))
                .collect::<Result<_>>()?,
            edges: self
                .edges
                .iter()
                .map(|e| {
                    Ok(EdgeTable {
                        table: e.table.open(disk, true)?,
                        directed: e.directed,
                    })
                })
                .collect::<Result<_>>()?,
        }))
    }
}

impl Disk {
    pub fn write_graph_table(&self, id: ObjectId, table: &Table) -> Result<TableManifest> {
        let path = self.directory.join(format!("graph-{id}.parquet"));
        let file = OpenOptions::new().write(true).create_new(true).open(path)?;
        let mut writer = ArrowWriter::try_new(file.try_clone()?, table.schema.clone(), None)
            .map_err(datafusion::error::DataFusionError::from)?;
        for batch in &table.batches {
            writer
                .write(batch)
                .map_err(datafusion::error::DataFusionError::from)?;
        }
        writer
            .close()
            .map_err(datafusion::error::DataFusionError::from)?;
        failpoint("parquet_write");
        file.sync_all()?;
        failpoint("parquet_sync");
        Ok(TableManifest {
            id,
            labels: table.labels.clone(),
            schema: table.schema.clone(),
            rows: table.row_count,
            bytes: file.metadata()?.len(),
        })
    }
    pub fn finish_graph_files(&self) -> Result<()> {
        sync_directory(&self.directory)?;
        failpoint("parquet_directory");
        Ok(())
    }
    pub fn check_graph_files(&self, state: &PublishedState) -> Result<()> {
        for generation in state.storage.generations.values() {
            if let Some(manifest) = &generation.parquet {
                for table in manifest.tables() {
                    table.check_file(self)?;
                }
            }
        }
        Ok(())
    }
    /// Called only after a durable checkpoint, under the exclusive lifecycle lease.
    pub fn reclaim_graph_files(&self, state: &PublishedState) {
        let live: BTreeSet<_> = state
            .storage
            .generations
            .values()
            .filter_map(|g| g.parquet.as_ref())
            .flat_map(|m| m.tables())
            .map(|t| t.name())
            .collect();
        let Ok(entries) = fs::read_dir(&self.directory) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            let managed = name
                .strip_prefix("graph-")
                .and_then(|s| s.strip_suffix(".parquet"))
                .and_then(|s| s.parse::<u64>().ok())
                .is_some_and(|id| name == format!("graph-{id}.parquet"));
            if managed && !live.contains(name) {
                let _ = fs::remove_file(entry.path());
                failpoint("parquet_cleanup");
            }
        }
        // Failed removals remain discoverable at the next checkpoint.
        let _ = sync_directory(&self.directory);
    }
}
