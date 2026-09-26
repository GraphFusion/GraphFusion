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

One published snapshot combines a commit sequence, immutable catalog root, storage root and identity reservation watermark. Statements and explicit transactions keep a lifecycle lease plus private changes. Catalog maps use copy-on-write entries; graph generations are immutable Arrow buffers or Parquet files.

At write commit, optimistic validation checks graph versions and catalog object/name/membership/dependency reads against the newest committed state. A validated delta merges into that state, preserving disjoint changes. Persistent publication stages and syncs graph files, appends a checksummed WAL record, then exposes the new snapshot. Catalog and graph data are not committed independently.

Lock order is lifecycle lease, process-local state mutex, then cross-process commit lock. Opening and autocommit/transaction start recover the latest durable generation under the commit lock. A live explicit transaction retains its starting snapshot rather than refreshing every statement.

## Storage lifecycle

Recovery establishes a durable manifest generation before accepting new writes. Checkpoint takes an exclusive lifecycle lease, publishes a synced checkpoint/WAL generation and reclaims files no active snapshot can use. Fixed coordinating lock files must keep stable inodes while the database is open.

The internal row-key storage participant is used to verify joint commit behavior; it is not the graph query engine. Current user-visible graph tables are Arrow or Parquet.

## Boundaries for future work

Typed graph execution, additional expressions/procedures, prepared or streaming lifetimes, more complete unbounded path planning, incremental storage, indexes and a normative ISO conformance audit remain future work. They are not prerequisites hidden behind a claim that those features already work. The [support matrix](/GraphFusion/start/status/) tracks the actual surface.
