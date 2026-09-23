# Delivery plan and capability ledger

The destination is an executable, persistent GQL graph database built on
DataFusion, followed through to the standard coverage audit. A read-only demo,
parser-only grammar coverage, or green CI is not completion of that objective.

Each row below is a separately reviewed PR task; split a row further when its
implementation cannot be reviewed coherently. Sequence numbers express dependencies,
not a promise that later work is already implemented.

| Task | Depends on | Deliverable and acceptance evidence |
| --- | --- | --- |
| 01 Architecture | Existing parser | Component ownership, storage model, semantics, review workflow and this ledger |
| 02 Continuous integration | Existing parser | PR/push format, strict Clippy, Linux/macOS workspace tests and minimum Rust checks; observe actual Actions results |
| 03 Catalog and sessions | 01, 02 | Stable object IDs, graph/schema/type DDL, session isolation, reopen, conflicts and crash recovery tests |
| 04 DataFusion query foundation | 02, 03 | Direct logical-plan construction, scalar results/parameters, Arrow output and explicit unsupported-feature errors |
| 05 Graph provider and MATCH | 04 | Validated node/edge Arrow tables; labeled node scans, directed one/multi-hop joins, properties and identity tests |
| 06 Parquet and CLI | 05 | Read actual Parquet graph tables through DataFusion; run a GQL file from a fresh CLI process and compare results |
| 07 Graph mutations | 05, 06 | INSERT/SET/REMOVE/DELETE, constraint checks and atomic immutable data publication; reopen and fault tests |
| 08 Relational GQL semantics | 05 | Optional/disconnected matches, LET/FOR, aggregates, grouping, sorting and composite queries with null/multiplicity tests |
| 09 Explicit transactions | 07 | Access modes, commit/rollback, read-your-writes, concurrent sessions/processes, and isolation documentation |
| 10 Paths | 05, 08 | Quantifiers, modes, selectors, path values, cyclic data and shortest-path ties; resource limits |
| 11 Expressions, scopes and procedures | 04, 08 | Remaining scalar/type families, nested queries, EXISTS, CALL, NEXT and session binding tables |
| 12 Catalog/type completeness | 07, 09, 11 | Copy operations, typed/open graph evolution, remaining schema/graph types and dependencies |
| 13 Operational hardening | 06–12 | Cancellation, memory bounds, explain/metrics, import validation, recovery/GC stress, deterministic workload benchmarks |
| 14 Standard coverage audit | All preceding tasks | Clause/feature inventory with normative references and linked parser/binder/executor/error tests; resolve all requirements before claiming conformance |

## Current evidence at design time

The main branch at `f6c8ad0` contains a GQL parser and a parse-only facade, with
158 passing parser integration tests. There is no DataFusion dependency, graph
query execution, CLI, or Actions workflow on that revision. The working catalog
implementation is proposed separately and must not be described as merged.

| Feature family | Parser on main | Runtime on main | Required runtime evidence |
| --- | --- | --- | --- |
| Catalog DDL / session commands | Present | Missing | Catalog/session and recovery tests |
| Scalar expressions and result clauses | Broad syntax | Missing | Typed Arrow results and negative type tests |
| MATCH / graph selection | Broad syntax | Missing | Provider scans/joins, identity and direction tests |
| Optional match / grouping / set operations | Present | Missing | Null, duplicate and empty-input cases |
| INSERT / SET / REMOVE / DELETE | Present | Missing | Atomic data writes, constraints and restart |
| Transactions | Present | Missing | Commit/rollback, conflict and access-mode tests |
| Quantified paths and path search | Broad syntax | Missing | Cycle, mode, bounds and tie cases |
| Procedures and nested queries | Present | Missing | Scope, correlation and result binding tests |
| Durable graph storage | N/A | Missing | DataFusion Parquet round trips and process restart |

“Present” describes syntax acceptance, not an exhaustive grammar audit. Update
individual rows with PR links and exact commands as implementations land. Keep
unsupported cases documented and tested; do not turn expected failures into
silently incomplete results.

## Executable-database acceptance scenario

This scenario is a runtime milestone, not the full standard completion audit:

1. Build a fresh checkout with the documented toolchain and locked dependencies.
2. Use the CLI to create a schema, graph type and graph in a new database directory.
3. Insert nodes and directed/undirected edges with typed properties, including a
   self-loop and parallel edges; run parameterized MATCH queries.
4. Verify filters, joins, optional matches, aggregation, ordering and paths against
   checked-in expected results. Explain must show the DataFusion execution path.
5. Update/remove properties and delete elements, rejecting invalid endpoints or
   type violations without partial writes. Verify transaction rollback and conflicts.
6. Restart in a separate process and verify graph metadata and query results from
   Parquet-backed storage. Exercise a crash before and after durable publication.
7. Run the same reproducible workload in CI, alongside parser, binder, negative
   execution, recovery and cross-platform tests.

Completion additionally requires the standard coverage audit, maintained public
API/CLI documentation, all required CI checks passing on final commits, and
maintainer decisions on the implementation PRs. Unmerged work remains work in
progress even if its local tests pass.
