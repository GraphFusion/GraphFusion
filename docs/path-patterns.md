# Compositional graph path patterns

The planner consumes the path AST's factors recursively. The parser now accepts
concatenation and alternatives following a leading parenthesized primary; its
old early return incorrectly stopped at that primary. Legacy AST start/chains
fields remain available, but the factor tree is authoritative for execution.

```gql
MATCH p = (a {name: 'A'})
          ((x)-[edges:Link]->(y) WHERE x.name <> 'B'){0,3}(b)
RETURN b.name AS destination, PATH_LENGTH(p) AS hops

MATCH p = ALL SHORTEST
          ((a {name: 'A'})-[:Link]->(via)-[:Link]->(b)
           WHERE via.name = 'C')
RETURN b.name AS destination, PATH_LENGTH(p) AS hops

MATCH p = (a {name: 'A'})(-[edge:Link]->(maybe))?(destination)
RETURN maybe.name AS optional_stop, destination.name AS destination

MATCH p = (a)-[:Link]->(b) | (a)-[:Backup]->(b)
RETURN p
```

## Binding and multiplicity rules

- Adjacent node patterns constrain one node. Consecutive edges have an implicit
  intermediate node. Parenthesized subpaths join at their endpoints and can have
  their own mode, subpath variable and predicate. Their WHERE predicate runs
  before an enclosing selector. Search prefixes and extra declarations in the
  parenthesized expression body are rejected; the subpath declaration/mode have
  their own grammar positions.
- Quantified groups require positive minimum edge length. Repetitions join the
  previous endpoint to the next start. Each new element declaration becomes a
  group list, in traversal order. Inside one repetition its binding has its local
  type; outside it is a list. Zero repetitions expose empty lists. Declarations
  must be fresh relative to the enclosing binding table. Nested group variables
  concatenate their lists; path declarations produce lists of subpath values.
- A mode at the head of a complete path applies across all group boundaries. A
  mode inside a quantified primary constrains each iteration. An inner TRAIL
  alone does not make an unbounded outer WALK finite.
- `?` includes both the zero- and one-occurrence branches. Its newly declared
  singleton variables are null on the absent branch. It is distinct from
  `{0,1}`, whose variables are group lists. It also differs from OPTIONAL MATCH:
  a zero-occurrence result remains even when a one-occurrence match exists.
- `|` deduplicates equal complete paths and exposed bindings for each incoming
  record. `|+|` retains the bag, including overlapping branches. Anonymous scan
  columns do not become accidental deduplication keys. Incoming ordinals keep
  equal input records separate. Alternatives align bindings by GQL name, fill
  absent bindings with typed nulls and reject incompatible element kinds or
  group/singleton exposure.
- A variable missing from an alternative, or introduced by `?`, is conditional.
  Implicit reuse of a conditional singleton within the same MATCH is rejected.
  It can be used in expressions, including null-safe element predicates and
  mutation targets. Null mutation targets remain no-ops. A later MATCH starts a
  new pattern scope and applies ordinary bound-element identity constraints.

These features compose: alternatives and nested quantified edges/groups may
appear inside a repeated group, and repeated groups can precede or follow fixed
segments. Repeated endpoint variables remain identity constraints. Parenthesized
predicates can refer to outer bindings; declaration capture is rejected rather
than silently changing a scalar into a group.

## Native DataFusion plans

A quantified group first compiles one iteration to a DataFusion relation. The
relation is evaluated for each incoming row and contains its ordinal, path node
and edge IDs, and newly exposed values. An outer RecursiveQuery/CteWorkTable
joins these rows by incoming ordinal and current endpoint, concatenates the
paths and group lists, and applies whole-path mode constraints. Nested recursion
is materialized before constructing the outer recursive term. This respects
DataFusion 55's single-work-table-reference restriction. No Rust code traverses
graph adjacency lists.

Alternatives project the incoming columns, exposed bindings and path identities
to a common schema. They use native UNION ALL and, for `|`, DISTINCT. An execution
boundary preserves the aligned schema before following joins; without it,
DataFusion 55's leaf-projection optimization can reference aliases that it has
removed. Query plan traces include the work performed at each boundary.

The iteration relation can be large: this is a correctness-first implementation,
not an adjacency index or a specialized shortest-path executor. It inherits the
statement snapshot and commit guard. Errors while building or executing it abort
the statement and fail an active explicit transaction as usual.

## Unbounded homogeneous WALK selectors

With a single quantified edge and no parenthesized/alternative factors, selective
prefixes now support repeatable unbounded WALK. The proof requires stable,
local edge predicates and endpoint predicates that do not inspect the edge group
list. Unknown expression kinds fail this proof conservatively. Graph-pattern
WHERE is still a post-selection filter.

Let N be graph node count and m the quantifier's lower bound. A shortest walk has
length at most m+N-1: retain the first m edges, then remove cycles from the suffix.
This covers ALL SHORTEST and single-path selection, including ties and cycles
required by a positive lower bound.

For k counted paths or k distinct length groups, use the product graph whose
state is `(node, min(depth,m))`. It has V=N*(m+1) states. A walk of at least k*V
edges contains k disjoint cycles (one within each V-edge block). Successively
removing those cycles gives k strictly shorter accepted walks of distinct
lengths. Thus every path in the first k length groups has length at most k*V-1;
that bound also contains sufficient candidates for counted path selection.
Zero-node graphs use a harmless positive planning bound and return no matches.

These are completeness bounds. If the required bound exceeds QueryLimits, the
query reports a resource error; it never substitutes the configured hop limit
for the proved bound. The planner currently enumerates candidates up to that
bound before window ranking. A history-dependent endpoint predicate, such as a
comparison of a group list to a previous group list, cannot use cycle removal;
it requires an explicit finite bound at present.

## Validation and remaining scope

Tests compare group results with flat edge expansions over multiple path modes,
quantifier ranges and duplicate inputs. They check ordered node/edge groups,
nested group flattening, alternative bag semantics, conditional nulls, mutation
atomicity, parenthesized prefilters and unbounded selector results against larger
explicit bounds. Parser regressions cover a leading parenthesized primary followed
by concatenation and alternation. CLI validation exercises Parquet after restart.

KEEP and graph YIELD remain explicit errors. General unbounded selective regular
patterns and predicates over complete path histories need further planning and
termination/resource work. [Element references](element-references.md) now provide
property/label dereference after group-list expansion; horizontal aggregation
remains part of the expression/scope task. Default MATCH
mode and the remaining ISO conformance details still require normative audit.
Multi-path DIFFERENT EDGES clauses with selective prefixes are rejected before
execution: cross-path uniqueness and per-path selection must not silently depend
on their order of application. Nonselective multi-path clauses and selective
REPEATABLE ELEMENTS clauses remain available.

Semantic cross-checks include the language designers' paper
[Graph Pattern Matching in GQL and SQL/PGQ](https://www.theoinf.uni-bayreuth.de/pool/documents/Paper2021-25/Paper2021/gql-sqlpgq.pdf),
[MillenniumDB's minimum path-length rules](https://mdb.imfd.cl/doc/docs/gql/quantified-path-patterns.html),
and [Ultipa's questioned-path examples](https://www.ultipa.com/docs/gql/questioned-paths).
These public descriptions support the implementation tests; they do not replace
the normative ISO standard or establish full conformance.
