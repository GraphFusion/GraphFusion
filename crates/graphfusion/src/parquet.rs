//! Immutable managed Parquet files. Only the coordinator publishes their manifests.
use crate::{
    catalog::ObjectId,
    graph::{EdgeTable, GraphData, NodeTable, Table, DESTINATION, ID, SOURCE},
    memtable::Fragment,
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
    collections::{BTreeSet, HashMap},
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};

fn table_url(path: &Path) -> Result<ListingTableUrl> {
    // Plain paths are interpreted as glob patterns by ListingTableUrl::parse.
    let url = url::Url::from_file_path(path)
        .map_err(|_| Error::UnsupportedFeature("invalid Parquet database path".into()))?;
    ListingTableUrl::try_new(url, None).map_err(|error| {
        Error::UnsupportedFeature(format!("unscannable Parquet database path: {error}"))
    })
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct GraphManifest {
    pub nodes: Vec<Fragment>,
    pub edges: Vec<EdgeManifest>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct TableManifest {
    pub id: ObjectId,
    pub labels: BTreeSet<String>,
    pub schema: SchemaRef,
    pub rows: usize,
    pub bytes: u64,
    pub deleted_ids: BTreeSet<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index_bytes: Option<u64>,
    #[serde(skip)]
    file: Arc<OnceLock<Arc<FileRows>>>,
    #[serde(skip)]
    pub(crate) cache: Arc<OnceLock<Table>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct EdgeManifest {
    pub table: Fragment,
    pub directed: bool,
}

impl TableManifest {
    fn name(&self) -> String {
        format!("graph-{}.parquet", self.id)
    }
    fn index_name(&self) -> String {
        format!("graph-{}.index", self.id)
    }
    pub(crate) fn empty_table(&self, edge: bool) -> Result<Table> {
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
    pub(crate) fn open(&self, disk: &Disk, edge: bool) -> Result<Table> {
        if let Some(table) = self.cache.get() {
            return Ok(table.clone());
        }
        let mut table = self.empty_table(edge)?;
        let path = disk.directory.join(self.name());
        if self.file.get().is_none() {
            let base = Arc::new(ListingTable::try_new(
                ListingTableConfig::new(table_url(&path)?)
                    .with_listing_options(ListingOptions::new(Arc::new(ParquetFormat::default())))
                    .with_schema(self.schema.clone()),
            )?);
            let _ = self.file.set(Arc::new(FileRows {
                path,
                index_file: self
                    .index_bytes
                    .map(|bytes| (disk.directory.join(self.index_name()), bytes)),
                base,
                rows: self.rows,
                edge,
                index: OnceLock::new(),
            }));
        }
        let file = self.file.get().unwrap().clone();
        table.provider = file.base.clone();
        table.file_rows = Some(file);
        table.row_count = self.rows;
        table.file_id = Some(self.id);
        table.mask_rows(&self.deleted_ids)?;
        let _ = self.cache.set(table);
        Ok(self.cache.get().unwrap().clone())
    }
    pub(crate) fn with_deletes(&self, ids: &BTreeSet<u64>) -> Result<Self> {
        let mut next = self.clone();
        next.deleted_ids.extend(ids.iter().copied());
        next.cache = Arc::default();
        if let Some(table) = self.cache.get() {
            let mut table = table.clone();
            table.mask_rows(ids)?;
            let _ = next.cache.set(table);
        }
        Ok(next)
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
        if let Some(bytes) = self.index_bytes {
            crate::row_index::read(
                &disk.directory.join(self.index_name()),
                bytes,
                self.rows,
                self.schema.column_with_name(SOURCE).is_some(),
            )?;
        }
        Ok(())
    }
}

impl Fragment {
    fn empty_table(&self, edge: bool) -> Result<Table> {
        match self {
            Self::Parquet(t) => t.empty_table(edge),
            Self::Memory(t) => Table::try_new(
                t.labels.iter().cloned().collect(),
                t.data.schema.clone(),
                vec![],
                if edge {
                    &[ID, SOURCE, DESTINATION]
                } else {
                    &[ID]
                },
            ),
        }
    }
    fn open(&self, disk: &Disk, edge: bool) -> Result<Table> {
        match self {
            Self::Parquet(t) => t.open(disk, edge),
            Self::Memory(t) => t.open(edge),
        }
    }
}
impl GraphManifest {
    pub fn fragments(&self) -> impl Iterator<Item = &Fragment> {
        self.nodes.iter().chain(self.edges.iter().map(|e| &e.table))
    }
    pub fn tables(&self) -> impl Iterator<Item = &TableManifest> {
        self.fragments().filter_map(|f| match f {
            Fragment::Parquet(t) => Some(t),
            Fragment::Memory(_) => None,
        })
    }
    pub fn validate(&self, next_id: ObjectId, files: &mut BTreeSet<ObjectId>) -> Result<()> {
        for fragment in self.fragments() {
            if fragment.id() < 3 || fragment.id() >= next_id || !files.insert(fragment.id()) {
                return Err(Error::Corrupt("invalid or shared graph fragment ID".into()));
            }
            if matches!(fragment, Fragment::Parquet(t) if t.bytes == 0 || t.deleted_ids.len() > t.rows)
            {
                return Err(Error::Corrupt("empty Parquet file".into()));
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
        // Reject paths the query provider cannot read before staging or committing data.
        table_url(&path)?;
        let file = OpenOptions::new().write(true).create_new(true).open(path)?;
        let mut writer = ArrowWriter::try_new(file.try_clone()?, table.schema.clone(), None)
            .map_err(datafusion::error::DataFusionError::from)?;
        let batches = table.visible_batches()?;
        for batch in &batches {
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
        let index_bytes = crate::row_index::write(
            &self.directory.join(format!("graph-{id}.index")),
            &batches,
            table.schema.column_with_name(SOURCE).is_some(),
        )?;
        Ok(TableManifest {
            id,
            labels: table.labels.clone(),
            schema: table.schema.clone(),
            rows: table.row_count,
            bytes: file.metadata()?.len(),
            deleted_ids: BTreeSet::new(),
            index_bytes: Some(index_bytes),
            file: Arc::default(),
            cache: Arc::default(),
        })
    }
    /// Called under the coordinator/commit lock before installing a checkpoint.
    /// IDs are reserved in the old WAL first so a failed seal never reuses a file name.
    pub fn seal_memtables(&self, state: &mut PublishedState) -> Result<()> {
        let mut next_id = state.next_id;
        for generation in Arc::make_mut(&mut state.storage).generations.values_mut() {
            if let Some(manifest) = &mut generation.parquet {
                let manifest = Arc::make_mut(manifest);
                for (fragment, edge) in manifest
                    .nodes
                    .iter_mut()
                    .map(|f| (f, false))
                    .chain(manifest.edges.iter_mut().map(|e| (&mut e.table, true)))
                {
                    if let Fragment::Memory(table) = fragment {
                        let loaded = table.open(edge)?;
                        *fragment = Fragment::Parquet(self.write_graph_table(next_id, &loaded)?);
                        next_id = next_id.checked_add(1).ok_or_else(|| {
                            Error::InvalidDefinition("object IDs exhausted".into())
                        })?;
                    }
                }
            }
        }
        state.next_id = next_id;
        self.finish_graph_files()?;
        state.validate()
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
                for fragment in &manifest.nodes {
                    if let Fragment::Memory(t) = fragment {
                        t.open(false)?;
                    }
                }
                for edge in &manifest.edges {
                    if let Fragment::Memory(t) = &edge.table {
                        t.open(true)?;
                    }
                }
            }
        }
        Ok(())
    }
    /// Called only after a durable checkpoint, under the exclusive lifecycle lease.
    pub fn reclaim_graph_files(&self, state: &PublishedState) -> Result<()> {
        let mut live: BTreeSet<_> = state
            .storage
            .generations
            .values()
            .filter_map(|g| g.parquet.as_ref())
            .flat_map(|m| m.tables())
            .flat_map(|t| [t.name(), t.index_name()])
            .collect();
        live.extend(self.branch_data_files()?);
        let Ok(entries) = fs::read_dir(&self.directory) else {
            return Ok(());
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            let managed = ["parquet", "index"].iter().any(|extension| {
                name.strip_prefix("graph-")
                    .and_then(|s| s.strip_suffix(&format!(".{extension}")))
                    .and_then(|s| s.parse::<u64>().ok())
                    .is_some_and(|id| name == format!("graph-{id}.{extension}"))
            });
            if managed && !live.contains(name) {
                let _ = fs::remove_file(entry.path());
                failpoint("parquet_cleanup");
            }
        }
        // Failed removals remain discoverable at the next checkpoint.
        let _ = sync_directory(&self.directory);
        Ok(())
    }
}

/// A shared, lazy index over only identity and endpoint columns of an immutable file.
/// Index lifetime is independent of snapshot-specific deletion vectors.
#[derive(Debug)]
pub(crate) struct FileRows {
    path: PathBuf,
    index_file: Option<(PathBuf, u64)>,
    base: Arc<ListingTable>,
    rows: usize,
    edge: bool,
    index: OnceLock<FileIndex>,
}
#[derive(Debug, Default)]
struct FileIndex {
    ids: Vec<(u64, usize)>,
    endpoints: Vec<(u64, u64, u64)>,
    incident: OnceLock<HashMap<u64, Vec<u64>>>,
}
impl FileIndex {
    fn adjacency(&self) -> &HashMap<u64, Vec<u64>> {
        self.incident.get_or_init(|| {
            let mut incident: HashMap<u64, Vec<u64>> = HashMap::new();
            for &(id, source, destination) in &self.endpoints {
                incident.entry(source).or_default().push(id);
                if source != destination {
                    incident.entry(destination).or_default().push(id);
                }
            }
            incident
        })
    }
    fn position(&self, id: u64) -> Option<usize> {
        self.ids
            .binary_search_by_key(&id, |(id, _)| *id)
            .ok()
            .map(|i| self.ids[i].1)
    }
}
impl FileRows {
    fn index(&self) -> Result<&FileIndex> {
        if self.index.get().is_none() {
            if let Some((path, bytes)) = &self.index_file {
                let entries = crate::row_index::read(path, *bytes, self.rows, self.edge)?;
                let mut index = FileIndex::default();
                for entry in entries {
                    index.ids.push((entry.id, entry.position));
                    if self.edge {
                        index
                            .endpoints
                            .push((entry.id, entry.source, entry.destination));
                    }
                }
                let _ = self.index.set(index);
                return Ok(self.index.get().unwrap());
            }
            use datafusion::parquet::arrow::ProjectionMask;
            let reader = ParquetRecordBatchReaderBuilder::try_new(File::open(&self.path)?)
                .map_err(datafusion::error::DataFusionError::from)?;
            let columns = if self.edge {
                vec![ID, SOURCE, DESTINATION]
            } else {
                vec![ID]
            };
            let projection = ProjectionMask::roots(
                reader.parquet_schema(),
                columns
                    .iter()
                    .map(|name| reader.schema().index_of(name))
                    .collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(datafusion::error::DataFusionError::from)?,
            );
            let reader = reader
                .with_projection(projection)
                .build()
                .map_err(datafusion::error::DataFusionError::from)?;
            let mut index = FileIndex::default();
            let mut position = 0;
            for batch in reader {
                let batch = batch.map_err(datafusion::error::DataFusionError::from)?;
                let ids = crate::rows::column(&batch, ID);
                for row in 0..batch.num_rows() {
                    let id = ids.value(row);
                    index.ids.push((id, position + row));
                    if self.edge {
                        let source = crate::rows::column(&batch, SOURCE).value(row);
                        let destination = crate::rows::column(&batch, DESTINATION).value(row);
                        index.endpoints.push((id, source, destination));
                    }
                }
                position += batch.num_rows();
            }
            if position != self.rows {
                return Err(Error::Corrupt("immutable graph row count changed".into()));
            }
            index.ids.sort_unstable_by_key(|(id, _)| *id);
            if index.ids.windows(2).any(|rows| rows[0].0 == rows[1].0) {
                return Err(Error::Corrupt(
                    "duplicate ID in immutable graph file".into(),
                ));
            }
            let _ = self.index.set(index);
        }
        Ok(self.index.get().unwrap())
    }
    pub(crate) fn locate(&self, ids: &BTreeSet<u64>) -> Result<Vec<(u64, usize)>> {
        let index = self.index()?;
        Ok(ids
            .iter()
            .filter_map(|id| index.position(*id).map(|row| (*id, row)))
            .collect())
    }
    pub(crate) fn incident_ids(
        &self,
        nodes: &BTreeSet<u64>,
        deleted: &crate::rows::DeleteVector,
    ) -> Result<BTreeSet<u64>> {
        let index = self.index()?;
        let incident = index.adjacency();
        Ok(nodes
            .iter()
            .flat_map(|node| incident.get(node).into_iter().flatten().copied())
            .filter(|id| !deleted.contains(index.position(*id).expect("indexed incident edge")))
            .collect())
    }
    pub(crate) fn provider(
        self: &Arc<Self>,
        deleted: Arc<crate::rows::DeleteVector>,
        positions: Option<Vec<usize>>,
    ) -> Arc<dyn datafusion::datasource::TableProvider> {
        Arc::new(SelectedParquet {
            file: self.clone(),
            deleted,
            positions,
        })
    }
}

#[derive(Debug)]
struct SelectedParquet {
    file: Arc<FileRows>,
    deleted: Arc<crate::rows::DeleteVector>,
    positions: Option<Vec<usize>>,
}
#[async_trait::async_trait]
impl datafusion::datasource::TableProvider for SelectedParquet {
    fn schema(&self) -> SchemaRef {
        self.file.base.schema()
    }
    fn table_type(&self) -> datafusion::logical_expr::TableType {
        datafusion::logical_expr::TableType::Base
    }
    fn supports_filters_pushdown(
        &self,
        filters: &[&datafusion::logical_expr::Expr],
    ) -> datafusion::error::Result<Vec<datafusion::logical_expr::TableProviderFilterPushDown>> {
        self.file.base.supports_filters_pushdown(filters)
    }
    async fn scan(
        &self,
        state: &dyn datafusion::catalog::Session,
        projection: Option<&Vec<usize>>,
        filters: &[datafusion::logical_expr::Expr],
        limit: Option<usize>,
    ) -> datafusion::error::Result<Arc<dyn datafusion::physical_plan::ExecutionPlan>> {
        use datafusion::datasource::physical_plan::parquet::ParquetRowSelection;
        let selection = match &self.positions {
            Some(positions) => crate::rows::selection(
                self.file.rows,
                positions
                    .iter()
                    .copied()
                    .filter(|row| !self.deleted.contains(*row)),
                true,
            ),
            None => self.deleted.selection(self.file.rows),
        };
        let plan = self
            .file
            .base
            .scan(state, projection, filters, limit)
            .await?;
        attach_selection(plan, &ParquetRowSelection::new(selection), &self.schema())
    }
}

fn attach_selection(
    plan: Arc<dyn datafusion::physical_plan::ExecutionPlan>,
    selection: &datafusion::datasource::physical_plan::parquet::ParquetRowSelection,
    schema: &SchemaRef,
) -> datafusion::error::Result<Arc<dyn datafusion::physical_plan::ExecutionPlan>> {
    use datafusion::datasource::{
        physical_plan::{FileGroup, FileScanConfig, FileScanConfigBuilder},
        source::DataSourceExec,
    };
    if let Some(scan) = plan.downcast_ref::<DataSourceExec>() {
        let config = scan
            .data_source()
            .downcast_ref::<FileScanConfig>()
            .ok_or_else(|| {
                datafusion::error::DataFusionError::Internal("expected Parquet file scan".into())
            })?;
        let groups = config
            .file_groups
            .iter()
            .map(|group| {
                FileGroup::new(
                    group
                        .files()
                        .iter()
                        .cloned()
                        .map(|mut file| {
                            // Raw-file statistics cannot describe a snapshot after row selection.
                            file.statistics = None;
                            file.with_extension(selection.clone())
                        })
                        .collect(),
                )
            })
            .collect();
        let config = FileScanConfigBuilder::from(config.clone())
            .with_file_groups(groups)
            .with_statistics(datafusion::common::Statistics::new_unknown(schema))
            .build();
        return Ok(Arc::new(scan.clone().with_data_source(Arc::new(config))));
    }
    if plan.children().is_empty() {
        return Ok(plan);
    }
    let children = plan
        .children()
        .into_iter()
        .map(|child| attach_selection(child.clone(), selection, schema))
        .collect::<datafusion::error::Result<Vec<_>>>()?;
    use datafusion::physical_plan::execution_plan::{
        ChildrenPropertiesMode, ReplaceChildrenOptions,
    };
    plan.replace_children(
        children,
        ReplaceChildrenOptions::new(ChildrenPropertiesMode::Recompute),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rows::DeleteVector;
    use datafusion::{
        arrow::{
            array::{Int64Array, UInt64Array},
            datatypes::{DataType, Field, Schema},
            record_batch::RecordBatch,
            util::display::array_value_to_string,
        },
        dataframe::DataFrame,
        execution::context::SessionContext,
        functions_aggregate::{count::count, sum::sum},
        logical_expr::{col, lit},
        parquet::file::properties::WriterProperties,
    };

    struct Directory(PathBuf);
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    async fn value(data: DataFrame) -> String {
        let batches = data.collect().await.unwrap();
        array_value_to_string(batches[0].column(0), 0).unwrap()
    }

    #[tokio::test]
    async fn physical_row_selection_preserves_projection_filters_counts_and_limits() {
        let directory = Directory(std::env::temp_dir().join(format!(
            "graphfusion-row-selection-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )));
        fs::create_dir(&directory.0).unwrap();
        let path = directory.0.join("graph.parquet");
        let schema = Arc::new(Schema::new(vec![
            Field::new(ID, DataType::UInt64, false),
            Field::new("v", DataType::Int64, true),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(UInt64Array::from(vec![90, 10, 70, 30, 80, 20, 60, 40, 50])),
                Arc::new(Int64Array::from(vec![
                    Some(90),
                    None,
                    Some(70),
                    Some(30),
                    Some(80),
                    Some(20),
                    None,
                    Some(40),
                    Some(50),
                ])),
            ],
        )
        .unwrap();
        let properties = WriterProperties::builder()
            .set_max_row_group_row_count(Some(3))
            .build();
        let mut writer = ArrowWriter::try_new(
            File::create(&path).unwrap(),
            schema.clone(),
            Some(properties),
        )
        .unwrap();
        writer.write(&batch).unwrap();
        writer.close().unwrap();
        let index_path = directory.0.join("graph.index");
        let bytes = crate::row_index::write(&index_path, &[batch], false).unwrap();
        // Exercise both old v4 files and newly sealed files across three row groups.
        for index_file in [None, Some((index_path.clone(), bytes))] {
            let base = Arc::new(
                ListingTable::try_new(
                    ListingTableConfig::new(table_url(&path).unwrap())
                        .with_listing_options(ListingOptions::new(Arc::new(
                            ParquetFormat::default(),
                        )))
                        .with_schema(schema.clone()),
                )
                .unwrap(),
            );
            let file = Arc::new(FileRows {
                path: path.clone(),
                index_file,
                base,
                rows: 9,
                edge: false,
                index: OnceLock::new(),
            });
            assert_eq!(
                file.locate(&BTreeSet::from([10, 30, 90])).unwrap(),
                vec![(10, 1), (30, 3), (90, 0)]
            );
            let mut deleted = DeleteVector::default();
            for position in [0, 1, 4, 8] {
                deleted.insert(position);
            }
            let ctx = SessionContext::new();
            ctx.register_table("old", file.base.clone()).unwrap();
            ctx.register_table("live", file.provider(Arc::new(deleted.clone()), None))
                .unwrap();
            ctx.register_table(
                "chosen",
                file.provider(Arc::new(deleted), Some(vec![1, 2, 3, 5, 8])),
            )
            .unwrap();
            let old = ctx.table("old").await.unwrap();
            let live = ctx.table("live").await.unwrap();
            let chosen = ctx.table("chosen").await.unwrap();
            assert_eq!(old.count().await.unwrap(), 9);
            assert_eq!(live.clone().count().await.unwrap(), 5);
            assert_eq!(
                value(
                    live.clone()
                        .aggregate(vec![], vec![count(col("v"))])
                        .unwrap()
                )
                .await,
                "4"
            );
            assert_eq!(
                value(live.clone().aggregate(vec![], vec![sum(col("v"))]).unwrap()).await,
                "160"
            );
            let filtered = live.clone().filter(col(ID).gt_eq(lit(60u64))).unwrap();
            assert_eq!(filtered.clone().count().await.unwrap(), 2);
            assert_eq!(
                value(filtered.aggregate(vec![], vec![sum(col("v"))]).unwrap()).await,
                "70"
            );
            assert_eq!(
                live.select(vec![col("v")])
                    .unwrap()
                    .limit(0, Some(2))
                    .unwrap()
                    .count()
                    .await
                    .unwrap(),
                2
            );
            assert_eq!(
                value(
                    chosen
                        .clone()
                        .aggregate(vec![], vec![sum(col("v"))])
                        .unwrap()
                )
                .await,
                "120"
            );
            assert_eq!(chosen.count().await.unwrap(), 3);
        }
    }
}
