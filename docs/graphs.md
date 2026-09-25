# Arrow graph storage and MATCH

GraphFusion can import an Arrow property graph and run fixed-length GQL
MATCH queries through DataFusion. The catalog selects the graph; DataFusion scans
Arrow/MemTable or Parquet providers and executes the joins. There is no graph traversal loop
outside DataFusion.

Run the complete example:

```sh
cargo run -p graphfusion --example social --locked
```

It creates a catalog graph, imports Alice → Bob → Cara, and executes:

```gql
MATCH (a:Person {name: 'Alice'})-[:Knows]->(b)-[:Knows]->(c)
RETURN c.name AS friend
```

The result is `Cara`; plan diagnostics include real DataFusion scans and joins.

## Import API and table layout

Build `NodeTable` and `EdgeTable` values from Arrow schemas and record batches,
then validate them together with `GraphData::try_new(nodes, edges)`.
`Session::replace_graph_data(data)` atomically replaces the current open graph.
Every table describes one exact set of labels and one property layout; a
multi-label node appears in one table, preserving its single identity.

| Column or attribute | Meaning |
| --- | --- |
| `graph::ID` (`__gf_id`) | Non-null UInt64 element ID, unique per graph and element kind |
| `graph::SOURCE` | Non-null UInt64 source node ID for an edge |
| `graph::DESTINATION` | Non-null UInt64 destination node ID for an edge |
| Node/edge table labels | Exact label set shared by the table's rows |
| Edge table `directed` | Directed edge or undirected connection |
| Other columns | Boolean, Int64, Float64, Utf8 or Binary properties |

An undirected edge stores two endpoints without giving their order semantic
significance. Parallel edges and self-loops are valid. Nodes and edges may use
the same numeric ID; their element identities remain different.

Import rejects duplicate IDs (including across tables/batches), dangling
endpoints, duplicate/empty labels or columns, null required columns, mismatched
batch schemas, and unsupported/mixed property types. The `__gf_` column prefix is
reserved by the import format. Decoded GQL binding names are mapped independently
and can contain dots, quotes, or the same text as an internal column.

Properties missing from one layout are projected as typed nulls when combining
tables. The storage keeps layout membership distinct from a present null value.
`PROPERTY_EXISTS(n, p)` is false when a non-null element lacks the property or
its value is null. For a null element introduced by OPTIONAL MATCH, the predicate
returns null.

## Query semantics currently executed

- Current graph, USE GRAPH names/paths, graph parameters, current/home graph
  references, and AT SCHEMA selection reuse the catalog/session resolver. Query
  context changes do not modify session state.
- Node/edge labels and AND/OR/NOT/wildcard label expressions, property maps,
  inline predicates, MATCH WHERE and FILTER. Property maps and inline predicates
  can reference any element bound in the same MATCH, including later patterns;
  variables introduced by a subsequent MATCH remain out of scope.
- Fixed-length node/edge chains, multiple MATCH clauses, disconnected patterns,
  and repeated bindings. A repeated node variable constrains identity rather than
  scanning a new independent node.
- Outgoing, incoming, undirected, either-directed and mixed direction patterns.
  Reverse orientation unions exclude a second copy of a self-loop. Parallel
  edges retain distinct IDs and therefore retain match multiplicity.
- DIFFERENT EDGES (the default) constrains every edge occurrence in one MATCH,
  including across disconnected paths; REPEATABLE ELEMENTS lifts that constraint.
  Each new MATCH starts its own edge-occurrence constraint set. The default MATCH
  choice still requires normative audit. [Paths](paths.md) adds named paths,
  quantified edges, all four modes and endpoint-partitioned selectors.
- Property projection and scalar expressions/LET, sorting and paging over result
  aliases, ELEMENT_ID, SAME, ALL_DIFFERENT, IS LABELED, IS DIRECTED and
  PROPERTY_EXISTS. Element IDs are opaque strings containing graph, kind and row
  identity; applications should not depend on their textual encoding.

[Relational queries](relational.md) implement optional matches, aggregate results
and SELECT FROM graph forms. [Element references](element-references.md) add
whole-element values and scalar reference lookups. Graph YIELD/KEEP remain
unimplemented. [Complex paths](path-patterns.md) cover
parenthesized expressions, repetition, alternatives and conditional variables. Reusing one variable as a different kind or across different
graphs is rejected. These limits are explicit errors, not partial query results.

## Snapshot and persistence boundaries

Graph providers belong to immutable storage generations in the statement
coordinator. Replacements record a graph version for optimistic conflict checks.
Disjoint graph replacements can commit concurrently; two replacements of the same
version cannot both commit. Catalog identity checks also reject a writer whose
graph was dropped or replaced.

Readers keep the old Arrow buffers or Parquet files and catalog snapshot through physical planning
and materialization. DROP retires the storage generation; checkpoint cannot
reclaim it until all statement/transaction leases are released. The import API has no direct
path that can independently publish a provider outside the coordinator.

Both `Database::new()` and `Database::open(...)` support open graph imports.
Persistent databases stage immutable Parquet files and publish their manifests
through the same coordinator; see [storage and CLI](storage-cli.md). Typed graph
imports and scans still wait for schema binding and validation. Memory buffers
cannot be serialized into the metadata log/checkpoint. [GQL writes](mutations.md)
now use DataFusion plans and the same coordinator. Standard conformance and the
remaining language features are still required before the objective is complete.

## Evidence and semantic references

`graph_tests` exercises actual DataFusion scans/joins, multiple table layouts,
multi-label identity, duplicates/cycles, all principal edge directions, graph
selection, stale references, typed nulls and binding isolation. Library tests
exercise graph replacement conflicts, old snapshots across replacement and DROP,
memory-serialization rejection, durable import/recovery, and Parquet file reclamation.

Implementation behavior was cross-checked against these primary implementation
documents. They are not a substitute for the final normative ISO coverage audit:

- [DataFusion logical plans and providers](https://datafusion.apache.org/library-user-guide/building-logical-plans.html).
- [Microsoft GQL match/path modes](https://github.com/MicrosoftDocs/dataexplorer-docs/blob/main/data-explorer/kusto/query/graph-query-language-guide.md).
- [Neo4j GQL property-exists predicate](https://neo4j.com/docs/cypher-manual/25/expressions/predicates/comparison-operators/).
