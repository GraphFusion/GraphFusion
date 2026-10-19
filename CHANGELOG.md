# Changelog

## 0.3.0 — Unreleased

- Compare a mixed property with a working-table value on each fragment's stored
  type. `WHERE n.id = k` no longer fails when `k` is bound, and a string value
  does not match a stringified integer.
- Keep boolean literals on a per-fragment comparison. `WHERE n.id = 1 OR FALSE`
  no longer fails after a multi-type scan, and `OR TRUE` keeps a row whose
  stored value cannot be compared.
- Compare a `FILTER` predicate on each matched fragment's stored property type.
  `FILTER n.id = 1` no longer fails after a multi-type scan has unified those
  values, and a string comparison does not match a stringified integer.
- Evaluate `XOR` of property comparisons on each fragment's stored type. A
  multi-type filter such as `n.id = 1 XOR n.id <> 1` no longer fails after the
  scan has unified those values into one column.
- Match every orientation in `<~[]~>` and `<~/label/~>`. The closing `>` adds
  rightward directed edges to the left-or-undirected pattern.
- Keep working-table order when a query resolves an element reference. A
  preceding `ORDER BY` stays in effect through `LET`, and `FOR` walks a list of
  element references in list order.
- Reject a null INSERT endpoint as an invalid query. A failed OPTIONAL MATCH no
  longer surfaces an internal Arrow error, and the statement inserts nothing.
- Cap a selective search of one unbounded edge at the complete node-derived hop
  bound. `ANY SHORTEST` and the other selectors no longer expand every trail out
  to the edge count when a shorter bound is complete. `ALL` still follows the
  match mode's own limit.
- Read a positive-length quantified edge from the edge itself when the path
  start is anonymous and unconstrained. `()-[:E]->+()` no longer cross-joins
  every node before the first hop. A named, labeled, or constrained start still
  reads the node tables, and a quantifier that allows zero repetitions still
  anchors on every node.
- Reject a decimal literal that overflows to infinity. `1e999` is a bad number,
  rather than the Float64 value `inf`, in queries and session initializers.
- Start an anonymous path at its first edge. `()-[]->()` no longer cross-joins
  every node to recover a source id the edge already stores. A named, labeled,
  or constrained start node still reads the node tables.
- Print null cells and null list elements as `NULL`. An empty list stays `[]`,
  and an empty string stays blank, so those values are no longer identical in
  CLI output.
- Reuse an edge endpoint for an anonymous unconstrained node. A pattern such as
  `()-[]->()` no longer joins every node just to copy an id the edge already
  carries. Labels, property maps, and named variables still read that node.
- Keep an anchored optional path segment on its known endpoint. The empty
  alternative no longer scans every node to copy that same id.
- Widen integer, float, and empty lists in `UNION`, `INTERSECT`, `EXCEPT`, and
  `OTHERWISE`, including nested lists. String and boolean lists stay incompatible
  with number lists.
- Compare integer lists with float lists, and compare an empty list with a typed
  list. The operands widen to one element type first, including nested lists.
  `COALESCE` and `NULLIF` use that same list type.
- Compare lists with three-valued logic. A null element stays unknown unless an
  earlier element, or the lengths after an equal prefix, already decides the
  result. `NULLIF` uses that same equality, so an unknown list comparison keeps
  the original list.
- Align a shared element across path alternatives when the branches store
  different properties. Integers widen to float, other mixed scalars are read
  as strings, and a property present on only one branch is null on the other.
- Reject a grouped result that still refers to an ungrouped input, including in
  `ORDER BY` and `HAVING`, as an invalid query.
- Keep a variable readable after `DELETE` when only some of its rows were
  removed. Property access still fails for a deleted identity, including
  through an alias, and that failure rolls the statement back.
- Refresh a bound element without adopting a one-label-apart neighbor's
  property types. A label edit can still find the moved row, and a later
  comparison keeps the type stored on the bound label set.
- Scan a label pattern's compatible fragments only, so a multi-type graph keeps
  each type's property types and does not read unrelated tables. Integers widen
  to float when a pattern matches both; other mixed scalar properties are read
  as strings. A label set still stores one type per property, and a write cannot
  change it or widen a bound element with another type's schema.
- Execute linear query `NEXT` / `YIELD` chains, and read session or nested
  binding tables with `TABLE` / `BINDING TABLE`.
- Expand a repeated group of plain nodes and edges from the current endpoint
  inside the recursive term, instead of first materializing every one-step
  match in the graph. A quantifier, alternative, or optional subpath inside
  the group still precomputes that one-step relation.
