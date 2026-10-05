//! Snapshot-owned row positions. Arrow buffers remain immutable until sealing.
use crate::graph::{DESTINATION, ID, SOURCE};
use datafusion::arrow::{
    array::{BooleanArray, UInt64Array},
    compute::filter_record_batch,
    record_batch::RecordBatch,
};
use datafusion::parquet::arrow::arrow_reader::{RowSelection, RowSelector};
use std::{
    collections::{BTreeSet, HashMap},
    sync::Arc,
};

#[derive(Clone, Debug, Default)]
pub(crate) struct DeleteVector {
    words: Vec<u64>,
    count: usize,
}
impl DeleteVector {
    pub fn contains(&self, row: usize) -> bool {
        self.words
            .get(row / 64)
            .is_some_and(|word| word & (1 << (row % 64)) != 0)
    }
    pub fn insert(&mut self, row: usize) -> bool {
        if self.contains(row) {
            return false;
        }
        self.words.resize(self.words.len().max(row / 64 + 1), 0);
        self.words[row / 64] |= 1 << (row % 64);
        self.count += 1;
        true
    }
    pub fn len(&self) -> usize {
        self.count
    }
    pub fn positions(&self) -> impl Iterator<Item = usize> + '_ {
        self.words.iter().enumerate().flat_map(|(i, word)| {
            let mut bits = *word;
            std::iter::from_fn(move || {
                if bits == 0 {
                    return None;
                }
                let row = i * 64 + bits.trailing_zeros() as usize;
                bits &= bits - 1;
                Some(row)
            })
        })
    }
    pub fn filter(&self, batch: &RecordBatch) -> datafusion::error::Result<RecordBatch> {
        if self.count == 0 {
            return Ok(batch.clone());
        }
        let keep = BooleanArray::from(
            (0..batch.num_rows())
                .map(|row| !self.contains(row))
                .collect::<Vec<_>>(),
        );
        Ok(filter_record_batch(batch, &keep)?)
    }
    pub fn selection(&self, rows: usize) -> RowSelection {
        selection(rows, self.positions(), false)
    }
}

