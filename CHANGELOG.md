# Changelog

## Unreleased

## 0.1.0 — 2026-09-27

First release of the GraphFusion database and CLI.

- Execute GQL graph queries through DataFusion, including optional matches,
  quantified/shortest paths, element references, aggregation, and query composition.
- Insert, update, and delete graph data with statement atomicity and explicit
  transactions.
- Manage catalog objects and isolated sessions, return Arrow batches, and persist
  local databases with Parquet, a write-ahead log, and checkpoints.
- Provide a Rust embedding API, CLI, runnable examples, and searchable documentation.
- Publish the accompanying parser as `gql-parser 0.2.0`, including its renamed
  `gql_parser` library and updated AST/grammar. Parser versions are independent.

Rust 1.94 or newer is required. Durable storage supports Linux and macOS on local
filesystems. Full ISO GQL conformance and all syntax accepted by the parser are
not yet supported by the execution engine; see the
[support matrix](https://graphfusion.github.io/start/status/) and
[limits](https://graphfusion.github.io/storage/limits/).