- Build a repeated group's one-step relation once when it does not read an
  outer variable, instead of crossing that relation with every input row. A
  group that reads an outer variable still keeps a separate step per row.
- Apply a MATCH predicate as soon as every variable it reads is bound, so a
  start-node filter runs before a later variable-length expansion. A predicate
  that names an element declared later in the same MATCH still waits for that
  element.
- Reject a binding-table column that mixes a number with a boolean, string, or
  bytes value. Integers and floats in one column still widen to float.
- Resolve a `LET` copy of a label-constrained element from that label set, so
  its property types are not widened by another label that stores the same name.
- A label that matches no fragment keeps property names as untyped columns.
  It does not borrow another type's Arrow type, so a filter on the empty
  match is not rejected because unrelated labels disagree.
- An integer inserted or assigned into an existing float property widens to
  float. The stored column type does not change, and a narrowing assignment
  is still rejected.
- An all-null binding-table column stays untyped, so comparing it with a
  graph property does not fail as a string/integer mismatch.
- Compare a filter on each fragment's stored property type before a multi-type
  scan unifies those columns. An integer comparison stays numeric, and a
  fragment that cannot compare is unknown rather than a match.
- Keep a MATCH `WHERE` on a conditional or repeated-group name. A variable
  missing from an alternative or a questioned segment stays null and does not
  match, and a property comparison on a repeated group is rejected because that
  name is a list.
- Enforce `CHAR(n)` as a maximum of `n` characters. A longer value is rejected,
  matching `VARCHAR(n)` and `STRING(n)`.
- `COALESCE` a null with a byte string. The null takes the byte-string type
  instead of failing while the call is planned.
- Construct a list of byte strings that includes null. The null stays a null
  element instead of failing while the list is built.
- Treat a simplified `<~label>` direction override as any orientation, so the
  closing `>` also matches rightward directed edges.
- Interpolate an integer PERCENTILE_CONT from the exact endpoints and round
  once to Float64, so large integers keep low bits through the blend.
- Evaluate `COALESCE` and `NULLIF` of a property on each fragment's stored
  type. A comparison wrapped in either function is unknown on a fragment that
  cannot compare, `NULLIF(property, value) IS NULL` does not stringify the
  property, and `COALESCE(property, fallback)` compared with a closed value
  uses that fragment's type.
- Classify that per-fragment result with `IS [NOT] TRUE`, `IS [NOT] FALSE` and
  `IS [NOT] UNKNOWN`. An integer is unknown against the same digits stored as
  text, so `IS TRUE` does not keep it and `IS UNKNOWN` does.
- Compare a parenthesized path predicate on each fragment's stored type. A
  mixed property no longer makes `WHERE n.v > 9` reject the whole pattern, and
  an integer is not equal to those same digits stored as text.
- Keep that per-fragment comparison when `AND`, `OR`, `XOR`, or `NOT` combines
  it with another boolean, and when `FILTER` or `LET` evaluates it. `id = '1' OR
  FALSE` does not match an integer `1` just because another label stores `id`
  as text.
- Keep every distinct spelling in a bare `INSERT` label set. `(A & a)` stores
  both labels, the same as `(:A & a)`. An exact repeat still collapses.
- Repeat a quantifier after a simplified path across the whole slash pattern.
  `-/E F/->{2}` repeats the E-then-F sequence, `-/L|R/->{1,2}` can mix labels
  between repetitions, and a quantifier on one label no longer discards the
  outer one.

## 0.2.0 — 2026-10-05

- Add typed AST visitors and deterministic tree dumps covering every stored AST
  field, with traversal hooks, early termination and checked-in output snapshots.
- Persist committed Arrow MemTables in the WAL and seal them to immutable Parquet
  files at configurable row or memory thresholds. Recovery restores unsealed
  batches; checkpoint seals remaining main-branch data.
- Index graph mutations with resident ID and adjacency lookups, Parquet index
  sidecars and snapshot-specific deletion vectors. Updates reuse unchanged
  buffers and files; selected Parquet rows are filtered before decoding.
- Reduce catalog and transaction overhead with copy-on-write catalog entries,
  encoded name keys, child/dependency indexes and shared persistent main state.
- Add a browser Playground powered by WebAssembly, with an in-memory GQL engine
  and interactive graph visualization. The optional `visualization` feature
  captures graph identities and properties from each query's source snapshot.

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
- Breaking: the persistent disk format is now v4. Older database directories are
  rejected, and no built-in migration is provided.
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
