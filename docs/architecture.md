# GraphFusion architecture

Status: proposed implementation design. This document describes the destination;
the [delivery plan](roadmap.md) distinguishes it from implemented behavior.

## Objective and boundaries

GraphFusion implements GQL over Apache DataFusion. GraphFusion owns GQL parsing,
semantic analysis, graph catalog, graph-to-relational planning, and transaction
coordination. DataFusion owns relational optimization, physical planning,
execution, memory accounting, parallelism, and supported data-source access.
Initial graph storage uses Arrow batches and Parquet through DataFusion providers.
A bespoke graph row store or a separate interpreter is not the execution path.

The language target is ISO/IEC 39075:2024. Track published corrigenda separately
when incorporating their changes. A parser accepting a construct does not establish
semantic support or conformance. Unsupported constructs must produce an explicit
error before the statement publishes effects. No full-conformance claim is made
until the normative requirements and feature identifiers have been audited against
tests. The public ISO description alone cannot establish those semantics.

## Request lifecycle

```mermaid
flowchart LR
  text[GQL text] --> parser[Lexer and AST]
  parser --> binder[Name and type binding]
  snapshot[Catalog and graph snapshot] --> binder
  binder --> graph[Bound graph plan]
  graph --> logical[DataFusion LogicalPlan]
  logical --> execution[DataFusion optimizer and executor]
  providers[Arrow and Parquet TableProviders] --> execution
  execution --> result[Arrow RecordBatch results]
  execution --> writes[Private mutation batches]
  writes --> commit[Validate and publish snapshot]
```

1. Parse the entire submitted program. Retain identifiers, scopes, graph selectors,
   match modes, types, and statement boundaries without silently approximating them.
2. Open one statement snapshot and resolve schema/graph references to stable IDs.
   Bind variables, properties, functions and parameter types; reject unsupported
   clauses and invalid combinations. A snapshot guard must outlive execution.
3. Build a graph plan with explicit bindings and graph identity. Lower supported
   operators to DataFusion `LogicalPlan`/`Expr` APIs, without concatenating SQL.
4. Execute using a DataFusion session associated with the graph snapshot. Return
   Arrow batches plus diagnostics and affected-object counts. Expose logical and
   physical plans for debugging and verify them in tests.
5. For mutations, materialize private candidate data, validate constraints and read
   dependencies, then publish catalog metadata and data references atomically.
   A failure before commit leaves no visible changes from that statement.

The facade should offer an asynchronous query API. Existing synchronous catalog
DDL can remain available; no nested Tokio runtime or hidden blocking query loop.
Initially materialize query results while holding the snapshot. A streaming API
must move the snapshot guard into the stream and release it on completion or drop.

## Graph representation and providers

A graph has an immutable graph ID and a storage generation. Node and edge IDs are
unique within the graph, survive reopening, and are never inferred from row order.
Element identity includes the graph ID and element kind. Edge endpoints refer to
node IDs in the same graph. Parallel edges and self-loops are valid data.

Start with typed node and edge tables, using reserved physical columns for IDs,
source, destination, and directedness. User properties keep separate physical
names, so a property cannot overwrite an endpoint or ID. Logical labels/types map
to provider partitions; multi-label nodes have one identity, regardless of how
many labels match. Open graph evolution needs an explicit schema union strategy
and must distinguish an absent property from a present null property where GQL
requires it. Do not silently stringify mixed property types.

`MemTable`/Arrow batches provide deterministic in-memory fixtures. Parquet tables
provide durable graph data. CSV and JSON can be imported or bound as external
read-only sources through DataFusion; importing validates IDs, endpoint integrity,
labels, and property types. A provider adapter connects catalog object IDs to
DataFusion tables and pins a specific version rather than mutating a global
table registration while queries are running.

Existing experimental catalog persistence may keep its metadata log. Its opaque
test-row participant only exercises atomicity; graph query data must use the
Arrow/Parquet provider path. Evolve the transaction participant to commit immutable
file manifests rather than serialize an entire graph into a JSON WAL.

## Graph-to-relational planning