/// Positions must be sorted, unique and expressed in original file coordinates.
pub(crate) fn selection(
    rows: usize,
    positions: impl Iterator<Item = usize>,
    keep_positions: bool,
) -> RowSelection {
    let mut selectors = Vec::new();
    let mut cursor = 0;
    for row in positions {
        if row > cursor {
            selectors.push(RowSelector {
                row_count: row - cursor,
                skip: keep_positions,
            });
        }
        selectors.push(RowSelector {
            row_count: 1,
            skip: !keep_positions,
        });
        cursor = row + 1;
    }
    if cursor < rows {
        selectors.push(RowSelector {
            row_count: rows - cursor,
            skip: keep_positions,
        });
    }
    RowSelection::from(selectors)
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct RowLocation {
    pub batch: usize,
    pub row: usize,
}

/// Copy-on-write shards keep a one-row append/removal from cloning every ID.
#[derive(Clone, Debug, Default)]
pub(crate) struct IdIndex {
    shards: HashMap<u8, Arc<HashMap<u64, RowLocation>>>,
}
impl IdIndex {
    fn shard(id: u64) -> u8 {
        (id.wrapping_mul(0x9e3779b97f4a7c15) >> 56) as u8
    }
    pub fn get(&self, id: u64) -> Option<RowLocation> {
        self.shards.get(&Self::shard(id))?.get(&id).copied()
    }
    fn insert(&mut self, id: u64, location: RowLocation) {
        Arc::make_mut(self.shards.entry(Self::shard(id)).or_default()).insert(id, location);
    }
    fn remove(&mut self, id: u64) -> Option<RowLocation> {
        Arc::make_mut(self.shards.get_mut(&Self::shard(id))?).remove(&id)
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ResidentRows {
    pub index: Arc<IdIndex>,
    pub deleted: Vec<Arc<DeleteVector>>,
    incident: Vec<Arc<HashMap<u64, Vec<u64>>>>,
}
impl ResidentRows {
    pub fn new(batches: &[RecordBatch], deleted: Vec<Arc<DeleteVector>>) -> Self {
        let mut result = Self::default();
        for (i, batch) in batches.iter().enumerate() {
            result.append(batch, deleted.get(i).cloned().unwrap_or_default());
        }
        result
    }
    pub fn append(&mut self, batch: &RecordBatch, deleted: Arc<DeleteVector>) {
        let batch_index = self.deleted.len();
        let ids = column(batch, ID);
        let endpoints = batch
            .column_by_name(SOURCE)
            .map(|_| (column(batch, SOURCE), column(batch, DESTINATION)));
        let index = Arc::make_mut(&mut self.index);
        let mut incident: HashMap<u64, Vec<u64>> = HashMap::new();
        for (row, id) in ids.values().iter().copied().enumerate() {
            if deleted.contains(row) {
                continue;
            }
            index.insert(
                id,
                RowLocation {
                    batch: batch_index,
                    row,
                },
            );
            if let Some((source, destination)) = endpoints {
                incident.entry(source.value(row)).or_default().push(id);
                if source.value(row) != destination.value(row) {
                    incident.entry(destination.value(row)).or_default().push(id);
                }
            }
        }
        self.deleted.push(deleted);
        self.incident.push(Arc::new(incident));
    }
    pub fn remove(&mut self, id: u64) -> bool {
        if self.index.get(id).is_none() {
            return false;
        }
        let Some(location) = Arc::make_mut(&mut self.index).remove(id) else {
            return false;
        };
        Arc::make_mut(&mut self.deleted[location.batch]).insert(location.row)
    }
    pub fn incident_ids(&self, nodes: &BTreeSet<u64>) -> BTreeSet<u64> {
        self.incident
            .iter()
            .flat_map(|adjacency| {
                nodes
                    .iter()
                    .flat_map(|id| adjacency.get(id).into_iter().flatten().copied())
            })
            .filter(|id| self.index.get(*id).is_some())
            .collect()
    }
    /// Drop wholly dead batches, updating only locations in batches that moved.
    pub fn prune(&mut self, batches: &mut Vec<RecordBatch>) {
        let mut keep = 0;
        for old in 0..batches.len() {
            if self.deleted[old].len() == batches[old].num_rows() {
                continue;
            }
            if keep != old {
                for (row, id) in column(&batches[old], ID)
                    .values()
                    .iter()
                    .copied()
                    .enumerate()
                {
                    if !self.deleted[old].contains(row) {
                        Arc::make_mut(&mut self.index).insert(id, RowLocation { batch: keep, row });
                    }
                }
                batches.swap(keep, old);
                self.deleted.swap(keep, old);
                self.incident.swap(keep, old);
            }
            keep += 1;
        }
        batches.truncate(keep);
        self.deleted.truncate(keep);
        self.incident.truncate(keep);
    }
}

pub(crate) fn column<'a>(batch: &'a RecordBatch, name: &str) -> &'a UInt64Array {
    batch
        .column_by_name(name)
        .expect("validated graph column")
        .as_any()
        .downcast_ref()
        .expect("validated graph ID type")
}

#[derive(Debug)]
pub(crate) struct ResidentTable {
    pub schema: datafusion::arrow::datatypes::SchemaRef,
    pub batches: Vec<RecordBatch>,
    pub deleted: Vec<Arc<DeleteVector>>,
}
#[async_trait::async_trait]
impl datafusion::datasource::TableProvider for ResidentTable {
    fn schema(&self) -> datafusion::arrow::datatypes::SchemaRef {
        self.schema.clone()
    }
    fn table_type(&self) -> datafusion::logical_expr::TableType {
        datafusion::logical_expr::TableType::Base
    }
    async fn scan(
        &self,
        state: &dyn datafusion::catalog::Session,
        projection: Option<&Vec<usize>>,
        filters: &[datafusion::logical_expr::Expr],
        limit: Option<usize>,
    ) -> datafusion::error::Result<Arc<dyn datafusion::physical_plan::ExecutionPlan>> {
        use datafusion::datasource::MemTable;
        let schema = match projection {
            Some(p) => Arc::new(self.schema.project(p)?),
            None => self.schema.clone(),
        };
        let batches = self
            .batches
            .iter()
            .zip(&self.deleted)
            .map(|(batch, deleted)| {
                let batch = match projection {
                    Some(p) => batch.project(p)?,
                    None => batch.clone(),
                };
                deleted.filter(&batch)
            })
            .collect::<datafusion::error::Result<Vec<_>>>()?;
        MemTable::try_new(schema, vec![batches])?
            .scan(state, None, filters, limit)
            .await
    }
}
