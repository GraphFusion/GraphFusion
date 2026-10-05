---
title: "Persistent databases"
description: "Durable WAL commits, resident MemTables and threshold-based Parquet sealing."
sidebar:
  order: 2
---

`Database::new()` creates an in-memory database. `Database::open(path, OpenOptions { create_if_missing: true })` creates or opens a durable directory. Default options require an existing database.

```sh
cargo run -p graphfusion --locked -- run --database ./demo-db --create \
  --query 'CREATE GRAPH social ANY GRAPH'
```

Writes first update private Arrow MemTables. At commit, a checksummed WAL record stores the new batches as Arrow IPC, fragment references, deletion masks, catalog changes and identity counters together. The WAL is synchronized before the new snapshot becomes visible or COMMIT succeeds. Recovery reconstructs unsealed MemTables from committed records; a successful write does not require a Parquet file.

Each graph seals its resident fragments when either **65,536 rows** or **16 MiB of Arrow array memory** is reached. Sealing is synchronous at commit: resident rows become database-owned immutable Parquet files, files and their directory are synchronized, then the WAL publishes the new layout. Previously sealed files are reused. Queries read both sealed files and resident batches through DataFusion. Updates to sealed rows log deletion masks and replacement rows; deletes preserve the original file until reclamation is safe.

Configure the thresholds for all handles sharing the database coordinator:

```rust
use graphfusion::StorageOptions;
database.configure_storage(StorageOptions {
    memtable_max_rows: 32_768,
    memtable_max_bytes: 8 * 1024 * 1024,
})?;
```

Both thresholds must be nonzero. They apply per graph to the published resident layout, not to process RSS, active historical snapshots or private transactions. Configuration is process-local and should be reapplied after reopening. A transaction may exceed a threshold privately; its final layout seals at COMMIT. ROLLBACK discards it without writing intermediate Parquet files.

Readers keep a consistent catalog/storage snapshot while a statement materializes results. Explicit transactions retain their snapshot between calls, including immutable MemTable batches. Branches persist framed incremental commit records and atomically move their refs; active branch snapshots are cached in the owning process. Main-branch snapshots for forking/listing are created lazily, rather than copied on every WAL commit.

The supported environment is Linux/macOS on a local filesystem with OS file locks, atomic rename and file/directory synchronization. Only one process may open a database directory at a time; a second process receives `Error::DatabaseInUse` immediately. Within the owning process, repeated opens of the same canonical path share one coordinator, and cloned database handles with independent sessions support concurrent threads. The exclusive OS lock remains held until the last database, session or transaction handle is dropped, and is released if the process exits or is killed. Network filesystems, object storage and inherited open handles after fork are not supported.

## Recovery and format

Recovery reads the manifest/checkpoint and committed WAL, validates referenced graph files, and restores a consistent generation before accepting work. Staged files that were never published do not become visible graph data.

After opening the directory, the owning process shares its published `main` snapshot between statements. Commits and ID reservations update this state under the coordinator's lock; successful checkpoints advance its WAL generation. A failed checkpoint or post-WAL publication triggers recovery before the state is reused. Checksum and format validation of checkpoints and WAL occurs during recovery, rather than on every statement.

The current format is v4. Older formats are rejected; there is no built-in migration path. Do not edit or delete database files while a database is open. This documentation does not promise compatibility across future format changes.

[Checkpointing](/storage/checkpoint/) forces remaining main MemTables to seal, rotates the WAL and reclaims obsolete generations when no active snapshot can use them. There is no automatic checkpoint scheduler.
