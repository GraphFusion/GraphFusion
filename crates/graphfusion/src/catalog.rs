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
#[serde(from = "CatalogSnapshotData")]
pub struct CatalogSnapshot {
    pub(crate) objects: BTreeMap<ObjectId, Arc<CatalogEntry>>,
    pub(crate) names: BTreeMap<String, NameBinding>,
    pub(crate) members: BTreeMap<ObjectId, CommitSeq>,
    pub(crate) references: BTreeMap<ObjectId, CommitSeq>,
    // Rebuilt from objects when a persisted catalog is loaded.
    #[serde(skip)]
    child_index: BTreeSet<(ObjectId, ObjectId)>,
    #[serde(skip)]
    dependent_index: BTreeSet<(ObjectId, ObjectId)>,
}

#[derive(Deserialize)]
struct CatalogSnapshotData {
    objects: BTreeMap<ObjectId, Arc<CatalogEntry>>,
    names: BTreeMap<String, NameBinding>,
    members: BTreeMap<ObjectId, CommitSeq>,
    references: BTreeMap<ObjectId, CommitSeq>,
}

impl From<CatalogSnapshotData> for CatalogSnapshot {
    fn from(data: CatalogSnapshotData) -> Self {
        let child_index = data
            .objects
            .iter()
            .map(|(id, entry)| (entry.parent, *id))
            .collect();
        let dependent_index = data
            .objects
            .iter()
            .filter_map(|(id, entry)| entry.definition.dependency().map(|target| (target, *id)))
            .collect();
        Self {
            objects: data.objects,
            names: data.names,
            members: data.members,
            references: data.references,
            child_index,
            dependent_index,
        }
    }
}

impl CatalogSnapshot {
    pub(crate) fn initial() -> Self {
        let mut result = Self {
            objects: BTreeMap::new(),
            names: BTreeMap::new(),
            members: BTreeMap::new(),
            references: BTreeMap::new(),
            child_index: BTreeSet::new(),
            dependent_index: BTreeSet::new(),
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
        self.lookup_by_key(&name_key(parent, kind, name))
    }
    pub(crate) fn lookup_by_key(&self, key: &str) -> Option<&CatalogEntry> {
        let id = self.names.get(key)?.object?;
        self.get(id)
    }
    pub fn children(&self, parent: ObjectId) -> impl Iterator<Item = &CatalogEntry> {
        self.child_index
            .range((parent, ObjectId::MIN)..=(parent, ObjectId::MAX))
            .map(|(_, id)| self.objects.get(id).expect("indexed child exists").as_ref())
    }
    pub(crate) fn has_children(&self, parent: ObjectId) -> bool {
        self.child_index
            .range((parent, ObjectId::MIN)..=(parent, ObjectId::MAX))
            .next()
            .is_some()
    }
    pub fn entries(&self) -> impl Iterator<Item = &CatalogEntry> {
        self.objects.values().map(Arc::as_ref)
    }

    pub(crate) fn has_dependents(&self, target: ObjectId) -> bool {
        self.dependent_index
            .range((target, ObjectId::MIN)..=(target, ObjectId::MAX))
            .next()
            .is_some()
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
        self.child_index.insert((entry.parent, entry.id));
        if let Some(target) = entry.definition.dependency() {
            self.references.insert(target, seq);
            self.dependent_index.insert((target, entry.id));
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
            self.child_index.remove(&(entry.parent, id));
            if let Some(target) = entry.definition.dependency() {
                self.references.insert(target, seq);
                self.dependent_index.remove(&(target, id));
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
        if self.child_index.len() != self.objects.len() {
            return Err(Error::Corrupt("invalid catalog child index".into()));
        }
        let mut dependencies = 0;
        for (id, entry) in &self.objects {
            if *id != entry.id {
                return Err(Error::Corrupt("catalog object ID mismatch".into()));
            }
            if !self.child_index.contains(&(entry.parent, *id)) {
                return Err(Error::Corrupt("invalid catalog child index".into()));
            }
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
                dependencies += 1;
                if !self.dependent_index.contains(&(id, entry.id)) {
                    return Err(Error::Corrupt("invalid catalog dependent index".into()));
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
        if self.dependent_index.len() != dependencies {
            return Err(Error::Corrupt("invalid catalog dependent index".into()));
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
