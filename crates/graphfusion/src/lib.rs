//! GQL catalog, sessions, and DataFusion query execution.
pub mod gql {
    pub use gql_parser::*;
}
pub mod catalog;
mod error;
pub mod graph;
mod parquet;
mod persistence;
mod query;
mod session;
mod storage;
mod transaction;
pub mod types;

pub use datafusion::arrow;
pub use error::{Error, Result};
use persistence::Disk;
pub use query::QueryResult;
pub use session::{
    ExecutionResult, Parameter, Session, SessionState, StatementOutput, StatementResult,
    TransactionAction, TransactionStatus, Value,
};
use std::{
    collections::HashMap,
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
}
#[derive(Debug)]
struct Inner {
    disk: Option<Disk>,
    state: Mutex<Arc<PublishedState>>,
    active: AtomicUsize,
    unhealthy: AtomicBool,
    metrics: Metrics,
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
type Registry = Mutex<HashMap<PathBuf, Weak<Inner>>>;
static DATABASES: OnceLock<Registry> = OnceLock::new();

impl Database {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                disk: None,
                state: Mutex::new(Arc::new(PublishedState::initial())),
                active: AtomicUsize::new(0),
                unhealthy: AtomicBool::new(false),
                metrics: Metrics::default(),
            }),
        }
    }
    /// Opens a database directory. All processes must use this locking protocol. Network
    /// filesystems and using inherited database handles after fork are unsupported.
    pub fn open(path: impl AsRef<Path>, options: OpenOptions) -> Result<Self> {
        if !cfg!(any(target_os = "linux", target_os = "macos")) {
            return Err(Error::UnsupportedFeature(
                "persistent databases require Linux or macOS".into(),
            ));
        }
        let path = path.as_ref();
        if options.create_if_missing {
            fs::create_dir_all(path)?;
            // Persist newly created directory entries before acknowledging durable commits.
            let absolute = fs::canonicalize(path)?;
            for parent in absolute.ancestors().skip(1) {
                fs::File::open(parent)?.sync_all()?;
            }
        }
        let directory = fs::canonicalize(path)?;
        let mut registry = DATABASES
            .get_or_init(Mutex::default)
            .lock()
            .map_err(|_| Error::Poisoned)?;
        if let Some(inner) = registry.get(&directory).and_then(Weak::upgrade) {
            if !inner.unhealthy.load(Ordering::SeqCst) {
                return Ok(Self { inner });
            }
        }
        let disk = Disk {
            directory: directory.clone(),
        };
        let _lease = disk.lifecycle(false)?;
        let _commit = disk.commit_lock()?;
        if !options.create_if_missing && !directory.join("MANIFEST").exists() {
            return Err(Error::NotFound("database manifest".into()));
        }
        disk.initialize()?;
        let (state, _) = disk.load()?;
        let inner = Arc::new(Inner {
            disk: Some(disk),
            state: Mutex::new(Arc::new(state)),
            active: AtomicUsize::new(0),
            unhealthy: AtomicBool::new(false),
            metrics: Metrics::default(),
        });
        registry.insert(directory, Arc::downgrade(&inner));
        Ok(Self { inner })
    }
    pub fn parse_gql(&self, input: &str) -> gql::Result<gql::Program> {
        gql::parse(input)
    }
    pub fn session(&self) -> Session {
        Session::new(self.clone())
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
        let lease = self
            .inner
            .disk
            .as_ref()
            .map(|d| d.lifecycle(true))
            .transpose();
        if matches!(lease, Err(Error::Busy)) {
            self.inner
                .metrics
                .checkpoint_busy
                .fetch_add(1, Ordering::Relaxed);
        }
        let _lease = lease?;
        let mut current = self.inner.state.lock().map_err(|_| Error::Poisoned)?;
        self.check_healthy()?;
        if self.inner.active.load(Ordering::SeqCst) != 0 {
            self.inner
                .metrics
                .checkpoint_busy
                .fetch_add(1, Ordering::Relaxed);
            return Err(Error::Busy);
        }
        let _commit = self
            .inner
            .disk
            .as_ref()
            .map(|d| d.commit_lock())
            .transpose()?;
        let generation = if let Some(disk) = &self.inner.disk {
            let (state, generation) = disk.load()?;
            *current = Arc::new(state);
            generation
        } else {
            0
        };
        let mut next = (**current).clone();
        Arc::make_mut(&mut next.storage).reclaim();
        Arc::make_mut(&mut next.catalog)
            .names
            .retain(|_, binding| binding.object.is_some());
        next.validate()?;
        if let Some(disk) = &self.inner.disk {
            disk.checkpoint(&next, generation)?;
        }
        *current = Arc::new(next);
        self.inner
            .metrics
            .checkpoint_micros
            .fetch_add(started.elapsed().as_micros() as u64, Ordering::Relaxed);
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
        };
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
    fn check_healthy(&self) -> Result<()> {
        if self.inner.unhealthy.load(Ordering::SeqCst) {
            return Err(Error::Poisoned);
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests;
