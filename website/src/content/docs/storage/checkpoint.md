---
title: "Checkpoint and reclamation"
description: "Compact catalog recovery state and reclaim obsolete graph files."
sidebar:
  order: 5
---

A checkpoint seals remaining main-branch MemTables to Parquet, writes a new durable snapshot and WAL generation, then permits obsolete storage generations to be reclaimed. File IDs are durably reserved before sealing; recovery uses either the old WAL or the new checkpoint if the process stops during this operation. The database must have no active statement or transaction snapshots in its owning process.

```sh
cargo run -p graphfusion --locked -- checkpoint --database ./demo-db
```

Close the interactive REPL before using this separate command, or run `\checkpoint` inside the REPL. A second process cannot open the same database while the REPL owns it.

`Database::checkpoint()` returns `Error::Busy` when a statement or explicit transaction pins a snapshot. Schedule retries at idle points. Simply keeping already materialized Arrow result batches does not hold the snapshot lease.

DROP and graph replacement retire storage; they do not immediately delete files that a reader might still scan. Reclamation happens after the new manifest is durable and no active lease can reference the old generation. Orphan files from interrupted staging can be reclaimed as well.

Checkpoint is not Parquet compaction, an index build or a backup command. There is no automatic scheduler or supported live-directory copying backup protocol yet.
