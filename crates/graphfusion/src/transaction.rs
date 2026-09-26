use crate::{
    catalog::*,
    persistence::{FileGuard, LogRecord},
    storage::{StorageChange, StorageSnapshot},
    Database, Error, Result,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    sync::{atomic::Ordering, Arc},
    time::Instant,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct PublishedState {
    pub commit_seq: CommitSeq,
    pub next_id: ObjectId,
    pub catalog: Arc<CatalogSnapshot>,
    pub storage: Arc<StorageSnapshot>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct CommitRecord {
    pub seq: CommitSeq,
    pub catalog: Vec<CatalogChange>,
    pub storage: Vec<StorageChange>,
}
impl PublishedState {
    pub fn initial() -> Self {
        Self {
            commit_seq: 0,
            next_id: 3,
            catalog: Arc::new(CatalogSnapshot::initial()),
            storage: Arc::new(StorageSnapshot::default()),
        }
    }
    pub fn apply(&self, record: &CommitRecord) -> Result<Self> {
        if self.commit_seq.checked_add(1) != Some(record.seq) {
            return Err(Error::Corrupt("nonconsecutive commit sequence".into()));
        }
        let mut next = self.clone();
        next.commit_seq = record.seq;
        for change in &record.catalog {
            match change {
                CatalogChange::Put(entry) => {
                    Arc::make_mut(&mut next.catalog).put(entry.clone(), record.seq)
                }
                CatalogChange::Delete(id) => {
                    Arc::make_mut(&mut next.catalog).remove(*id, record.seq)
                }
            }
        }
        for change in &record.storage {
            Arc::make_mut(&mut next.storage).apply(change, record.seq)?;
        }
        next.validate()?;
        Ok(next)
    }
    pub fn validate(&self) -> Result<()> {
        self.catalog.validate()?;
        let mut owners = std::collections::BTreeSet::new();
        let mut files = self
            .catalog
            .entries()
            .map(|e| e.id)
            .chain(self.storage.generations.keys().copied())
            .collect();
        for entry in self.catalog.entries() {
            if entry.id >= self.next_id || entry.version > self.commit_seq {
                return Err(Error::Corrupt(
                    "catalog ID or version exceeds high watermark".into(),
                ));
            }
            if let ObjectDefinition::Graph { storage, .. } = entry.definition {
                if !owners.insert(storage)
                    || self
                        .storage
                        .generations
                        .get(&storage)
                        .map(|g| g.retired_at.is_none())
                        != Some(true)
                {
                    return Err(Error::Corrupt(
                        "graph references missing, shared or retired storage".into(),
                    ));
                }
            }
        }
        for (id, generation) in &self.storage.generations {
            if generation.graph.is_some() && generation.parquet.is_some() {
                return Err(Error::Corrupt("graph has two storage providers".into()));
            }
            if let Some(manifest) = &generation.parquet {
                manifest.validate(self.next_id, &mut files)?;
            }
            if *id >= self.next_id
                || (generation.retired_at.is_none() && !owners.contains(id))
                || generation.retired_at.is_some_and(|s| s > self.commit_seq)
                || generation.graph_version > self.commit_seq
                || generation
                    .rows
                    .values()
                    .any(|r| r.version > self.commit_seq)
            {
                return Err(Error::Corrupt(
                    "invalid storage ownership or watermark".into(),
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
pub(crate) struct StatementTxn {
    db: Database,
    read_only: bool,
    _lease: Option<FileGuard>,
    pub base: Arc<PublishedState>,
    view: CatalogSnapshot,
    changes: Vec<CatalogChange>,
    storage_changes: Vec<StorageChange>,
    objects_read: BTreeMap<ObjectId, Option<CommitSeq>>,
    names_read: BTreeMap<String, NameBinding>,
    members_read: BTreeMap<ObjectId, CommitSeq>,
    references_read: BTreeMap<ObjectId, CommitSeq>,
    rows_written: BTreeMap<(ObjectId, String), Option<CommitSeq>>,
    graph_versions: BTreeMap<ObjectId, CommitSeq>,
    ids: std::ops::Range<ObjectId>,
}
impl Drop for StatementTxn {
    fn drop(&mut self) {
        self.db.inner.active.fetch_sub(1, Ordering::SeqCst);
    }
}
impl StatementTxn {
    pub fn begin(db: &Database) -> Result<Self> {
        db.check_healthy()?;
        let lease = db
            .inner
            .disk
            .as_ref()
            .map(|d| d.lifecycle(false))
            .transpose()?;
        let mut state = db.inner.state.lock().map_err(|_| Error::Poisoned)?;
        db.check_healthy()?;
        let _commit = db
            .inner
            .disk
            .as_ref()
            .map(|d| d.commit_lock())
            .transpose()?;
        if let Some(disk) = &db.inner.disk {
            let started = Instant::now();
            *state = Arc::new(disk.load()?.0);
            db.inner
                .metrics
                .recovery_micros
                .fetch_add(started.elapsed().as_micros() as u64, Ordering::Relaxed);
        }
        db.inner.active.fetch_add(1, Ordering::SeqCst);
        Ok(Self {
            db: db.clone(),
            read_only: false,
            _lease: lease,
            base: state.clone(),
            view: (*state.catalog).clone(),
            changes: Vec::new(),
            storage_changes: Vec::new(),
            objects_read: BTreeMap::new(),
            names_read: BTreeMap::new(),
            members_read: BTreeMap::new(),
            references_read: BTreeMap::new(),
            rows_written: BTreeMap::new(),
            graph_versions: BTreeMap::new(),
            ids: 0..0,
        })
    }
    pub fn set_read_only(&mut self, read_only: bool) {
        self.read_only = read_only;
    }
    fn require_writable(&self) -> Result<()> {
        if self.read_only {
            Err(Error::ReadOnlyTransaction)
        } else {
            Ok(())
        }
    }
    pub fn catalog(&self) -> &CatalogSnapshot {
        &self.view
    }
    pub fn graph_data(&mut self, graph: ObjectId) -> Result<Arc<crate::graph::GraphData>> {
        let ObjectDefinition::Graph { storage, shape } = self.get(graph)?.definition else {
            return Err(Error::InvalidReference("graph required".into()));
        };
        if shape != GraphShape::Open {
            return Err(Error::UnsupportedFeature(
                "typed graph scan schema binding".into(),
            ));
        }
        self.graph_versions.entry(storage).or_insert_with(|| {
            self.base
                .storage
                .generations
                .get(&storage)
                .map_or(0, |g| g.graph_version)
        });
        for change in self.storage_changes.iter().rev() {
            if let StorageChange::ReplaceGraph { generation, data } = change {
                if *generation == storage {
                    return Ok(data.clone());
                }
            }
            if let StorageChange::ReplaceParquet {
                generation,
                manifest,
            } = change
            {
                if *generation == storage {
                    return manifest.open(self.db.inner.disk.as_ref().ok_or_else(|| {
                        Error::Corrupt("Parquet graph without database directory".into())
                    })?);
                }
            }
        }
        if let Some(manifest) = self
            .base
            .storage
            .generations
            .get(&storage)
            .and_then(|g| g.parquet.as_ref())
        {
            return manifest.open(self.db.inner.disk.as_ref().ok_or_else(|| {
                Error::Corrupt("Parquet graph without database directory".into())
            })?);
        }
        Ok(self
            .base
            .storage
            .generations
            .get(&storage)
            .and_then(|generation| generation.graph.clone())
            .unwrap_or_default())
    }
    pub fn replace_graph_data(
        &mut self,
        graph: ObjectId,
        data: crate::graph::GraphData,
    ) -> Result<()> {
        self.require_writable()?;
        let ObjectDefinition::Graph { storage, shape } = self.get(graph)?.definition else {
            return Err(Error::InvalidReference("graph required".into()));
        };
        if shape != GraphShape::Open {
            return Err(Error::UnsupportedFeature(
                "typed graph import validation".into(),
            ));
        }
        self.graph_versions.entry(storage).or_insert_with(|| {
            self.base
                .storage
                .generations
                .get(&storage)
                .map_or(0, |g| g.graph_version)
        });
        let next_element_id = data.next_element_id();
        self.storage_changes.push(StorageChange::AdvanceElementId {
            generation: storage,
            next: next_element_id,
        });
        if self.db.inner.disk.is_some() {
            let mut manifest = crate::parquet::GraphManifest::default();
            for table in &data.nodes {
                let id = self.allocate_id()?;
                manifest.nodes.push(
                    self.db
                        .inner
                        .disk
                        .as_ref()
                        .unwrap()
                        .write_graph_table(id, &table.0)?,
                );
            }
            for edge in &data.edges {
                let id = self.allocate_id()?;
                manifest.edges.push(crate::parquet::EdgeManifest {
                    table: self
                        .db
                        .inner
                        .disk
                        .as_ref()
                        .unwrap()
                        .write_graph_table(id, &edge.table)?,
                    directed: edge.directed,
                });
            }
            self.db.inner.disk.as_ref().unwrap().finish_graph_files()?;
            self.storage_changes.push(StorageChange::ReplaceParquet {
                generation: storage,
                manifest: Arc::new(manifest),
            });
        } else {
            self.storage_changes.push(StorageChange::ReplaceGraph {
                generation: storage,
                data: Arc::new(data),
            });
        }
        Ok(())
    }
    pub fn allocate_element_ids(&mut self, graph: ObjectId, count: usize) -> Result<Vec<u64>> {
        self.require_writable()?;
        let ObjectDefinition::Graph { storage, .. } = self.get(graph)?.definition else {
            return Err(Error::InvalidReference("graph required".into()));
        };
        let mut next = self
            .base
            .storage
            .generations
            .get(&storage)
            .map_or(Some(0), |g| g.next_element_id);
        for change in &self.storage_changes {
            if let StorageChange::AdvanceElementId {
                generation,
                next: advanced,
            } = change
            {
                if *generation == storage {
                    next = match (next, *advanced) {
                        (Some(a), Some(b)) => Some(a.max(b)),
                        _ => None,
                    };
                }
            }
        }
        let mut ids = Vec::with_capacity(count);
        for _ in 0..count {
            let id =
                next.ok_or_else(|| Error::InvalidDefinition("graph element IDs exhausted".into()))?;
            ids.push(id);
            next = id.checked_add(1);
        }
        if count > 0 {
            self.graph_versions.entry(storage).or_insert_with(|| {
                self.base
                    .storage
                    .generations
                    .get(&storage)
                    .map_or(0, |g| g.graph_version)
            });
            self.storage_changes.push(StorageChange::AdvanceElementId {
                generation: storage,
                next,
            });
        }
        Ok(ids)
    }
    pub fn get(&mut self, id: ObjectId) -> Result<CatalogEntry> {
        self.objects_read
            .entry(id)
            .or_insert_with(|| self.base.catalog.get(id).map(|e| e.version));
        self.view
            .get(id)
            .cloned()
            .ok_or_else(|| Error::InvalidReference(format!("object {id} no longer exists")))
    }
    pub fn lookup(
        &mut self,
        parent: ObjectId,
        kind: ObjectKind,
        name: &str,
    ) -> Option<CatalogEntry> {
        let key = name_key(parent, kind, name);
        self.names_read.entry(key.clone()).or_insert_with(|| {
            self.base
                .catalog
                .names
                .get(&key)
                .cloned()
                .unwrap_or_default()
        });
        let entry = self.view.lookup(parent, kind, name)?.clone();
        self.objects_read
            .entry(entry.id)
            .or_insert_with(|| self.base.catalog.get(entry.id).map(|e| e.version));
        Some(entry)
    }
    pub fn children(&mut self, parent: ObjectId) -> Vec<CatalogEntry> {
        self.members_read
            .entry(parent)
            .or_insert_with(|| *self.base.catalog.members.get(&parent).unwrap_or(&0));
        self.view.children(parent).cloned().collect()
    }
    pub fn create(
        &mut self,
        parent: ObjectId,
        name: &str,
        definition: ObjectDefinition,
    ) -> Result<ObjectId> {
        self.require_writable()?;
        if name.is_empty() {
            return Err(Error::InvalidDefinition("empty object name".into()));
        }
        let container = self.get(parent)?;
        let kind = definition.kind();
        let expected = if matches!(kind, ObjectKind::Schema | ObjectKind::Directory) {
            ObjectKind::Directory
        } else {
            ObjectKind::Schema
        };
        if container.definition.kind() != expected {
            return Err(Error::InvalidDefinition(
                "wrong catalog container kind".into(),
            ));
        }
        if self.lookup(parent, kind, name).is_some() {
            return Err(Error::AlreadyExists(name.into()));
        }
        if let Some(dependency) = definition.dependency() {
            if self.get(dependency)?.definition.kind() != ObjectKind::GraphType {
                return Err(Error::InvalidReference("graph type required".into()));
            }
        }
        let id = self.allocate_id()?;
        let entry = CatalogEntry {
            id,
            parent,
            name: name.into(),
            version: 0,
            definition,
        };
        if let ObjectDefinition::Graph { storage, .. } = entry.definition {
            self.storage_changes.push(StorageChange::Create(storage));
        }
        self.view.put(entry.clone(), 0);
        self.changes.push(CatalogChange::Put(entry));
        Ok(id)
    }
    pub fn create_graph(
        &mut self,
        parent: ObjectId,
        name: &str,
        shape: GraphShape,
    ) -> Result<ObjectId> {
        let storage = self.allocate_id()?;
        self.create(parent, name, ObjectDefinition::Graph { shape, storage })
    }
    pub fn drop_object(&mut self, id: ObjectId) -> Result<()> {
        self.require_writable()?;
        if matches!(id, ROOT_DIRECTORY | MAIN_SCHEMA) {
            return Err(Error::DependencyExists(
                "bootstrap objects cannot be dropped".into(),
            ));
        }
        let entry = self.get(id)?;
        if !self.children(id).is_empty() {
            return Err(Error::DependencyExists(entry.name));
        }
        self.references_read
            .entry(id)
            .or_insert_with(|| *self.base.catalog.references.get(&id).unwrap_or(&0));
        if !self.view.dependents(id).is_empty() {
            return Err(Error::DependencyExists(entry.name));
        }
        self.lookup(entry.parent, entry.definition.kind(), &entry.name);
        if let ObjectDefinition::Graph { storage, .. } = entry.definition {
            self.storage_changes.push(StorageChange::Retire(storage));
        }
        self.view.remove(id, 0);
        self.changes.push(CatalogChange::Delete(id));
        Ok(())
    }
    pub fn commit(self) -> Result<CommitSeq> {
        self.db.check_healthy()?;
        if self.changes.is_empty() && self.storage_changes.is_empty() {
            return Ok(self.base.commit_seq);
        }
        let started = Instant::now();
        let mut state = self.db.inner.state.lock().map_err(|_| Error::Poisoned)?;
        self.db.check_healthy()?;
        let _commit = self
            .db
            .inner
            .disk
            .as_ref()
            .map(|d| d.commit_lock())
            .transpose()?;
        self.db
            .inner
            .metrics
            .commit_wait_micros
            .fetch_add(started.elapsed().as_micros() as u64, Ordering::Relaxed);
        let generation = if let Some(disk) = &self.db.inner.disk {
            let (latest, generation) = disk.load()?;
            *state = Arc::new(latest);
            generation
        } else {
            0
        };
        if let Err(error) = self.validate(&state) {
            self.db
                .inner
                .metrics
                .conflicts
                .fetch_add(1, Ordering::Relaxed);
            return Err(error);
        }
        let record = CommitRecord {
            seq: state
                .commit_seq
                .checked_add(1)
                .ok_or_else(|| Error::InvalidDefinition("commit sequence exhausted".into()))?,
            catalog: self.changes.clone(),
            storage: self.storage_changes.clone(),
        };
        let candidate = Arc::new(state.apply(&record)?);
        if let Some(disk) = &self.db.inner.disk {
            if let Err(error) = disk.append(generation, &LogRecord::Commit(record)) {
                if matches!(error, Error::CommitUnknown(_)) {
                    self.db.inner.unhealthy.store(true, Ordering::SeqCst);
                }
                return Err(error);
            }
        }
        crate::persistence::failpoint("before_publish");
        let seq = candidate.commit_seq;
        *state = candidate;
        self.db
            .inner
            .metrics
            .commits
            .fetch_add(1, Ordering::Relaxed);
        Ok(seq)
    }
    fn validate(&self, current: &PublishedState) -> Result<()> {
        for (generation, version) in &self.graph_versions {
            if current
                .storage
                .generations
                .get(generation)
                .map_or(0, |g| g.graph_version)
                != *version
            {
                return Err(Error::Conflict("graph data changed".into()));
            }
        }
        for (id, version) in &self.objects_read {
            if current.catalog.get(*id).map(|e| e.version) != *version {
                return Err(Error::Conflict(format!("catalog object {id} changed")));
            }
        }
        for (key, binding) in &self.names_read {
            if current.catalog.names.get(key).cloned().unwrap_or_default() != *binding {
                return Err(Error::Conflict(format!("catalog name {key} changed")));
            }
        }
        for (id, version) in &self.members_read {
            if current.catalog.members.get(id).copied().unwrap_or(0) != *version {
                return Err(Error::Conflict(format!("members of {id} changed")));
            }
        }
        for (id, version) in &self.references_read {
            if current.catalog.references.get(id).copied().unwrap_or(0) != *version {
                return Err(Error::Conflict(format!("dependents of {id} changed")));
            }
        }
        for ((generation, key), version) in &self.rows_written {
            if current.storage.row_version(*generation, key) != *version {
                return Err(Error::Conflict("data row changed".into()));
            }
        }
        Ok(())
    }
    fn allocate_id(&mut self) -> Result<ObjectId> {
        self.require_writable()?;
        if let Some(id) = self.ids.next() {
            return Ok(id);
        }
        let mut state = self.db.inner.state.lock().map_err(|_| Error::Poisoned)?;
        self.db.check_healthy()?;
        let _commit = self
            .db
            .inner
            .disk
            .as_ref()
            .map(|d| d.commit_lock())
            .transpose()?;
        let generation = if let Some(disk) = &self.db.inner.disk {
            let (latest, generation) = disk.load()?;
            *state = Arc::new(latest);
            generation
        } else {
            0
        };
        let start = state.next_id;
        let end = start
            .checked_add(64)
            .ok_or_else(|| Error::InvalidDefinition("object IDs exhausted".into()))?;
        if let Some(disk) = &self.db.inner.disk {
            if let Err(error) = disk.append(generation, &LogRecord::Reserve { next_id: end }) {
                if matches!(error, Error::CommitUnknown(_)) {
                    self.db.inner.unhealthy.store(true, Ordering::SeqCst);
                }
                return Err(error);
            }
        }
        Arc::make_mut(&mut state).next_id = end;
        self.ids = start + 1..end;
        Ok(start)
    }
    #[cfg(test)]
    pub fn read_row(&mut self, graph: ObjectId, key: &str) -> Result<Option<Vec<u8>>> {
        let ObjectDefinition::Graph { storage, .. } = self.get(graph)?.definition else {
            return Err(Error::InvalidReference("graph required".into()));
        };
        for change in self.storage_changes.iter().rev() {
            if let StorageChange::Put {
                generation,
                key: row_key,
                value,
            } = change
            {
                if *generation == storage && key == row_key {
                    return Ok(value.clone());
                }
            }
        }
        Ok(self
            .base
            .storage
            .generations
            .get(&storage)
            .and_then(|g| g.rows.get(key))
            .and_then(|r| r.value.clone()))
    }
    #[cfg(test)]
    pub fn write_row(&mut self, graph: ObjectId, key: &str, value: Option<Vec<u8>>) -> Result<()> {
        self.require_writable()?;
        let ObjectDefinition::Graph { storage, .. } = self.get(graph)?.definition else {
            return Err(Error::InvalidReference("graph required".into()));
        };
        self.rows_written
            .entry((storage, key.into()))
            .or_insert_with(|| self.base.storage.row_version(storage, key));
        self.storage_changes.push(StorageChange::Put {
            generation: storage,
            key: key.into(),
            value,
        });
        Ok(())
    }
}
