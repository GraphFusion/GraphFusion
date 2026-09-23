# Explicit GQL transactions

A session supports `START TRANSACTION [READ ONLY | READ WRITE]`, `COMMIT`, and
`ROLLBACK` through both `execute` and `run`. The default access mode is READ WRITE.
A transaction can span API calls; queries, catalog changes, graph mutations and
`replace_graph_data` all use the same snapshot and private changes until it ends.

```rust
let mut session = db.session();
session.execute("SESSION SET GRAPH social; START TRANSACTION")?;
session.run("INSERT (:Person {name: 'Alice'})").await?;
let pending = session.query("MATCH (n:Person) RETURN COUNT(*) AS people").await?;
assert!(pending.transaction_pending);
session.execute("COMMIT")?;
```

`examples/transactions.gql` creates two accounts, transfers 25 in one transaction,
rolls back another update, and reads balances 75/75 with total 150. Run it against
a new database directory:

```sh
cargo run -p graphfusion --locked -- run --database /tmp/gql-transactions --create \
  --file examples/transactions.gql
cargo run -p graphfusion --locked -- run --database /tmp/gql-transactions \
  --query "USE GRAPH ledger MATCH (a:Account) RETURN SUM(a.balance) AS total"
```

The CLI is a one-shot session. A file/query must end its explicit transaction;
otherwise the run reports an error and session destruction discards the pending
changes. Earlier autocommitted statements remain committed. Transaction commands
and provisional query results are labeled separately in CLI output.

## State and failure behavior

`Session::transaction_status()` returns Idle, Active or Failed. Active/Failed
include the access mode and starting snapshot sequence.

| State and command | Result |
| --- | --- |
| Idle + START TRANSACTION | Capture one snapshot; enter Active |
| Active + successful statement | Keep its private changes; stay Active |
| Active + COMMIT | Validate and durably publish all changes once; enter Idle |
| Active + ROLLBACK | Discard changes; enter Idle |
| Active + statement/parse/control error or cancelled query | Discard changes and snapshot lease; enter Failed |
| Failed + COMMIT/query/write | Error; no changes can be published |
| Failed + ROLLBACK | Acknowledge the abort; enter Idle |
| Any open session + SESSION CLOSE | Discard any transaction and close the session |

Dropping the session also discards an open transaction. COMMIT/ROLLBACK in Idle
report NoTransaction; START while a transaction is present reports TransactionActive
and fails the existing transaction. There is no nesting, savepoint or implicit
retry. Transaction controls and SESSION CLOSE cannot be NEXT continuations.
A batch stops at the first error. Statements outside explicit transactions
retain autocommit behavior, including survival of earlier successful statements.
The read-only `query` API participates in an active transaction but still accepts
exactly one read query; use `execute` or `run` for transaction control.

A statement guard owns the coordinator during asynchronous execution. If a query
future is dropped while DataFusion is planning/collecting, the guard discards
pending changes and marks the transaction Failed. This prevents an interrupted
query from returning a partly changed coordinator for a later COMMIT.

An I/O error during WAL append can mean COMMIT succeeded durably despite the
error. `CommitUnknown` poisons the database coordinator; existing active sessions
also reject further work. ROLLBACK/SESSION CLOSE can clear local session state but
cannot undo a commit with unknown outcome. Reopen the database and inspect the
recovered state before deciding what to retry.

## Isolation and access modes

The snapshot is captured at START, not at the first graph read. Every statement
uses that catalog/storage snapshot plus the transaction's own earlier writes.
Other sessions/processes see only committed snapshots. No statement refreshes an
active transaction to a newer snapshot.

Public GQL graph/catalog write transactions use optimistic serializable
validation. All graph reads (including empty matches), catalog object/name
lookups, absent-name lookups, membership scans and dependency scans accumulate
across statements. Commit checks these versions under the same cross-process
lock that publishes the delta. This prevents lost graph writes, graph phantoms,
catalog phantoms and cross-graph write skew. Data validation is at graph granularity:
two writers to different nodes of one graph can conflict. Disjoint graph writes
can merge when their accumulated read dependencies are still valid. Read-only
transactions finish against the consistent snapshot captured at their start.

