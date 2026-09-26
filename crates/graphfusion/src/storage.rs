//! Storage generations with immutable Arrow graphs and internal commit-protocol test rows.
//! No participant may publish independently of the catalog coordinator.
use crate::{
    catalog::{CommitSeq, ObjectId},
    Error, Result,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct StorageSnapshot {
    pub generations: BTreeMap<ObjectId, Generation>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Generation {
    pub retired_at: Option<CommitSeq>,
    pub rows: BTreeMap<String, Row>,
    #[serde(default)]
    pub graph_version: CommitSeq,
    #[serde(
        default,
        serialize_with = "serialize_memory_graph",
        deserialize_with = "deserialize_memory_graph"
    )]
    pub graph: Option<Arc<crate::graph::GraphData>>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Row {
    pub version: CommitSeq,
    pub value: Option<Vec<u8>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) enum StorageChange {
    #[serde(skip)]
    ReplaceGraph {
        generation: ObjectId,
        data: Arc<crate::graph::GraphData>,
    },
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
            StorageChange::ReplaceGraph { generation, data } => {
                let target = self
                    .generations
                    .get_mut(generation)
                    .ok_or_else(|| Error::Corrupt("missing graph generation".into()))?;
                if target.retired_at.is_some() {
                    return Err(Error::Conflict("graph generation was retired".into()));
                }
                target.graph = Some(data.clone());
                target.graph_version = seq;
            }
            StorageChange::Create(id) => {
                if self.generations.contains_key(id) {
                    return Err(Error::Corrupt("duplicate storage generation".into()));
                }
                self.generations.insert(
                    *id,
                    Generation {
                        retired_at: None,
                        rows: BTreeMap::new(),
                        graph: None,
                        graph_version: 0,
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

fn serialize_memory_graph<S: serde::Serializer>(
    graph: &Option<Arc<crate::graph::GraphData>>,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
    if graph.is_some() {
        return Err(serde::ser::Error::custom(
            "memory graph requires a durable provider before persistence",
        ));
    }
    serializer.serialize_none()
}
fn deserialize_memory_graph<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<Arc<crate::graph::GraphData>>, D::Error> {
    let value = Option::<()>::deserialize(deserializer)?;
    if value.is_some() {
        return Err(serde::de::Error::custom("invalid memory graph snapshot"));
    }
    Ok(None)
}
