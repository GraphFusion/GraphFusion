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

The supported environment is Linux/macOS on a local filesystem with OS file locks, atomic rename and file/directory synchronization. Cooperating processes can share the same database path. Network filesystems, object storage and inherited open handles after fork are not supported.

## Recovery and format

Recovery reads the manifest/checkpoint and committed WAL, validates referenced graph files, and restores a consistent generation before accepting work. Staged files that were never published do not become visible graph data.

The current format is v3. Older formats are rejected; there is no built-in migration path. Do not edit or delete coordinating lock files while a database is open. This documentation does not promise compatibility across future format changes.

[Checkpointing](/GraphFusion/storage/checkpoint/) reclaims obsolete generations when no active snapshot can use them. There is no automatic checkpoint scheduler.