| GQL operation | Planned DataFusion lowering and semantic checks |
| --- | --- |
| Node pattern | Table scan, label predicate, property predicates, fresh binding |
| Directed edge | Edge scan joined to source/destination node scans by IDs |
| Incoming/undirected edge | Reverse endpoint binding or directional union; avoid duplicate self-loop matches |
| Repeated variable | Identity equality on the existing binding, never independent rebinding |
| Disconnected patterns | Cross join, preserving result multiplicity |
| Multiple edge bindings | Enforce the selected match mode, including edge identity restrictions |
| Optional match | Left join over the entire optional pattern; keep its predicates inside the optional side |
| Filter/projection | Typed DataFusion expressions with GQL null and value semantics |
| Aggregation/distinct | DataFusion aggregate/distinct, with GQL grouping and empty-input behavior |
| Order/offset/limit | DataFusion sort/limit, with validated counts and explicit null ordering |
| Composite queries | Union/intersection/difference with verified multiplicity and type alignment |
| Quantified paths | Bounded expansion first; recursive or custom DataFusion extension plans as needed |

Path expansion must implement WALK/TRAIL/SIMPLE/ACYCLIC constraints and path
search selectors independently of a generic recursive join. Zero-length paths,
cycles, duplicate edges, and shortest-path ties require dedicated tests. Extension
operators still use DataFusion's execution interfaces; an unrelated executor is
not an acceptable shortcut.

The binder owns a symbol table for element and scalar bindings. Bound properties
carry Arrow types and nullability. GQL-specific functions or values become typed
UDFs/extension expressions where ordinary DataFusion expressions are insufficient.
Overflow, unsupported casts, temporal zones, graph identity, and three-valued
logic need explicit decisions and tests, not accidental SQL compatibility.

## Catalog, transactions, and durable publication

Catalog entries model directories, schemas, graph types, and graphs. Name binding
preserves decoded identifiers and resolves relative paths against session state.
Drop/recreate yields new IDs; stale references do not bind to a new object. Named
graph types record dependencies, and data writes enforce type/endpoint constraints.

The statement coordinator publishes one catalog-and-data version. Readers pin
immutable versions. Writers stage changes, validate conflicts under a commit
lock, durably write new Parquet files, and publish their manifest together with
catalog changes. Unreferenced files from failed commits are reclaimable only after
recovery proves them unreachable. Old files remain until all older snapshots are
released. Cross-process tests and crash injection must cover publication and GC.

Autocommit is the first runtime milestone. Explicit START TRANSACTION / COMMIT /
ROLLBACK requires staged session and data state, read-your-writes, access modes,
conflict detection, and rollback. Until implemented, reject explicit transactions
before executing their submitted program's effects. Document actual isolation;
do not claim serializability based only on write-write conflict checks.

## Observable database behavior

Provide a CLI for a database directory, file/stdin GQL programs, tabular results,
diagnostics, and nonzero failure exits. Commit sequence, planning/execution time,
scan/join metrics, and DataFusion explain plans support investigation. Cancellation,
memory limits, malformed input, and bounded diagnostics are required before a
production-readiness claim. Parameter values must never be substituted into query
text or appear in unrequested logs.

## Verification and delivery contract

Every feature PR includes executable positive and negative semantics, meaningful
edge cases, and an update to the capability ledger. Tests compare unordered row
multisets unless ORDER BY is specified. They verify Arrow schemas as well as
values. Integration tests prove that plans execute via DataFusion and use actual
Parquet files for persistent paths. Crash tests verify joint metadata/data state.

Each task is a separate PR. Dependent PRs may be stacked and must name their base
dependency. The maintainer chooses yes/no for each merge; creating a PR does not
authorize merging or enabling auto-merge. After an approved merge, rebase dependent
PRs and rerun CI against their updated heads. Keep main usable at every merge.

## Sources

- [ISO/IEC 39075:2024](https://www.iso.org/standard/76120.html): language target
  and scope; full semantic conformance needs a normative audit.
- [DataFusion library guide](https://datafusion.apache.org/library-user-guide/index.html):
  expression, plan, provider, function and optimizer extension points.
- [DataFusion architecture](https://docs.rs/datafusion/latest/datafusion/):
  Arrow execution and the logical/physical planning pipeline.
