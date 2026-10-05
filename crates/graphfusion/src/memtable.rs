//! Immutable resident fragments and replayable Arrow IPC deltas.
use crate::{catalog::ObjectId, graph::Table, parquet::TableManifest, Error, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use datafusion::arrow::{
    datatypes::SchemaRef,
    ipc::{reader::StreamReader, writer::StreamWriter},
    record_batch::RecordBatch,
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{
    collections::BTreeSet,
    io::Cursor,
    sync::{Arc, OnceLock},
};

#[derive(Clone, Debug)]
pub(crate) struct Batches {
    pub schema: SchemaRef,
    pub batches: Vec<RecordBatch>,
    /// Runtime masks are encoded as live rows when a full IPC image is needed.
    pub deleted: Vec<Arc<crate::rows::DeleteVector>>,
}
impl Serialize for Batches {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let mut bytes = Vec::new();
        {
            let mut writer = StreamWriter::try_new(&mut bytes, &self.schema)
                .map_err(serde::ser::Error::custom)?;
            for (i, batch) in self.batches.iter().enumerate() {
                let batch = match self.deleted.get(i) {
                    Some(deleted) => deleted.filter(batch).map_err(serde::ser::Error::custom)?,
                    None => batch.clone(),
                };
                if batch.num_rows() != 0 {
                    writer.write(&batch).map_err(serde::ser::Error::custom)?;
                }
            }
            writer.finish().map_err(serde::ser::Error::custom)?;
        }
        serializer.serialize_str(&STANDARD.encode(bytes))
    }
}
impl<'de> Deserialize<'de> for Batches {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let encoded = String::deserialize(deserializer)?;
        let bytes = STANDARD.decode(encoded).map_err(serde::de::Error::custom)?;
        let reader =
            StreamReader::try_new(Cursor::new(bytes), None).map_err(serde::de::Error::custom)?;
        let schema = reader.schema();
        let batches = reader
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(serde::de::Error::custom)?;
        Ok(Self {
            schema,
            batches,
            deleted: vec![],
        })
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct MemoryTable {
    pub id: ObjectId,
    pub labels: BTreeSet<String>,
    pub data: Batches,
    #[serde(skip)]
    pub cache: Arc<OnceLock<Table>>,
}
impl MemoryTable {
    pub fn open(&self, edge: bool) -> Result<Table> {
        if let Some(table) = self.cache.get() {
            return Ok(table.clone());
        }
        let mut table = Table::try_new_with_deletions(
            self.labels.iter().cloned().collect(),
            self.data.schema.clone(),
            self.data.batches.clone(),
            if edge {
                &[
                    crate::graph::ID,
                    crate::graph::SOURCE,
                    crate::graph::DESTINATION,
                ]
            } else {
                &[crate::graph::ID]
            },
            self.data.deleted.clone(),
        )?;
        table.memory_id = Some(self.id);
        let _ = self.cache.set(table);
        Ok(self.cache.get().unwrap().clone())
    }
    pub fn from_table(id: ObjectId, mut table: Table) -> Self {
        table.memory_id = Some(id);
        table.memory_deleted_ids.clear();
        let data = Batches {
            schema: table.schema.clone(),
            batches: table.batches.clone(),
            deleted: table.resident.deleted.clone(),
        };
        let cache = Arc::new(OnceLock::new());
        let labels = table.labels.clone();
        let _ = cache.set(table);
        Self {
            id,
            labels,
            data,
            cache,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) enum Fragment {
    Parquet(TableManifest),
    Memory(MemoryTable),
}
impl Fragment {
    pub fn id(&self) -> ObjectId {
        match self {
            Self::Parquet(t) => t.id,
            Self::Memory(t) => t.id,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) enum TableChange {
    Reuse(ObjectId),
    Mask {
        id: ObjectId,
        deleted_ids: BTreeSet<u64>,
    },
    Parquet(TableManifest),
    Memory {
        table: MemoryTable,
        append: bool,
        deleted_ids: BTreeSet<u64>,
    },
}
impl TableChange {
    pub fn apply(&self, previous: &[Fragment], edge: bool) -> Result<Fragment> {
        match self {
            Self::Mask { id, deleted_ids } => {
                let Some(Fragment::Parquet(old)) = previous.iter().find(|f| f.id() == *id) else {
                    return Err(Error::Corrupt("missing immutable mask base".into()));
                };
                Ok(Fragment::Parquet(old.with_deletes(deleted_ids)?))
            }
            Self::Reuse(id) => previous
                .iter()
                .find(|t| t.id() == *id)
                .cloned()
                .ok_or_else(|| Error::Corrupt("missing reused graph fragment".into())),
            Self::Parquet(table) => Ok(Fragment::Parquet(table.clone())),
            Self::Memory {
                table,
                append: false,
                deleted_ids,
            } => {
                if !deleted_ids.is_empty() {
                    return Err(Error::Corrupt("new MemTable has a removal base".into()));
                }
                let mut loaded = table.open(edge)?;
                loaded.prune_deleted_batches()?;
                Ok(Fragment::Memory(MemoryTable::from_table(table.id, loaded)))
            }
            Self::Memory {
                table,
                append: true,
                deleted_ids,
            } => {
                let Some(Fragment::Memory(old)) = previous.iter().find(|t| t.id() == table.id)
                else {
                    return Err(Error::Corrupt("missing MemTable append base".into()));
                };
                if old.labels != table.labels || old.data.schema != table.data.schema {
                    return Err(Error::Corrupt("MemTable append layout differs".into()));
                }
                let extra = table.open(edge)?;
                let mut loaded = old.open(edge)?;
                loaded.mask_rows(deleted_ids)?;
                loaded.append_rows(extra)?;
                loaded.prune_deleted_batches()?;
                Ok(Fragment::Memory(MemoryTable::from_table(old.id, loaded)))
            }
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct EdgeChange {
    pub table: TableChange,
    pub directed: bool,
}
