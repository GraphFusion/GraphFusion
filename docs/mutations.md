# GQL graph mutations

`Session::run(...).await` and the CLI execute `INSERT`, `SET`, `REMOVE`, `DELETE`,
and `DETACH DELETE` against open graphs in memory or Parquet. A write can follow
MATCH/LET/FILTER and return bindings, or terminate with FINISH (implicit for a
standalone modifying statement). `Session::query` stays read-only and rejects
write clauses. The synchronous `Session::execute` remains for catalog/session
commands.

## Try it

Use a new database directory for the example:

```sh
cargo run -p graphfusion --locked -- run --database /tmp/gql-social --create \
  --file examples/social.gql --explain
cargo run -p graphfusion --locked -- run --database /tmp/gql-social \
  --query "USE GRAPH social MATCH (n:Person) RETURN n.name AS name, n.age AS age ORDER BY name"
```

The GQL file creates Alice → Bob → Cara, updates Alice's age to 31 and adds the
Admin label, then returns Cara from a two-hop MATCH. The second process reopens
Parquet storage and returns all three people. No external import is required.

```gql
USE GRAPH social
MATCH (n:Person {name: 'Alice'})
REMOVE n.age
SET n = {name: 'Alicia', active: TRUE}
RETURN n.name AS name, PROPERTY_EXISTS(n, age) AS has_age;

USE GRAPH social
MATCH (n:Person {name: 'Alicia'})
DETACH DELETE n;
```

## Implemented behavior

- INSERT supports node/edge paths, multiple labels, properties, shared bound
  nodes, incoming/outgoing directed edges, undirected edges, loops and parallel
  edges. Every input row creates its new elements, preserving input multiplicity.
  Already bound nodes must appear as bare references. Edge variables are new.
- SET assigns properties, replaces the entire property set with a map, and adds
  a label. REMOVE removes properties or labels. The implementation evaluates SET
  items in order; subsequent items and clauses see preceding changes. Existing
  aliases of the same element are refreshed before further expressions execute.
  A null property has PROPERTY_EXISTS=false; an untyped NULL assignment removes
  the property from the affected layout.
- A property patch is deduplicated by identity and payload. Identical updates
  from repeated input rows affect the element once while retaining result rows.
  Different values for the same target from different input rows are explicitly
  rejected, rather than choosing an arbitrary row. This is a current execution
  restriction, not a claim about the full standard's update-conflict semantics.
- DELETE/NODETACH DELETE requires that no remaining edge references a removed
  node. A single DELETE may name both nodes and their incident edges. DETACH
  DELETE also removes incident edges. Surviving element references and scalar
  bindings remain usable; bindings containing a deleted identity are removed,
  including aliases. Referencing one afterwards rejects the statement rather
  than returning stale values. Mixed live/deleted values under one variable
  currently invalidate that entire variable.
- Empty input produces no inserted elements or updates. Empty INSERT results
  retain the inferred property types. Open graph layouts still require one
  consistent scalar type per property name and element kind.
- `QueryResult::affected_elements` counts inserted/deleted elements and distinct
  targets per update item. Multiple items can count the same element multiple
  times; it is an operation count, not a count of unique elements in the program.
  FINISH prints this count and the commit sequence in the CLI.

## DataFusion execution and atomic publication

The binder and planner produce DataFusion expressions, projections, distinct
plans, joins, and anti-joins. Write barriers materialize the binding table once
before applying a clause. SET/REMOVE split each layout into untouched and changed
rows with joins on element IDs, projecting the new property layout. DELETE uses
anti-joins, including incident-edge predicates for DETACH. INSERT evaluates its
properties with DataFusion and attaches coordinator-allocated UInt64 IDs to Arrow
batches. This is not an expression interpreter or a separate traversal executor.

All changed graphs remain private until the complete statement, including its
RETURN or FINISH pipeline, executes successfully. Graph-wide identity, endpoint,
and property-layout validation runs before publication. The coordinator then
stages Parquet files and commits their manifests and identity counters together.
An error in a later clause or result publishes none of that statement's writes.
Top-level semicolon-separated statements still auto-commit independently.

Graph read versions are checked when a statement writes. This includes a graph
whose values were read to calculate a change to another graph. Concurrent
replacements of the same graph conflict; disjoint graph writes can merge.
There is no automatic retry that might duplicate an INSERT after an uncertain
commit outcome. Existing lifecycle leases protect scans and file reclamation.

Each storage generation has a monotonically increasing element-ID watermark.
Imports raise it above their largest node or edge ID, and deletion cannot lower
it. The watermark is part of the same WAL commit as graph data, survives
checkpoint/restart, and reports exhaustion instead of wrapping at UInt64::MAX.
Aborted, never-published element IDs may be reused. IDs are scoped by graph and
element kind when exposed through ELEMENT_ID. This adds database format v3;
older formats are rejected and migration is not yet implemented.

## Limits and validation

This first writer materializes the complete affected graph and rewrites its
tables. It does not yet provide incremental file deltas, streaming writes,
compaction, indexes, typed-graph constraint enforcement, explicit transactions,
or query NEXT continuations. Mutating a binding from a different working graph
is rejected. Read-only query limitations such as OPTIONAL/aggregation/path search
remain; full GQL standard conformance is unproven.

Tests cover memory and Parquet CRUD, aliases, labels, all-property replacement,
nulls, directions, duplicate inputs, empty input, rollback on later errors,
endpoint restrictions, ID exhaustion/reuse, optimistic read/write conflicts,
and independent CLI processes. Crash tests interrupt a GQL write during file
staging and WAL publication, then verify both graph contents and the next ID.

The [Neo4j DELETE documentation](https://neo4j.com/docs/cypher-manual/current/clauses/delete/)
is a primary implementation reference for explicit versus detach deletion and
GQL's NODETACH spelling. It is not a substitute for the normative ISO audit;
the execution restrictions above must remain visible until that audit and the
remaining language work are complete.
