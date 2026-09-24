# Path planning and execution

Roadmap tasks 10a/10b provide recursive edges and compositional path patterns.
[Complex path patterns](path-patterns.md) include parenthesized expressions,
quantified groups, alternation and questioned paths. KEEP, graph YIELD and the
remaining history-dependent unbounded selective WALK cases still need work;
this is not a complete ISO GQL conformance claim.

## Implemented queries

```gql
MATCH p = ALL SHORTEST (a {name: 'A'})-[:Link]->{0,4}(b)
RETURN b.name AS destination, PATH_LENGTH(p) AS hops
ORDER BY destination, hops

MATCH REPEATABLE ELEMENTS p = SIMPLE (a)-[:Link]->+(b)
RETURN p

MATCH p = (a)-[edges:Link]->{1,3}(b)
FOR edge IN edges
RETURN ELEMENT_ID(edge) AS id

MATCH p = (a)-[:Link]->(b)
FOR element IN ELEMENTS(p) WITH ORDINALITY position
RETURN position, ELEMENT_ID(element) AS id
```

Fixed chains and edge quantifiers `{m}`, `{m,n}`, `{m,}`, `{,n}`, `*`, `+`
and the AST optional quantifier execute through DataFusion plans. The parser's
surface syntax restrictions still apply. An upper bound must be positive; a
zero lower bound includes the start node as a path with no edges. Endpoint
labels/properties apply to that same node on a zero-hop match. Edge predicates
apply to each traversed edge; start/end node predicates do not accidentally
constrain intermediate nodes. Repeated endpoint variables enforce identity.

An edge variable inside a quantifier is a group list in traversal order. Within
its edge predicate it denotes the current edge. Outside the quantified segment
it is an Arrow list of element references; zero repetitions produce `[]`. A
fixed prefix or suffix contributes to the path, but not to that group's list.
Group variables must be fresh. Temporary element declarations remain unsupported.

## Modes and selection

The path prefix defaults to WALK. WALK permits repeated nodes and edges; TRAIL
forbids repeated edges; ACYCLIC forbids repeated nodes; SIMPLE permits a repeated
start node only as the final node of a cycle. A mode constrains the whole path,
including all fixed and quantified segments.

The current MATCH default remains DIFFERENT EDGES, which also prevents reuse
across disconnected paths in that MATCH. REPEATABLE ELEMENTS removes that
MATCH-wide constraint; it does not remove an explicit path-mode constraint.
Each new MATCH starts a new constraint scope. The default MATCH choice is a
documented compatibility decision pending the normative standard audit, not a
claim that a vendor's default proves ISO conformance.

ALL retains the path bag. ANY / ANY k, ANY SHORTEST / SHORTEST, SHORTEST k,
ALL SHORTEST, and SHORTEST k GROUP(S) select within each ordered start/end pair
and each incoming binding-table row. Duplicate incoming rows have independent
selection partitions. Counts are positive signed integers, either literals or
integer parameters. SHORTEST k retains up to k paths ordered by edge count;
GROUP(S) retains all ties in the first k distinct lengths. ANY is free to choose
the shortest candidates. Ties that must be broken have no promised ordering.
Final MATCH WHERE filters the selected results; it does not restart selection
to find a longer path. Parenthesized predicates filter their subpath before an enclosing selector.

## Arrow values

A path result is a struct containing `__gql_path_graph: UInt64`,
`__gql_path_nodes: List<UInt64>` and `__gql_path_edges: List<UInt64>`. Node IDs
are in traversal order; there is always one more node than edge. The path's
graph identity separates equal numeric IDs in different graphs. PATH_LENGTH
returns an Int64 edge count. A missing OPTIONAL MATCH path is a null struct,
and its length is null.

ELEMENTS interleaves node, edge, node references. Group lists contain references
of one kind. Each reference is a struct with `__gql_element_graph: UInt64`,
`__gql_element_kind: Utf8` (`n`/`e`) and `__gql_element_id: UInt64`.
FOR can unnest these lists and ELEMENT_ID produces the same opaque identity as
for a directly matched element. Reference values can be returned, counted and
collected. These field names are the current driver encoding, not additional
GQL property names. Property/label dereference and rebinding of these scalar
references, and whole directly matched element values, remain follow-up work.
PATH_LENGTH and ELEMENTS reject ordinary scalar/list values.

## DataFusion execution and limits

The anchor of a RecursiveQuery is the incoming binding table plus a zero-hop
path. Its recursive term scans a CteWorkTable, joins an oriented edge provider,
applies predicates and mode restrictions, and appends IDs using DataFusion list
functions. UNION ALL preserves distinct routes, parallel edges and duplicate
input rows. The final term applies the quantifier's lower bound. Native window
functions implement endpoint selection: ROW_NUMBER for counted paths and
DENSE_RANK for length groups. There is no Rust traversal executor or SQL text
translation. Small scalar UDFs only encode ordered element-reference values.

Selection materializes an input ordinal once to preserve duplicate input rows.
OPTIONAL MATCH materializes its matches before the outer join: besides preserving
the optional block's semantics, this prevents DataFusion 55 projection pruning
from producing invalid recursive work-table schemas. These barriers appear in
the returned logical/physical plan trace.

Unbounded homogeneous edge WALK now supports selective prefixes when predicates
are independent of the path history. Its finite completeness proofs and limits
are detailed in [complex path patterns](path-patterns.md). Other unbounded modes
use graph edge count for TRAIL/DIFFERENT EDGES, node count minus one for ACYCLIC,
and node count for SIMPLE. Remaining repeatable unbounded patterns require a
finite upper bound. No arbitrary search cutoff is presented as a complete result.

`Session::set_query_limits(QueryLimits { .. })` configures a default maximum of
256 path hops and a 256 MiB DataFusion tracked operator memory budget per
statement. Bounds above the hop limit fail during planning; candidates are never
silently truncated. The planner combines concatenation, alternative and repetition
bounds, then applies any whole-path mode bound. The memory pool applies to DataFusion operators, including
recursive work tables; it is not an RSS limit and does not account for every
imported graph buffer or final materialized result. Memory exhaustion is an
execution error. Errors follow normal statement/explicit-transaction rollback
rules. Limits are session-local and can be changed for subsequent statements.

The implementation currently enumerates all eligible finite candidates before
ranking, so selective queries on dense graphs can still be expensive. Native
frontier pruning, the remaining path grammar and scopes, cancellation and full
resource accounting remain follow-up tasks.

## Evidence and semantic references

Integration tests exercise zero lengths, cycles, mixed directions, parallel edges,
multiple segments, duplicate input rows, group variables, path-value nulls,
selection ties, count parameters and resource-error atomicity. A separate
exhaustive test enumerates tiny finite walks and compares complete identity bags
across 32 combinations of direction, path mode and MATCH mode. It is test-only.

Primary implementation references are DataFusion 55.1.0's
`LogicalPlanBuilder::to_recursive_query`, `CteWorkTable`, and `RecursiveQueryExec`.
Public semantic cross-checks include
[Neo4j's optional GQL feature list](https://neo4j.com/docs/cypher-manual/current/appendix/gql-conformance/supported-optional/),
[Ultipa's path mode and selector documentation](https://www.ultipa.com/docs/gql/path-patterns),
and [its shortest-path examples](https://www.ultipa.com/docs/gql/shortest-paths).
Vendor deviations are not treated as normative standard text. The full
ISO/IEC 39075 conformance audit is still outstanding.
