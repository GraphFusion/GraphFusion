# Relational GQL queries

The query planner now executes OPTIONAL MATCH, FOR, grouping/aggregates, SELECT
sources and composite queries through DataFusion. Run the persistent example in a
new directory:

```sh
cargo run -p graphfusion --locked -- run --database /tmp/gql-analytics --create \
  --file examples/analytics.gql --explain
```

Alice has one outgoing friend; Bob and Cara have zero. The second result reports
three people with median age 31. The final query combines two result bags and
reports two occurrences each of scores 10 and 20.

## Optional matches and row identity

An optional pattern applies all its predicates before adding nulls for an input
row that has no complete match. For example:

```gql
MATCH (p:Person)
OPTIONAL MATCH (p)-[:Knows]->(friend) WHERE friend.age > 30
RETURN p.name AS name, COUNT(friend) AS friends
GROUP BY name ORDER BY name
```

The planner materializes the incoming working table once, attaches a unique row
ordinal, runs the whole pattern as DataFusion joins/filters, and left joins the
result to that input. The ordinal keeps identical input rows distinct; matching
by the values of the input columns would multiply duplicates incorrectly.
`OPTIONAL { MATCH ... MATCH ... }` pads the entire block when it fails. An inner
OPTIONAL MATCH retains its own outer-join semantics.

Newly introduced elements can be null. Their properties and ELEMENT_ID return
null; label, property-existence, direction and identity predicates follow nullable
boolean semantics. COUNT(element) ignores missing elements. A later mandatory
MATCH on a missing node cannot match that null binding. SET/REMOVE/DELETE skip
null targets and retain other bindings; existing mutation alias restrictions
still apply. INSERT cannot use a null endpoint.

This implementation buffers optional inputs in memory. It does not yet stream
those barriers or share them through an executor cache.

## FOR and aggregation

`FOR x IN list` uses DataFusion UNNEST. Empty/null lists produce zero rows;
null list entries produce rows containing null. `WITH ORDINALITY i` starts at 1,
and `WITH OFFSET i` starts at 0, for each incoming row. Scalars, typed/untyped
session lists, and nested homogeneous lists are supported. Numeric list elements
can share a common numeric type, including nested lists containing empty or
all-null sublists; incompatible families are rejected instead of
silently converting text or booleans to numbers. Heterogeneous GQL union-valued
lists need additional value-type representation and are not implemented.

Aggregates include COUNT(*), COUNT(value/element), SUM, AVG, MIN, MAX,
COLLECT_LIST, STDDEV_POP, STDDEV_SAMP, PERCENTILE_CONT and PERCENTILE_DISC, with
ALL/DISTINCT. Null inputs are omitted. An empty global aggregate returns one row;
COUNT is zero, COLLECT_LIST is an empty list, and numeric aggregates are null.
COLLECT_LIST has no guaranteed ordering. Discrete percentiles retain the input
numeric type and exact selected integer; continuous percentiles return Float64.
Percentile fractions must be row-independent numeric expressions in [0, 1]; a
null fraction produces null. Integer SUM widens accumulation before a checked
Int64 conversion; non-finite floating aggregate results raise an error.

Without GROUP BY, nonaggregate projected expressions with input references form
the grouping keys. Explicit GROUP BY accepts binding variables, including output
aliases, and `GROUP BY ()` selects a global group. Grouping a graph variable uses
its identity and available element columns. SELECT HAVING filters groups. ORDER
BY can reference output aliases, available grouping expressions and aggregate
expressions; DISTINCT requires its ordering expressions to depend on projected
results. OFFSET/LIMIT apply after result construction. Nested aggregates and
ungrouped input references are rejected.

## Composition and SELECT sources

UNION, INTERSECT and EXCEPT default to DISTINCT; ALL preserves bag multiplicity.
For an entire result row occurring L times on the left and R times on the right:

| Operator | Output occurrences |
| --- | --- |
| UNION ALL | L + R |
| INTERSECT ALL | min(L, R) |
| EXCEPT ALL | max(L - R, 0) |

Null fields compare equal for this purpose. DataFusion's logical semijoin/antijoin
builders alone do not implement the ALL counts, so the planner numbers each
occurrence within an equal-row partition, then joins on the values and occurrence
number using null equality. UNION uses native DataFusion union plans.

Branches must have the same column names, count and order, with matching types,
a null type widened to the other type, or Int64/Float64 numeric widening. General
GQL union types and differently named result alignment are not implemented.
Each branch starts with an independent variable scope and ambient graph context;
a branch-local USE GRAPH does not alter another branch or the session.

OTHERWISE returns the first nonempty branch; a row of nulls is still nonempty.
All branches undergo binding and schema validation before evaluation. Validation
retains barrier schemas without executing their inputs. Execution materializes
branches until one returns rows, so cold branches do not evaluate runtime
expressions such as division by zero. This buffers the chosen result. Composite
writes are rejected before executing any branch.

The existing parser requires a single conjunction kind at each composite level.
Nested `{ ... }` query primaries can combine levels, and now forward their result
unless an explicit FINISH is present. SELECT supports graph MATCH sources and
`FROM { query }` / `FROM graph { query }`, with scalar result bindings, WHERE,
GROUP BY and HAVING. Correlated nested queries, procedures, query NEXT, whole
element/path results, typed LET, quantified/path-search semantics and several
value expressions remain unsupported. These are explicit errors. The project
does not yet claim complete ISO GQL conformance.

## Validation and references

`relational_tests` checks duplicates, disconnected patterns, nested optional
blocks, null element predicates, empty inputs/groups, aggregate errors, checked
sums, percentile endpoints, list ordinals/types, distinct ordering, set bag counts,
independent scopes, OTHERWISE laziness, SELECT sources and write rollback.
Parser tests distinguish inherited nested results from FINISH. CLI tests create
a graph using GQL, reopen Parquet in another process, execute optional aggregation,
and verify that a failed aggregate result leaves graph data unchanged.

Primary implementation references used to check these semantics (not a normative
ISO conformance proof):

- [Neo4j OPTIONAL MATCH](https://neo4j.com/docs/cypher-manual/current/clauses/optional-match/).
- [Ultipa aggregate functions](https://www.ultipa.com/docs/gql/aggregate-functions).
- [Ultipa GROUP BY binding variables](https://www.ultipa.com/docs/gql/return).
- [Ultipa composite queries](https://www.ultipa.com/docs/gql/composite-query).
- [DataFusion 55.1 logical-plan builder source](https://docs.rs/datafusion-expr/55.1.0/src/datafusion_expr/logical_plan/builder.rs.html).
