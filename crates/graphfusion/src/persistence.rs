use crate::{
    transaction::{CommitRecord, PublishedState},
    Error, Result,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

const FORMAT: u32 = 1;
const MAGIC: &[u8; 8] = b"GFLOG001";
const END: &[u8; 8] = b"GFCOMMIT";
const HEADER: usize = 24;
const MAX_RECORD: usize = 64 * 1024 * 1024;

#[derive(Debug)]
pub(crate) struct Disk {
    pub directory: PathBuf,
}

#[derive(Debug)]
pub(crate) struct FileGuard(File);
impl Drop for FileGuard {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

#[derive(Serialize, Deserialize)]
struct Manifest {
    database_id: [u8; 16],
    version: u32,
    generation: u64,
    commit_seq: u64,
    next_id: u64,
    obsolete: Vec<u64>,
}
#[derive(Serialize, Deserialize)]
struct Snapshot<T> {
    database_id: [u8; 16],
    version: u32,
    generation: u64,
    commit_seq: u64,
    value: T,
}

#[derive(Serialize, Deserialize)]
pub(crate) enum LogRecord {
    Header {
        version: u32,
        database_id: [u8; 16],
        generation: u64,
        base_seq: u64,
    },
    Reserve {
        next_id: u64,
    },
    Commit(CommitRecord),
}

impl Disk {
    pub fn lifecycle(&self, exclusive: bool) -> Result<FileGuard> {
        let file = lock_file(&self.directory.join("lifecycle.lock"))?;
        if exclusive {
            match file.try_lock() {
                Ok(()) => (),
                Err(std::fs::TryLockError::WouldBlock) => return Err(Error::Busy),
                Err(std::fs::TryLockError::Error(e)) => return Err(e.into()),
            }
        } else {
            file.lock_shared()?;
        }
        Ok(FileGuard(file))
    }
    pub fn commit_lock(&self) -> Result<FileGuard> {
        let file = lock_file(&self.directory.join("commit.lock"))?;
        file.lock()?;
        Ok(FileGuard(file))
    }

    pub fn initialize(&self) -> Result<()> {
        if self.directory.join("MANIFEST").exists() {
            match fs::remove_file(self.directory.join("INIT")) {
                Ok(()) => sync_directory(&self.directory)?,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                Err(e) => return Err(e.into()),
            }
            return Ok(());
        }
        let mut has_initial_files = false;
        for entry in fs::read_dir(&self.directory)? {
            let name = entry?.file_name();
            let name = name.to_string_lossy();
            if !(name == "commit.lock"
                || name == "lifecycle.lock"
                || name == "MANIFEST.tmp"
                || name == "catalog-0.snapshot"
                || name == "data-0.snapshot"
                || name == "wal-0.log"
                || name == "INIT")
            {
                return Err(Error::Corrupt(
                    "manifest missing from nonempty database directory".into(),
                ));
            }
            has_initial_files |= name != "commit.lock" && name != "lifecycle.lock";
        }
        let marker = self.directory.join("INIT");
        let database_id = if marker.exists() {
            read_document(&marker)?
        } else {
            if has_initial_files {
                return Err(Error::Corrupt(
                    "manifest missing without initialization marker".into(),
                ));
            }
            let mut id = [0; 16];
            File::open("/dev/urandom")?.read_exact(&mut id)?;
            write_document(&marker, &id)?;
            sync_directory(&self.directory)?;
            id
        };
        failpoint("init_marker");
        self.install_checkpoint(&PublishedState::initial(), database_id, 0, Vec::new())?;
        fs::remove_file(marker)?;
        sync_directory(&self.directory)
    }

    pub fn load(&self) -> Result<(PublishedState, u64)> {
        let manifest: Manifest = read_document(&self.directory.join("MANIFEST"))?;
        check_version(manifest.version)?;
        if manifest.obsolete.iter().any(|g| *g >= manifest.generation) {
            return Err(Error::Corrupt("invalid obsolete file generation".into()));
        }
        let catalog: Snapshot<_> =
            read_document(&self.path("catalog", manifest.generation, "snapshot"))?;
        let data: Snapshot<_> = read_document(&self.path("data", manifest.generation, "snapshot"))?;
        for (identity, version, generation, seq) in [
            (
                catalog.database_id,
                catalog.version,
                catalog.generation,
                catalog.commit_seq,
            ),
            (
                data.database_id,
                data.version,
                data.generation,
                data.commit_seq,
            ),
        ] {
            check_version(version)?;
            if identity != manifest.database_id
                || generation != manifest.generation
                || seq != manifest.commit_seq
            {
                return Err(Error::Corrupt(
                    "checkpoint components have different watermarks".into(),
                ));
            }
        }
        let mut state = PublishedState {
            commit_seq: manifest.commit_seq,
            next_id: manifest.next_id,
            catalog: catalog.value,
            storage: data.value,
        };
        state.validate()?;
        let mut log = OpenOptions::new().read(true).write(true).open(self.path(
            "wal",
            manifest.generation,
            "log",
        ))?;
        let header = read_frame(&mut log, false)?
            .ok_or_else(|| Error::Corrupt("missing WAL header".into()))?;
        match serde_json::from_slice::<LogRecord>(&header)? {
            LogRecord::Header {
                version,
                database_id,
                generation,
                base_seq,
            } if database_id == manifest.database_id
                && generation == manifest.generation
                && base_seq == manifest.commit_seq =>
            {
                check_version(version)?
            }
            _ => {
                return Err(Error::Corrupt(
                    "WAL identity or checkpoint watermark mismatch".into(),
                ))
            }
        }
        let mut valid_end = log.stream_position()?;
        while let Some(payload) = read_frame(&mut log, true)? {
            match serde_json::from_slice::<LogRecord>(&payload)? {
                LogRecord::Header { .. } => {
                    return Err(Error::Corrupt("unexpected WAL header".into()))
                }
                LogRecord::Reserve { next_id } => {
                    if next_id <= state.next_id {
                        return Err(Error::Corrupt("ID reservation regressed".into()));
                    }
                    state.next_id = next_id;
                }
                LogRecord::Commit(record) => {
                    state = state.apply(&record)?;
                }
            }
            valid_end = log.stream_position()?;
        }
        if valid_end != log.metadata()?.len() {
            log.set_len(valid_end)?;
        }
        // A crashed writer can leave a complete but unsynced commit. Make it durable before
        // exposing it to a reader, including readers in a different OS process.
        log.sync_all()?;
        // A checkpoint may have crashed after renaming MANIFEST but before syncing the
        // directory. Persist the selected generation before accepting writes to its WAL.
        inject_io_error("recovery_directory_sync")?;
        sync_directory(&self.directory)?;
        Ok((state, manifest.generation))
    }

    pub fn append(&self, generation: u64, record: &LogRecord) -> Result<()> {
        let payload = serde_json::to_vec(record)?;
        let bytes = encode_frame(&payload)?;
        // Serialization does not enforce the recovery deserializer's recursion limit.
        // Reject unreadable records before writing any part of their commit frame.
        serde_json::from_slice::<LogRecord>(&payload).map_err(|error| {
            Error::UnsupportedFeature(format!("WAL record cannot be recovered: {error}"))
        })?;
        let mut file = OpenOptions::new()
            .append(true)
            .open(self.path("wal", generation, "log"))?;
        let write = (|| -> std::io::Result<()> {
            file.write_all(&bytes[..HEADER])?;
            failpoint("wal_header");
            file.write_all(&bytes[HEADER..bytes.len() - END.len()])?;
            failpoint("wal_payload");
            file.write_all(END)?;
            failpoint("wal_commit");
            inject_io_error("wal_sync")?;
            file.sync_all()?;
            failpoint("wal_sync");
            Ok(())
        })();
        write.map_err(Error::CommitUnknown)
    }

    pub fn checkpoint(&self, state: &PublishedState, old_generation: u64) -> Result<()> {
        let old: Manifest = read_document(&self.directory.join("MANIFEST"))?;
        if old.generation != old_generation {
            return Err(Error::Corrupt(
                "checkpoint generation changed under commit lock".into(),
            ));
        }
        let mut obsolete = old.obsolete;
        obsolete.retain(|g| {
            [
                ("catalog", "snapshot"),
                ("data", "snapshot"),
                ("wal", "log"),
            ]
            .iter()
            .any(|(p, s)| self.path(p, *g, s).try_exists().unwrap_or(true))
        });
        obsolete.push(old_generation);
        let generation = old_generation
            .checked_add(1)
            .ok_or_else(|| Error::InvalidDefinition("checkpoint generation exhausted".into()))?;
        self.install_checkpoint(state, old.database_id, generation, obsolete)
    }

    fn install_checkpoint(
        &self,
        state: &PublishedState,
        database_id: [u8; 16],
        generation: u64,
        obsolete: Vec<u64>,
    ) -> Result<()> {
        write_document(
            &self.path("catalog", generation, "snapshot"),
            &Snapshot {
                database_id,
                version: FORMAT,
                generation,
                commit_seq: state.commit_seq,
                value: &state.catalog,
            },
        )?;
        failpoint("checkpoint_catalog");
        write_document(
            &self.path("data", generation, "snapshot"),
            &Snapshot {
                database_id,
                version: FORMAT,
                generation,
                commit_seq: state.commit_seq,
                value: &state.storage,
            },
        )?;
        failpoint("checkpoint_data");
        write_document(
            &self.path("wal", generation, "log"),
            &LogRecord::Header {
                version: FORMAT,
                database_id,
                generation,
                base_seq: state.commit_seq,
            },
        )?;
        sync_directory(&self.directory)?;
        let manifest = Manifest {
            database_id,
            version: FORMAT,
            generation,
            commit_seq: state.commit_seq,
            next_id: state.next_id,
            obsolete,
        };
        let temp = self.directory.join("MANIFEST.tmp");
        write_document(&temp, &manifest)?;
        fs::rename(temp, self.directory.join("MANIFEST"))?;
        failpoint("manifest_rename");
        sync_directory(&self.directory)?;
        failpoint("manifest_sync");
        self.cleanup(&manifest);
        Ok(())
    }

    fn cleanup(&self, manifest: &Manifest) {
        // Retirement remains in the manifest until a later checkpoint; failed unlink is retryable.
        for generation in &manifest.obsolete {
            if *generation >= manifest.generation {
                continue;
            }
            for (prefix, suffix) in [
                ("catalog", "snapshot"),
                ("data", "snapshot"),
                ("wal", "log"),
            ] {
                let _ = fs::remove_file(self.path(prefix, *generation, suffix));
                failpoint("cleanup");
            }
        }
        let _ = sync_directory(&self.directory);
    }

    fn path(&self, prefix: &str, generation: u64, suffix: &str) -> PathBuf {
        self.directory
            .join(format!("{prefix}-{generation}.{suffix}"))
    }
}

fn lock_file(path: &Path) -> Result<File> {
    Ok(OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?)
}
fn sync_directory(path: &Path) -> Result<()> {
    Ok(File::open(path)?.sync_all()?)
}
fn check_version(version: u32) -> Result<()> {
    if version != FORMAT {
        return Err(Error::UnsupportedFeature(format!(
            "database format {version}"
        )));
    }
    Ok(())
}

fn write_document<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let frame = encode_frame(&serde_json::to_vec(value)?)?;
    let mut file = File::create(path)?;
    file.write_all(&frame)?;
    file.sync_all()?;
    Ok(())
}
fn read_document<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let mut file = File::open(path)?;
    let payload = read_frame(&mut file, false)?
        .ok_or_else(|| Error::Corrupt("empty checkpoint document".into()))?;
    if file.stream_position()? != file.metadata()?.len() {
        return Err(Error::Corrupt("trailing checkpoint bytes".into()));
    }
    Ok(serde_json::from_slice(&payload)?)
}

