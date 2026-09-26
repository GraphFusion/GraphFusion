//! Internal storage participant. Rows exercise the joint commit protocol; GQL data execution
//! and physical graph indexes are separate work. No participant may publish independently.
use crate::{
    catalog::{CommitSeq, ObjectId},
    Error, Result,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct StorageSnapshot {
    pub generations: BTreeMap<ObjectId, Generation>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Generation {
    pub retired_at: Option<CommitSeq>,
    pub rows: BTreeMap<String, Row>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Row {
    pub version: CommitSeq,
    pub value: Option<Vec<u8>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) enum StorageChange {
    Create(ObjectId),
    Retire(ObjectId),
    Put {
        generation: ObjectId,
        key: String,
        value: Option<Vec<u8>>,
    },
}

impl StorageSnapshot {
    pub fn apply(&mut self, change: &StorageChange, seq: CommitSeq) -> Result<()> {
        match change {
            StorageChange::Create(id) => {
                if self.generations.contains_key(id) {
                    return Err(Error::Corrupt("duplicate storage generation".into()));
                }
                self.generations.insert(
                    *id,
                    Generation {
                        retired_at: None,
                        rows: BTreeMap::new(),
                    },
                );
            }
            StorageChange::Retire(id) => {
                self.generations
                    .get_mut(id)
                    .ok_or_else(|| Error::Corrupt("missing retired generation".into()))?
                    .retired_at = Some(seq);
            }
            StorageChange::Put {
                generation,
                key,
                value,
            } => {
                let target = self
                    .generations
                    .get_mut(generation)
                    .ok_or_else(|| Error::Corrupt("missing storage generation".into()))?;
                if target.retired_at.is_some() {
                    return Err(Error::Conflict("storage generation was retired".into()));
                }
                target.rows.insert(
                    key.clone(),
                    Row {
                        version: seq,
                        value: value.clone(),
                    },
                );
            }
        }
        Ok(())
    }
    pub fn row_version(&self, generation: ObjectId, key: &str) -> Option<CommitSeq> {
        self.generations
            .get(&generation)?
            .rows
            .get(key)
            .map(|r| r.version)
    }
    pub fn reclaim(&mut self) {
        self.generations.retain(|_, g| g.retired_at.is_none());
        for g in self.generations.values_mut() {
            g.rows.retain(|_, r| r.value.is_some());
        }
    }
}
