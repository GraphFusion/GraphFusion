---
title: "Architecture"
description: "How parsing, DataFusion execution and transactional storage fit together."
sidebar:
  order: 1
---

GraphFusion is a Cargo workspace with two main responsibilities:

- `crates/gql-parser`: lexer, AST, parser and grammar tests. It has no catalog, storage or execution ownership.
- `crates/graphfusion`: database/session facade, catalog/type binding, DataFusion plans, graph storage and commit coordination.

## Query path

GQL text → AST → name/type binding → DataFusion logical plans → optimizer → physical plans → Arrow batches.

The SQL frontend is disabled. Scalar expressions become DataFusion expressions; graph patterns become scans and joins. Quantified paths use RecursiveQuery/CteWorkTable plans, list expressions and window ranking. There is no separate Rust adjacency traversal engine.

OPTIONAL MATCH, path composition and writes sometimes materialize intermediate tables to preserve input identity, schemas or mutation boundaries. Returned plan diagnostics include the work at those barriers. The current implementation favors correct semantics over specialized traversal indexes or frontier pruning.

## Published state and commit

One published snapshot combines a commit sequence, immutable catalog root, storage root and identity reservation watermark. Statements and explicit transactions register active snapshots and keep private changes. Catalog maps use copy-on-write entries; graph generations contain immutable resident Arrow fragments and sealed Parquet files, with ID/row-position indexes, edge adjacency lookups and snapshot-specific deletion bitmaps. New Parquet files have checksummed immutable index sidecars; Arrow buffers stay shared when masking resident rows.

At write commit, optimistic validation checks graph versions and catalog object/name/membership/dependency reads against the newest committed state. A validated delta merges into that state, preserving disjoint changes. Persistent publication appends and syncs a checksummed WAL record containing Arrow IPC deltas and layout changes, then exposes the new snapshot. When a graph reaches its MemTable threshold, sealing stages and syncs new fragment files before the committing WAL record references them. Catalog and graph data are not committed independently.

Opening acquires an exclusive nonblocking lifecycle lock retained by the database coordinator until its last handle drops. The canonical-path registry shares that coordinator across threads. Operations take the process-local state mutex, then the commit lock; query execution runs outside the state mutex. Opening recovers the latest durable generation under the commit lock. Subsequent statements share the published snapshot; failed checkpoint/publication triggers recovery before reuse. A live explicit transaction retains its starting snapshot rather than refreshing every statement.

## Storage lifecycle

Recovery establishes a durable manifest generation before accepting new writes. Checkpoint holds the state mutex, refuses active snapshots, forces main MemTables to seal, publishes a synced checkpoint/WAL generation and reclaims files no active snapshot can use. Fixed coordinating lock files must keep stable inodes while the database is open.

The internal row-key storage participant is used to verify joint commit behavior; it is not the graph query engine. Current user-visible graph tables are Arrow or Parquet.

## Boundaries for future work

Typed graph execution, additional expressions/procedures, prepared or streaming lifetimes, more complete unbounded path planning, background sealing, deletion-mask compaction, indexes and a normative ISO conformance audit remain future work. They are not prerequisites hidden behind a claim that those features already work. The [support matrix](/start/status/) tracks the actual surface.