fn encode_frame(payload: &[u8]) -> Result<Vec<u8>> {
    if payload.len() > MAX_RECORD {
        return Err(Error::UnsupportedFeature(
            "record exceeds 64 MiB format limit".into(),
        ));
    }
    let mut bytes = Vec::with_capacity(HEADER + payload.len() + END.len());
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&crc32(&bytes).to_le_bytes());
    bytes.extend_from_slice(&crc32(payload).to_le_bytes());
    bytes.extend_from_slice(payload);
    bytes.extend_from_slice(END);
    Ok(bytes)
}

fn read_frame(file: &mut File, allow_tail: bool) -> Result<Option<Vec<u8>>> {
    let position = file.stream_position()?;
    let remaining = file
        .metadata()?
        .len()
        .checked_sub(position)
        .ok_or_else(|| Error::Corrupt("invalid file position".into()))?;
    if remaining == 0 {
        return Ok(None);
    }
    if remaining < HEADER as u64 {
        if allow_tail {
            return Ok(None);
        }
        return Err(Error::Corrupt("torn document header".into()));
    }
    let mut header = [0; HEADER];
    file.read_exact(&mut header)?;
    if &header[..8] != MAGIC
        || crc32(&header[..16]) != u32::from_le_bytes(header[16..20].try_into().unwrap())
    {
        return Err(Error::Corrupt("invalid frame header".into()));
    }
    let length = u64::from_le_bytes(header[8..16].try_into().unwrap());
    if length > MAX_RECORD as u64 {
        return Err(Error::Corrupt("frame length exceeds limit".into()));
    }
    if remaining < HEADER as u64 + length + END.len() as u64 {
        if allow_tail {
            file.seek(SeekFrom::End(0))?;
            return Ok(None);
        }
        return Err(Error::Corrupt("torn checkpoint document".into()));
    }
    let mut payload = vec![0; length as usize];
    file.read_exact(&mut payload)?;
    let mut end = [0; 8];
    file.read_exact(&mut end)?;
    if &end != END || crc32(&payload) != u32::from_le_bytes(header[20..24].try_into().unwrap()) {
        return Err(Error::Corrupt("invalid frame body or commit marker".into()));
    }
    Ok(Some(payload))
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in bytes {
        crc ^= *byte as u32;
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb88320 & (0u32.wrapping_sub(crc & 1)));
        }
    }
    !crc
}

pub(crate) fn failpoint(_name: &str) {
    #[cfg(test)]
    if std::env::var("GRAPHFUSION_TEST_CRASH").as_deref() == Ok(_name) {
        std::process::exit(86);
    }
}

fn inject_io_error(_name: &str) -> std::io::Result<()> {
    #[cfg(test)]
    if std::env::var("GRAPHFUSION_TEST_IO").as_deref() == Ok(_name) {
        return Err(std::io::Error::other("injected I/O failure"));
    }
    Ok(())
}
