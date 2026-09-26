# DataFusion query execution

`Session::query(&mut self, gql).await` executes one read-only GQL query and returns
Arrow `RecordBatch` values. It builds DataFusion logical plans and expressions
directly, then uses DataFusion's optimizer, physical planner and executor. The SQL
frontend feature is disabled. Catalog/session commands use `Session::execute`.

```rust
use graphfusion::{Database, Value};

let db = Database::new();
let mut session = db.session();
session.set_parameter("n", Value::Integer(6))?;
let result = session.query(
    "LET answer = $n * 7 FILTER answer > 40 RETURN answer"
).await?;
assert_eq!(result.row_count(), 1);
```

Run the complete example with `cargo run -p graphfusion --example scalar --locked`.
The caller supplies the async runtime. GraphFusion does not create a nested
runtime or block on an asynchronous query from its synchronous DDL method.

## Current scope

- Single RETURN/SELECT queries, ALL/DISTINCT, untyped LET bindings, FILTER, output
  aliases and wildcards over bound variables.
- Int64, Float64, boolean, string, byte and null scalar literals/parameters;
  UNKNOWN is a nullable boolean. The existing parser represents decimal literals
  as f64, so this is not an exact-decimal implementation.
  Session parameter declarations preserve these type families, including typed
  nulls and integer initializers of FLOAT parameters. Physical integer and float
  widths are currently normalized to Int64/Float64 after session validation.
- Arithmetic, numeric comparison, compatible-type equality/comparison, boolean
  operators (including XOR), string concatenation, null/truth predicates,
  COALESCE and NULLIF. GQL binding rejects implicit string-to-number and
  boolean-to-number coercions before DataFusion planning.
  Arithmetic uses checked Arrow kernels through immutable DataFusion UDFs, so
  integer overflow, division by zero and non-finite floating results raise errors
  in both constant expressions and batch execution.
- Ordering over available bindings/result aliases, explicit null ordering, and
  nonnegative integer OFFSET/LIMIT counts, including session parameters. Default
  ordering treats null as greatest. Unaliased scalar expressions receive names
  `column_1`, `column_2`, etc.; a variable retains its name.

Graph selection, MATCH and graph values, graph mutations, aggregation, composite
queries, nested/procedure queries, typed LET, temporal expressions, casts and
remaining expressions are not yet executed. Unsupported features produce an
error; they are not discarded or interpreted as SQL. The query API rejects mixed
programs and multiple top-level statements before any command can execute.

## Results and snapshots

`QueryResult` includes the Arrow schema even for empty results, batches, snapshot
commit sequence, the original logical plan and the optimized physical plan.
Plan text is diagnostic rather than a stable serialization format and can contain
bound parameter values; it is returned to the caller and is not logged.
`graphfusion::arrow` re-exports the matching Arrow version for consumers.

The statement lease remains alive across physical planning and collection.
Materialization finishes before the lease is released, so a caller holding batches
does not pin catalog files. Failed queries release their leases and do not modify
catalog/session state. Streaming and prepared statements need separate lifetime
and invalidation contracts and are not exposed yet.

DataFusion 55.1 requires Rust 1.94. This PR raises the workspace MSRV and its CI
matrix accordingly; Rust 1.98 remains the pinned development/lint toolchain.
Cargo's compatible-version resolver and the lockfile keep dependency selection
reproducible. Refer to the [versioned DataFusion crate documentation](https://docs.rs/datafusion/55.1.0/datafusion/)
for the underlying query engine.

## Verification

`cargo test -p graphfusion --test query_tests --locked` checks actual execution,
Arrow values/types, plan presence, null/unknown truth tables, parameter isolation,
delimited identifiers, empty-result schemas, pagination, type errors, overflow,
division by zero, closed sessions and unsupported programs without side effects.
This foundation is not yet an executable property-graph database; provider scans
and graph-pattern lowering are the next dependency in the delivery plan.
