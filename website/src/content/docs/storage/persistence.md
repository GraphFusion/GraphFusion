---
title: "Persistent databases"
description: "Open a local database directory with joint catalog and Parquet commits."
sidebar:
  order: 2
---

`Database::new()` creates an in-memory database. `Database::open(path, OpenOptions { create_if_missing: true })` creates or opens a durable directory. Default options require an existing database.

```sh
cargo run -p graphfusion --locked -- run --database ./demo-db --create \
  --query 'CREATE GRAPH social ANY GRAPH'
```

Durable graph changes stage immutable Parquet files and publish their metadata with catalog changes in one checksummed logical WAL commit. Readers keep a consistent catalog/storage snapshot while a statement materializes results. Explicit transactions retain their snapshot between calls.

The supported environment is Linux/macOS on a local filesystem with OS file locks, atomic rename and file/directory synchronization. Only one process may open a database directory at a time; a second process receives `Error::DatabaseInUse` immediately. Within the owning process, repeated opens of the same canonical path share one coordinator, and cloned database handles with independent sessions support concurrent threads. The exclusive OS lock remains held until the last database, session or transaction handle is dropped, and is released if the process exits or is killed. Network filesystems, object storage and inherited open handles after fork are not supported.

## Recovery and format

Recovery reads the manifest/checkpoint and committed WAL, validates referenced graph files, and restores a consistent generation before accepting work. Staged files that were never published do not become visible graph data.

After opening the directory, the owning process shares its published `main` snapshot between statements. Commits and ID reservations update this state under the coordinator's lock; successful checkpoints advance its WAL generation. A failed checkpoint or snapshot publication triggers recovery before the state is reused. Checksum and format validation of checkpoints and WAL occurs during recovery, rather than on every statement.

The current format is v3. Older formats are rejected; there is no built-in migration path. Do not edit or delete database files while a database is open. This documentation does not promise compatibility across future format changes.

[Checkpointing](/storage/checkpoint/) reclaims obsolete generations when no active snapshot can use them. There is no automatic checkpoint scheduler.
