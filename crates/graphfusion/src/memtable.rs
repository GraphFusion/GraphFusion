//! Immutable resident fragments and replayable Arrow IPC deltas.
use crate::{catalog::ObjectId, graph::Table, parquet::TableManifest, Error, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use datafusion::arrow::{
    datatypes::SchemaRef,
    ipc::{reader::StreamReader, writer::StreamWriter},
    record_batch::RecordBatch,
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{collections::BTreeSet, io::Cursor};

#[derive(Clone, Debug)]
pub(crate) struct Batches {
    pub schema: SchemaRef,
    pub batches: Vec<RecordBatch>,
}
impl Serialize for Batches {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let mut bytes = Vec::new();
        {
            let mut writer = StreamWriter::try_new(&mut bytes, &self.schema)
                .map_err(serde::ser::Error::custom)?;
            for batch in &self.batches {
                writer.write(batch).map_err(serde::ser::Error::custom)?;
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
        Ok(Self { schema, batches })
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct MemoryTable {
    pub id: ObjectId,
    pub labels: BTreeSet<String>,
    pub data: Batches,
}
impl MemoryTable {
    pub fn open(&self, edge: bool) -> Result<Table> {
        let mut table = Table::try_new(
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
        )?;
        table.memory_id = Some(self.id);
        Ok(table)
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
                let mut table = old.clone();
                table.deleted_ids.extend(deleted_ids.iter().copied());
                Ok(Fragment::Parquet(table))
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
                table.open(edge)?;
                let mut table = table.clone();
                table.data.batches.retain(|b| b.num_rows() != 0);
                Ok(Fragment::Memory(table))
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
                if deleted_ids.is_empty() {
                    let mut next = old.clone();
                    next.data.batches.extend(
                        table
                            .data
                            .batches
                            .iter()
                            .filter(|b| b.num_rows() != 0)
                            .cloned(),
                    );
                    return Ok(Fragment::Memory(next));
                }
                let mut loaded = old.open(edge)?;
                loaded.mask_rows(deleted_ids)?;
                loaded.append_rows(extra)?;
                Ok(Fragment::Memory(MemoryTable {
                    id: old.id,
                    labels: old.labels.clone(),
                    data: Batches {
                        schema: loaded.schema,
                        batches: loaded
                            .batches
                            .into_iter()
                            .filter(|b| b.num_rows() != 0)
                            .collect(),
                    },
                }))
            }
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct EdgeChange {
    pub table: TableChange,
    pub directed: bool,
}
