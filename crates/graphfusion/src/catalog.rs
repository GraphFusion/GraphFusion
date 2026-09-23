use crate::{types::GraphDefinition, Error, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

pub type ObjectId = u64;
pub type CommitSeq = u64;
pub const ROOT_DIRECTORY: ObjectId = 1;
pub const MAIN_SCHEMA: ObjectId = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ObjectKind {
    Directory,
    Schema,
    Graph,
    GraphType,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GraphShape {
    Open,
    Inline(GraphDefinition),
    Named(ObjectId),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ObjectDefinition {
    Directory,
    Schema,
    Graph {
        shape: GraphShape,
        storage: ObjectId,
    },
    GraphType(GraphDefinition),
}

impl ObjectDefinition {
    pub fn kind(&self) -> ObjectKind {
        match self {
            Self::Directory => ObjectKind::Directory,
            Self::Schema => ObjectKind::Schema,
            Self::Graph { .. } => ObjectKind::Graph,
            Self::GraphType(_) => ObjectKind::GraphType,
        }
    }
    pub(crate) fn dependency(&self) -> Option<ObjectId> {
        match self {
            Self::Graph {
                shape: GraphShape::Named(id),
                ..
            } => Some(*id),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogEntry {
    pub id: ObjectId,
    pub parent: ObjectId,
    pub name: String,
    pub version: CommitSeq,
    pub definition: ObjectDefinition,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct NameBinding {
    pub object: Option<ObjectId>,
    pub version: CommitSeq,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CatalogSnapshot {
    pub(crate) objects: BTreeMap<ObjectId, Arc<CatalogEntry>>,
    pub(crate) names: BTreeMap<String, NameBinding>,
    pub(crate) members: BTreeMap<ObjectId, CommitSeq>,
    pub(crate) references: BTreeMap<ObjectId, CommitSeq>,
}

impl CatalogSnapshot {
    pub(crate) fn initial() -> Self {
        let mut result = Self {
            objects: BTreeMap::new(),
            names: BTreeMap::new(),
            members: BTreeMap::new(),
            references: BTreeMap::new(),
        };
        result.put(
            CatalogEntry {
                id: ROOT_DIRECTORY,
                parent: 0,
                name: String::new(),
                version: 0,
                definition: ObjectDefinition::Directory,
            },
            0,
        );
        result.put(
            CatalogEntry {
                id: MAIN_SCHEMA,
                parent: ROOT_DIRECTORY,
                name: "main".into(),
                version: 0,
                definition: ObjectDefinition::Schema,
            },
            0,
        );
        result
    }

    pub fn get(&self, id: ObjectId) -> Option<&CatalogEntry> {
        self.objects.get(&id).map(Arc::as_ref)
    }
    pub fn lookup(&self, parent: ObjectId, kind: ObjectKind, name: &str) -> Option<&CatalogEntry> {
        let id = self.names.get(&name_key(parent, kind, name))?.object?;
        self.get(id)
    }
    pub fn children(&self, parent: ObjectId) -> impl Iterator<Item = &CatalogEntry> {
        self.objects
            .values()
            .filter(move |e| e.parent == parent)
            .map(Arc::as_ref)
    }
    pub fn entries(&self) -> impl Iterator<Item = &CatalogEntry> {
        self.objects.values().map(Arc::as_ref)
    }

    pub(crate) fn dependents(&self, target: ObjectId) -> BTreeSet<ObjectId> {
        self.objects
            .values()
            .filter(|e| e.definition.dependency() == Some(target))
            .map(|e| e.id)
            .collect()
    }

    pub(crate) fn put(&mut self, mut entry: CatalogEntry, seq: CommitSeq) {
        if self.objects.contains_key(&entry.id) {
            self.remove(entry.id, seq);
        }
        entry.version = seq;
        self.names.insert(
            name_key(entry.parent, entry.definition.kind(), &entry.name),
            NameBinding {
                object: Some(entry.id),
                version: seq,
            },
        );
        self.members.insert(entry.parent, seq);
        if let Some(target) = entry.definition.dependency() {
            self.references.insert(target, seq);
        }
        self.objects.insert(entry.id, Arc::new(entry));
    }

    pub(crate) fn remove(&mut self, id: ObjectId, seq: CommitSeq) {
        if let Some(entry) = self.objects.remove(&id) {
            self.names.insert(
                name_key(entry.parent, entry.definition.kind(), &entry.name),
                NameBinding {
                    object: None,
                    version: seq,
                },
            );
            self.members.insert(entry.parent, seq);
            if let Some(target) = entry.definition.dependency() {
                self.references.insert(target, seq);
            }
        }
    }

    pub(crate) fn validate(&self) -> Result<()> {
        if self.get(ROOT_DIRECTORY).map(|e| (&e.definition, e.parent))
            != Some((&ObjectDefinition::Directory, 0))
            || self
                .get(MAIN_SCHEMA)
                .map(|e| (&e.definition, e.parent, e.name.as_str()))
                != Some((&ObjectDefinition::Schema, ROOT_DIRECTORY, "main"))
        {
            return Err(Error::Corrupt("missing bootstrap catalog objects".into()));
        }
        for entry in self.entries() {
            if entry.id == ROOT_DIRECTORY {
                continue;
            }
            let expected = match entry.definition.kind() {
                ObjectKind::Schema | ObjectKind::Directory => ObjectKind::Directory,
                _ => ObjectKind::Schema,
            };
            if self.get(entry.parent).map(|e| e.definition.kind()) != Some(expected) {
                return Err(Error::Corrupt("invalid catalog parent".into()));
            }
            if let Some(id) = entry.definition.dependency() {
                if self.get(id).map(|e| e.definition.kind()) != Some(ObjectKind::GraphType) {
                    return Err(Error::Corrupt("dangling graph type reference".into()));
                }
            }
            if self
                .lookup(entry.parent, entry.definition.kind(), &entry.name)
                .map(|e| e.id)
                != Some(entry.id)
            {
                return Err(Error::Corrupt("invalid name index".into()));
            }
        }
        for (key, binding) in &self.names {
            if let Some(id) = binding.object {
                let entry = self
                    .get(id)
                    .ok_or_else(|| Error::Corrupt("dangling name binding".into()))?;
                if *key != name_key(entry.parent, entry.definition.kind(), &entry.name) {
                    return Err(Error::Corrupt("name key mismatch".into()));
                }
            }
        }
        Ok(())
    }
}

pub(crate) fn name_key(parent: ObjectId, kind: ObjectKind, name: &str) -> String {
    // JSON escaping prevents delimited identifiers containing separators from aliasing paths.
    serde_json::to_string(&(parent, kind, name)).expect("serializing a name cannot fail")
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) enum CatalogChange {
    Put(CatalogEntry),
    Delete(ObjectId),
}
