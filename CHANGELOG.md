# Changelog

## 0.2.0 — Unreleased

- Record immutable commit snapshots and named branch refs. A branch starts at
  the current `main` snapshot; later commits move only that ref, and checkpoint
  keeps Parquet files still referenced by another branch.
- Fix parser crashes and literal handling: cap nesting, give `NOT` comparison
  precedence, accept `i64::MIN` and decimal exponents, reject a name glued
  onto a number, and keep quoted parameter
  names. Repeated graph writes append to the existing table and keep unchanged
  Parquet files. `run` prints statements that already committed, and a non-readable
  parent directory no longer blocks reopening a database.
- Add `graphfusion run --dump-ast` to inspect GQL query and file syntax trees
  without executing statements or opening a persistent database.
- Launch an interactive session with `graphfusion [DIR]`; open or create a local
  database directory, or omit it for in-memory use. Includes multiline GQL,
  history, cancellation, transaction prompts and file execution.
- Breaking: persistent database directories are owned exclusively by one process
  until its last handle drops. Threads in that process share the coordinator and
  execute through independent sessions; other processes receive `DatabaseInUse`.
  Recovery after `CommitUnknown` requires dropping all database/session handles.

- Breaking: rename the parser's Rust library from `graphfusion_gql_parser` to
  `gql_parser`, following Cargo's default for the `gql-parser` package. Update
  Rust import paths when upgrading; the package name remains `gql-parser`.

## 0.1.1 — 2026-09-27

First release of the GraphFusion database and CLI.

- Execute GQL graph queries through DataFusion, including optional matches,
  quantified/shortest paths, element references, aggregation, and query composition.
- Insert, update, and delete graph data with statement atomicity and explicit
  transactions.
- Manage catalog objects and isolated sessions, return Arrow batches, and persist
  local databases with Parquet, a write-ahead log, and checkpoints.
- Provide a Rust embedding API, CLI, runnable examples, and searchable documentation.
- Publish both crates at 0.1.1 and preserve the parser library name
  `graphfusion_gql_parser` used by existing consumers.
- Fix parsing of nested query results and parenthesized path patterns.

Rust 1.94 or newer is required. Durable storage supports Linux and macOS on local
filesystems. Full ISO GQL conformance and all syntax accepted by the parser are
not yet supported by the execution engine; see the
[support matrix](https://graphfusion.github.io/start/status/) and
[limits](https://graphfusion.github.io/storage/limits/).
