//! GQL catalog, sessions, and DataFusion query execution.
pub mod gql {
    pub use gql_parser::*;
}
pub mod catalog;
mod error;
pub mod graph;
mod memtable;
mod parquet;
mod persistence;
mod query;
mod session;
mod storage;
mod transaction;
pub mod types;

pub use datafusion::arrow;
pub use error::{Error, Result};
use persistence::{Disk, FileGuard};
pub use query::QueryResult;
pub use session::{
    ExecutionResult, Parameter, QueryLimits, Session, SessionState, StatementOutput,
    StatementResult, TransactionAction, TransactionStatus, Value,
};
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex, OnceLock, Weak,
    },
    time::Instant,
};
use transaction::{PublishedState, StatementTxn};

#[derive(Clone, Debug, Default)]
pub struct OpenOptions {
    /// Persistent databases require a local filesystem with working file locks and atomic rename.
    pub create_if_missing: bool,
}
/// Limits apply to resident batches in each graph. Either limit triggers sealing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StorageOptions {
    pub memtable_max_rows: usize,
    pub memtable_max_bytes: usize,
}
impl Default for StorageOptions {
    fn default() -> Self {
        Self {
            memtable_max_rows: 65_536,
            memtable_max_bytes: 16 * 1024 * 1024,
        }
    }
}
#[derive(Debug, Default)]
struct Metrics {
    commits: AtomicU64,
    conflicts: AtomicU64,
    commit_wait_micros: AtomicU64,
    recovery_micros: AtomicU64,
    checkpoint_micros: AtomicU64,
    checkpoint_busy: AtomicU64,
}
#[derive(Clone, Debug, Default)]
pub struct Statistics {
    pub commits: u64,
    pub conflicts: u64,
    pub commit_wait_micros: u64,
    pub recovery_micros: u64,
    pub checkpoint_micros: u64,
    pub checkpoint_busy: u64,
    pub log_bytes: u64,
    /// Resident main-branch rows and Arrow array bytes in the published snapshot.
    pub memtable_rows: usize,
    pub memtable_bytes: usize,
    pub sealed_files: usize,
}
#[derive(Debug)]
struct Inner {
    disk: Option<Disk>,
    state: Mutex<MainState>,
    branches: Mutex<MemoryBranches>,
    branch_cache: Mutex<HashMap<String, (u64, Arc<PublishedState>)>>,
    active: AtomicUsize,
    unhealthy: AtomicBool,
    metrics: Metrics,
    storage_options: Mutex<StorageOptions>,
}
#[derive(Debug)]
struct MainState {
    published: Arc<PublishedState>,
    generation: u64,
    // A failed checkpoint or post-WAL publication can leave disk ahead of this state.
    needs_recovery: bool,
}
impl MainState {
    fn new(published: PublishedState, generation: u64) -> Self {
        Self {
            published: Arc::new(published),
            generation,
            needs_recovery: false,
        }
    }
}
#[derive(Debug, Default)]
struct MemoryBranches {
    next_id: u64,
    commits: BTreeMap<u64, StoredBranchCommit>,
    refs: BTreeMap<String, u64>,
}
#[derive(Debug)]
struct StoredBranchCommit {
    #[allow(dead_code)]
    parent: Option<u64>,
    state: Arc<PublishedState>,
}
impl MemoryBranches {
    fn publish_main(&mut self, state: &PublishedState) -> Result<()> {
        let parent = self.refs.get("main").copied();
        if let Some(id) = parent {
            if self.commits.get(&id).map(|commit| commit.state.commit_seq) == Some(state.commit_seq)
            {
                return Ok(());
            }
        }
        let id = self.allocate()?;
        self.commits.insert(
            id,
            StoredBranchCommit {
                parent,
                state: Arc::new(state.clone()),
            },
        );
        self.refs.insert("main".into(), id);
        Ok(())
    }
    fn allocate(&mut self) -> Result<u64> {
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or_else(|| Error::InvalidDefinition("commit ids exhausted".into()))?;
        Ok(self.next_id)
    }
}
/// A named ref pointing at an immutable catalog commit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Branch {
    pub name: String,
    pub commit: u64,
}
#[derive(Clone, Debug)]
pub struct Database {
    inner: Arc<Inner>,
}
impl Default for Database {
    fn default() -> Self {
        Self::new()
    }
}
struct RegisteredDatabase {
    inner: Weak<Inner>,
    _owner: FileGuard,
}
type Registry = Mutex<HashMap<PathBuf, RegisteredDatabase>>;
static DATABASES: OnceLock<Registry> = OnceLock::new();