READ ONLY rejects catalog DDL, graph mutations and the direct graph-import API,
even when a requested modification would affect zero rows. Enforcement occurs
both before statement execution and at coordinator mutation/ID-allocation entry
points. Session characteristics and driver parameters are allowed because they
do not publish database data.

Session characteristics are session state, not transactional catalog/data:
successful SESSION SET/RESET and driver parameter changes remain after ROLLBACK.
Each such statement is itself atomic. A graph/schema reference bound to an object
created by a subsequently aborted transaction becomes stale; recreating its name
cannot rebind the old identity. Select a live graph/schema or reset the relevant
characteristic before using it. A Failed transaction only permits ROLLBACK or
SESSION CLOSE; status inspection remains available.

The parser intentionally rejects SQL `ISOLATION LEVEL` syntax; GQL transaction
access modes are the implemented characteristics. General isolation configuration
and implementation-defined characteristics are not exposed. Full normative GQL
behavior and diagnostic-code coverage still require the standard audit.

## Results, durability and lifecycle

QueryResult and StatementResult have `transaction_pending`. When true, the result
was produced within an open explicit transaction and `commit_seq` is its starting
snapshot, not a commit acknowledging the writes. These returned values do not
change later; use the separate successful COMMIT result as the durability
acknowledgment. StatementResult also reports TransactionAction for START, COMMIT
and ROLLBACK (including a SESSION CLOSE that discards a transaction). A write
COMMIT returns the single new commit sequence; empty/read-only
COMMIT creates no WAL commit and returns the starting snapshot sequence.

`replace_graph_data` stages into an active transaction and returns its starting
snapshot sequence; without one it commits immediately as before. Database-level
`with_catalog`, `create_directory`, and `checkpoint` remain independent
administrative operations, not participants in a session's transaction.

Catalog changes, final graph manifests and element-ID watermarks publish in one
existing format-v3 commit record. Individual successful statements can stage
immutable Parquet files, but these files are not visible to other sessions until
COMMIT references them. Rollback, conflict and crash orphans are reclaimed by a
later checkpoint. Published element IDs are never reused; never-published element
IDs may be reused after abort. Object/file IDs are durably reserved and can have
gaps after aborts.

A lifecycle lease remains held between calls while a transaction is Active.
Checkpoint returns Busy while any process has such a lease, protecting old and
staged files. Successful commit, rollback, failure, cancellation, session close,
process exit and session destruction release it. Returned Arrow batches alone do
not pin files. There are no transaction timeouts or user savepoints yet. Long
transactions retain snapshots/files and accumulate change metadata; full-graph
rewrites, metadata-frame size bounds and the other current storage limits remain.

## Verification

Tests cover memory and Parquet read-your-writes, one-publication multi-statement
commit, catalog/data rollback, imports, read-only enforcement, session references,
parse/execution errors, cleanup, and aggregate results. A controlled DataFusion
provider pauses a real scan to verify cancellation independently of timing.

Independent OS processes test uncommitted visibility, same-graph conflict,
abandoned writers, and old read-only snapshots across Parquet replacement.
Crash/fault tests interrupt a transaction that changes two graphs and catalog
entries during file staging, WAL header/payload/completion/sync and publication;
recovery must expose all statements or none and recover the matching ID watermark.
A complete frame followed by a simulated fsync error tests CommitUnknown and
poisoned active sessions. CLI tests checkpoint and reopen the transfer example and
reject writes in read-only, failed, or unfinished transaction programs.

The [TuGraph GQL grammar](https://github.com/TuGraph-family/gql-grammar/blob/main/GQLParser.g4)
provides a primary grammar reference for transaction controls and access modes.
[Ultipa's transaction documentation](https://www.ultipa.com/docs/gql/transactions)
provides an implementation reference for explicit snapshots and read-your-writes;
GraphFusion's conflict validation and failure policy are described independently
above. Neither reference establishes complete normative ISO conformance.