impl Drop for Inner {
    fn drop(&mut self) {
        if let Some(disk) = &self.disk {
            let mut registry = DATABASES
                .get()
                .unwrap()
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if registry
                .get(&disk.directory)
                .is_some_and(|entry| std::ptr::eq(entry.inner.as_ptr(), self))
            {
                registry.remove(&disk.directory);
            }
        }
    }
}

impl Database {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                disk: None,
                state: Mutex::new(MainState::new(PublishedState::initial(), 0)),
                branches: Mutex::new(MemoryBranches::default()),
                branch_cache: Mutex::new(HashMap::new()),
                active: AtomicUsize::new(0),
                unhealthy: AtomicBool::new(false),
                metrics: Metrics::default(),
                storage_options: Mutex::new(StorageOptions::default()),
            }),
        }
    }
    /// Opens a database directory exclusively for this process. Reopening the same
    /// canonical directory shares its coordinator; cloned handles and independent
    /// sessions may be used on multiple threads. Another process gets DatabaseInUse.
    /// Network filesystems and inherited handles after fork are unsupported.
    pub fn open(path: impl AsRef<Path>, options: OpenOptions) -> Result<Self> {
        if !cfg!(any(target_os = "linux", target_os = "macos")) {
            return Err(Error::UnsupportedFeature(
                "persistent databases require Linux or macOS".into(),
            ));
        }
        let path = path.as_ref();
        if options.create_if_missing {
            let existed = path.exists();
            fs::create_dir_all(path)?;
            // Persist a newly created directory entry. An existing database does not need
            // its ancestors opened, so a non-readable parent still allows a reopen.
            if !existed {
                let absolute = fs::canonicalize(path)?;
                for parent in absolute.ancestors().skip(1) {
                    match fs::File::open(parent) {
                        Ok(file) => file.sync_all()?,
                        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                            break;
                        }
                        Err(error) => return Err(error.into()),
                    }
                }
            }
        }
        let directory = fs::canonicalize(path)?;
        let mut registry = DATABASES
            .get_or_init(Mutex::default)
            .lock()
            .map_err(|_| Error::Poisoned)?;
        if let Some(inner) = registry
            .get(&directory)
            .and_then(|entry| entry.inner.upgrade())
        {
            drop(registry);
            if inner.unhealthy.load(Ordering::SeqCst) {
                return Err(Error::Poisoned);
            }
            return Ok(Self { inner });
        }
        // A concurrent last-handle drop can leave a dead Weak while Inner::drop
        // waits for this mutex. Release its lock here before acquiring a new one;
        // otherwise this process could spuriously reject its own reopen. That
        // destructor checks identity so it cannot remove the replacement entry.
        registry.remove(&directory);
        let disk = Disk {
            directory: directory.clone(),
            next_commit_id: Mutex::new(None),
        };
        let owner = disk.acquire_ownership()?;
        let _commit = disk.commit_lock()?;
        if !options.create_if_missing && !directory.join("MANIFEST").exists() {
            return Err(Error::NotFound("database manifest".into()));
        }
        disk.initialize()?;
        let (state, generation) = disk.load()?;
        let inner = Arc::new(Inner {
            disk: Some(disk),
            state: Mutex::new(MainState::new(state, generation)),
            branches: Mutex::new(MemoryBranches::default()),
            branch_cache: Mutex::new(HashMap::new()),
            active: AtomicUsize::new(0),
            unhealthy: AtomicBool::new(false),
            metrics: Metrics::default(),
            storage_options: Mutex::new(StorageOptions::default()),
        });
        registry.insert(
            directory,
            RegisteredDatabase {
                inner: Arc::downgrade(&inner),
                _owner: owner,
            },
        );
        Ok(Self { inner })
    }
    pub fn parse_gql(&self, input: &str) -> gql::Result<gql::Program> {
        gql::parse(input)
    }
    pub fn session(&self) -> Session {
        Session::new(self.clone())
    }
    /// Points a new branch at the current `main` snapshot. Later commits on that
    /// branch write a new immutable snapshot and move only its ref.
    pub fn create_branch(&self, name: &str) -> Result<u64> {
        validate_branch_name(name)?;
        if name == "main" {
            return Err(Error::AlreadyExists(name.into()));
        }
        let mut state = self.inner.state.lock().map_err(|_| Error::Poisoned)?;
        self.check_healthy()?;
        self.recover_main(&mut state)?;
        if let Some(disk) = &self.inner.disk {
            let _commit = disk.commit_lock()?;
            if disk.read_ref(name)?.is_some() {
                return Err(Error::AlreadyExists(name.into()));
            }
            let id = disk.ensure_main_snapshot(&state.published)?;
            disk.write_ref(name, id)?;
            return Ok(id);
        }
        let mut branches = self.inner.branches.lock().map_err(|_| Error::Poisoned)?;
        if branches.refs.contains_key(name) {
            return Err(Error::AlreadyExists(name.into()));
        }
        branches.publish_main(&state.published)?;
        let id = *branches
            .refs
            .get("main")
            .ok_or_else(|| Error::Corrupt("main branch snapshot missing".into()))?;
        branches.refs.insert(name.to_owned(), id);
        Ok(id)
    }
    pub fn branches(&self) -> Result<Vec<Branch>> {
        let mut listed = if let Some(disk) = &self.inner.disk {
            let mut state = self.inner.state.lock().map_err(|_| Error::Poisoned)?;
            self.check_healthy()?;
            self.recover_main(&mut state)?;
            let _commit = disk.commit_lock()?;
            disk.ensure_main_snapshot(&state.published)?;
            disk.list_refs()?
        } else {
            let branches = self.inner.branches.lock().map_err(|_| Error::Poisoned)?;
            branches
                .refs
                .iter()
                .map(|(name, commit)| (name.clone(), *commit))
                .collect()
        };
        if !listed.iter().any(|(name, _)| name == "main") {
            listed.push(("main".into(), 0));
        }
        listed.sort();
        Ok(listed
            .into_iter()
            .map(|(name, commit)| Branch { name, commit })
            .collect())
    }
    pub(crate) fn read_branch(&self, name: &str) -> Result<(u64, Arc<PublishedState>)> {
        if let Some(disk) = &self.inner.disk {
            let id = disk
                .read_ref(name)?
                .ok_or_else(|| Error::NotFound(name.into()))?;
            let mut cache = self
                .inner
                .branch_cache
                .lock()
                .map_err(|_| Error::Poisoned)?;
            if let Some((cached_id, state)) = cache.get(name) {
                if *cached_id == id {
                    return Ok((id, state.clone()));
                }
            }
            let state = Arc::new(disk.read_commit(id)?.state);
            disk.check_graph_files(&state)?;
            cache.insert(name.to_owned(), (id, state.clone()));
            return Ok((id, state));
        }
        let branches = self.inner.branches.lock().map_err(|_| Error::Poisoned)?;
        let id = *branches
            .refs
            .get(name)
            .ok_or_else(|| Error::NotFound(name.into()))?;
        let state = branches
            .commits
            .get(&id)
            .ok_or_else(|| Error::Corrupt(format!("missing commit {id}")))?
            .state
            .clone();
        Ok((id, state))
    }
    pub(crate) fn note_main_commit(&self, state: &PublishedState) -> Result<()> {
        self.inner
            .branches
            .lock()
            .map_err(|_| Error::Poisoned)?
            .publish_main(state)
    }
    pub(crate) fn publish_branch_commit(
        &self,
        name: &str,
        expected: u64,
        state: PublishedState,
    ) -> Result<()> {
        let mut branches = self.inner.branches.lock().map_err(|_| Error::Poisoned)?;
        let current = *branches
            .refs
            .get(name)
            .ok_or_else(|| Error::NotFound(name.into()))?;
        if current != expected {
            return Err(Error::Conflict("branch moved".into()));
        }
        let id = branches.allocate()?;
        branches.commits.insert(
            id,
            StoredBranchCommit {
                parent: Some(expected),
                state: Arc::new(state),
            },
        );
        branches.refs.insert(name.to_owned(), id);
        Ok(())
    }
    /// Inspects one consistent catalog snapshot, without opening a cross-statement transaction.
    pub fn with_catalog<T>(
        &self,
        inspect: impl FnOnce(&catalog::CatalogSnapshot) -> T,
    ) -> Result<T> {
        let tx = StatementTxn::begin(self)?;
        Ok(inspect(tx.catalog()))
    }
    /// Administrative provisioning; schema DDL does not implicitly create directories.
    pub fn create_directory(&self, components: &[&str]) -> Result<catalog::ObjectId> {
        let mut tx = StatementTxn::begin(self)?;
        let mut parent = catalog::ROOT_DIRECTORY;
        for name in components {
            parent = match tx.lookup(parent, catalog::ObjectKind::Directory, name) {
                Some(entry) => entry.id,
                None => tx.create(parent, name, catalog::ObjectDefinition::Directory)?,
            };
        }
        tx.commit()?;
        Ok(parent)
    }
    /// Returns Busy instead of waiting for an active statement or explicit transaction,
    /// which might belong to this thread. Retry after snapshot leases are released.
    pub fn checkpoint(&self) -> Result<()> {
        self.check_healthy()?;
        let started = Instant::now();
        let mut current = self.inner.state.lock().map_err(|_| Error::Poisoned)?;
        self.check_healthy()?;
        if self.inner.active.load(Ordering::SeqCst) != 0 {
            self.inner
                .metrics
                .checkpoint_busy
                .fetch_add(1, Ordering::Relaxed);
            return Err(Error::Busy);
        }
        self.recover_main(&mut current)?;
        let _commit = self
            .inner
            .disk
            .as_ref()
            .map(|d| d.commit_lock())
            .transpose()?;
        let mut next = (*current.published).clone();
        Arc::make_mut(&mut next.storage).reclaim();
        Arc::make_mut(&mut next.catalog)
            .names
            .retain(|_, binding| binding.object.is_some());
        next.validate()?;
        if let Some(disk) = &self.inner.disk {
            let count = next
                .storage
                .generations
                .values()
                .filter_map(|g| g.parquet.as_ref())
                .flat_map(|m| m.fragments())
                .filter(|f| matches!(f, memtable::Fragment::Memory(_)))
                .count() as u64;
            let reserved = next
                .next_id
                .checked_add(count)
                .ok_or_else(|| Error::InvalidDefinition("object IDs exhausted".into()))?;
            current.needs_recovery = true;
            if count != 0 {
                if let Err(error) = disk.append(
                    current.generation,
                    &persistence::LogRecord::Reserve { next_id: reserved },
                ) {
                    if matches!(error, Error::CommitUnknown(_)) {
                        self.inner.unhealthy.store(true, Ordering::SeqCst);
                    }
                    return Err(error);
                }
                disk.seal_memtables(&mut next)?;
            }
            current.generation = disk.checkpoint(&next, current.generation)?;
            current.needs_recovery = false;
        }
        current.published = Arc::new(next);
        self.inner
            .metrics
            .checkpoint_micros
            .fetch_add(started.elapsed().as_micros() as u64, Ordering::Relaxed);
        Ok(())
    }
    /// Changes the thresholds for all handles sharing this coordinator. A successful
    /// write seals resident batches when either threshold is reached; checkpoint forces sealing.
    pub fn configure_storage(&self, options: StorageOptions) -> Result<()> {
        if options.memtable_max_rows == 0 || options.memtable_max_bytes == 0 {
            return Err(Error::InvalidDefinition(
                "MemTable thresholds must be nonzero".into(),
            ));
        }
        *self
            .inner
            .storage_options
            .lock()
            .map_err(|_| Error::Poisoned)? = options;
        Ok(())
    }
    pub fn statistics(&self) -> Result<Statistics> {
        let m = &self.inner.metrics;
        let mut stats = Statistics {
            commits: m.commits.load(Ordering::Relaxed),
            conflicts: m.conflicts.load(Ordering::Relaxed),
            commit_wait_micros: m.commit_wait_micros.load(Ordering::Relaxed),
            recovery_micros: m.recovery_micros.load(Ordering::Relaxed),
            checkpoint_micros: m.checkpoint_micros.load(Ordering::Relaxed),
            checkpoint_busy: m.checkpoint_busy.load(Ordering::Relaxed),
            log_bytes: 0,
            memtable_rows: 0,
            memtable_bytes: 0,
            sealed_files: 0,
        };
        let current = self.inner.state.lock().map_err(|_| Error::Poisoned)?;
        for manifest in current
            .published
            .storage
            .generations
            .values()
            .filter_map(|g| g.parquet.as_ref())
        {
            stats.sealed_files += manifest.tables().count();
            for fragment in manifest.fragments() {
                if let memtable::Fragment::Memory(t) = fragment {
                    stats.memtable_rows +=
                        t.data.batches.iter().map(|b| b.num_rows()).sum::<usize>();
                    stats.memtable_bytes += t
                        .data
                        .batches
                        .iter()
                        .map(|b| b.get_array_memory_size())
                        .sum::<usize>();
                }
            }
        }
        drop(current);
        if let Some(disk) = &self.inner.disk {
            for entry in fs::read_dir(&disk.directory)? {
                let entry = entry?;
                if entry.file_name().to_string_lossy().starts_with("wal-") {
                    match entry.metadata() {
                        Ok(m) => stats.log_bytes += m.len(),
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                        Err(e) => return Err(e.into()),
                    }
                }
            }
        }
        Ok(stats)
    }
    // The lifetime ownership lock excludes other processes, and state serializes all
    // writers in this process. Reload only when a failed write made the cache uncertain.
    fn recover_main(&self, current: &mut MainState) -> Result<()> {
        if current.needs_recovery {
            let disk =
                self.inner.disk.as_ref().ok_or_else(|| {
                    Error::Corrupt("in-memory state requires disk recovery".into())
                })?;
            let _commit = disk.commit_lock()?;
            let started = Instant::now();
            let (state, generation) = disk.load()?;
            current.published = Arc::new(state);
            current.generation = generation;
            current.needs_recovery = false;
            self.inner
                .metrics
                .recovery_micros
                .fetch_add(started.elapsed().as_micros() as u64, Ordering::Relaxed);
        }
        Ok(())
    }
    fn check_healthy(&self) -> Result<()> {
        if self.inner.unhealthy.load(Ordering::SeqCst) {
            return Err(Error::Poisoned);
        }
        Ok(())
    }
}

fn validate_branch_name(name: &str) -> Result<()> {
    let ok = !name.is_empty()
        && name != "."
        && name != ".."
        && !name.starts_with('.')
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_');
    if ok {
        Ok(())
    } else {
        Err(Error::InvalidDefinition(format!(
            "invalid branch name {name}"
        )))
    }
}

#[cfg(test)]
mod tests;
