# Graph element values and references

Matched nodes and edges now work as values in RETURN, LET, lists and aggregate
results. They use the same identity representation as path elements and group
lists. A reference contains its graph ID, node/edge kind and element ID; it does
not copy a mutable property map into the value.

```gql
MATCH p = (a {name:'A'})-[:Link]->{1,3}(b)
FOR item IN ELEMENTS(p) WITH ORDINALITY position
RETURN position, item.name AS name, item IS LABELED Station AS station

MATCH (n:Station)
LET alias = n
RETURN alias.name AS name, SAME(alias,n) AS same

MATCH (n {name:'A'})
FOR alias IN [n,n,NULL]
MATCH (alias)-[:Link]->(destination)
RETURN destination.name AS name
```

## Values, predicates and matching

The Arrow representation is a nullable struct with non-null UInt64 graph and
element fields and a UTF-8 kind field. These are implementation details; clients
should treat the reference and ELEMENT_ID result as opaque, database-local
identities. Reference values are query results, not yet accepted as external
session parameters. Property data remains in the graph's Arrow/Parquet tables.

Whole-element results, identity equality, SAME, ALL_DIFFERENT, ELEMENT_ID, null
tests, property access, PROPERTY_EXISTS, IS LABELED and IS DIRECTED work on scalar
references. Lists, COLLECT_LIST and derived/composite query results can carry
references. Explicit GROUP BY over a reference groups its associated lookup
columns with its identity so properties can be projected. IS DIRECTED requires
an edge; passing a non-null node reference raises an error.

Optional and conditional elements become actual null structs, including within
lists. Missing properties yield null; property existence follows the existing
non-null-property rule. Predicates on null elements preserve unknown rather than
inventing an element. Graph ID and kind participate in equality and lookup, so
equal numeric node/edge IDs or IDs in different graphs never alias.

An existing scalar reference can be used in a subsequent node or edge pattern.
The pattern constrains graph, kind and identity, and applies labels, properties,
direction and path rules normally. A null reference or one belonging to another
graph/kind produces no match. Non-reference scalar bindings remain type errors.
Duplicate incoming records retain their multiplicity.

## Native DataFusion lookups and snapshots

The planner tracks graph/kind domains for matched values, aliases, list
constructors, path elements, quantified groups, alternatives and COLLECT_LIST.
Result projections carry these domains through derived queries; composite
queries union the possible domains of their branches. The planner uses this
information to scan only possible source kinds. An expression without that
static information conservatively considers graph sources used by its query.
Every lookup is a DataFusion left join on graph/kind/ID. Uniqueness is guaranteed
by GraphData validation. Rust code encodes identities and validates liveness; it
does not retrieve properties or traverse graph adjacency lists.

Execution boundaries materialize newly created references and their lookup
relations. They preserve nullable rows and isolate the relations from known
DataFusion 55 common-expression/leaf-projection alias rewrites. Mutation identity
allocation is likewise isolated before reference refresh. These boundaries
increase buffering and plan size; this implementation does not claim to be an
indexed or streaming lookup engine. Statement resource limits still apply, with
the existing distinction between tracked operator memory and process RSS.

Sources come from the statement/explicit-transaction snapshot and staged writes.
Changing the working graph does not change a reference's identity. Writes refresh
reference lookups against the staged graph, while scalar property values captured
by an earlier LET retain their earlier values. Dereferencing a deleted element
raises an execution error, which aborts the statement and fails an active
explicit transaction. An opaque identity can still be carried without property
dereference; it is not a storage lease or a durable client-side object handle.

## Mutations and current limits

References with one statically known graph and element kind can be INSERT
endpoints or SET, REMOVE and DELETE targets. True null targets are no-ops for
SET/REMOVE/DELETE; deleted non-null references fail before null-target elimination.
Duplicate target IDs use the existing mutation conflict/counting rules. A target
from another working graph is rejected. A mixed or insufficiently known reference
must first be constrained by MATCH to a node or edge binding.

```gql
MATCH (a {name:'A'})((x)-[:Link]->(y)){1,2}(b)
FOR destination IN y
SET destination.visits = destination.visits + 1
RETURN destination.name AS name
```

Properties with incompatible physical types across the possible source domains
are rejected rather than coerced to text or silently choosing one type. Compatible
numeric domains can use DataFusion numeric coercion. General variant property
types, arbitrary
property-base expressions, horizontal aggregates, richer type predicates,
correlated subqueries and procedures remain follow-up expression/scope work.

`reference_tests` covers nulls, duplicate inputs, path/group elements, aliases,
cross-graph identity, type domains, grouping, reference rebinding and mutations.
CLI tests exercise the example after checkpoint and independent-process reopen.
Existing transaction, cancellation, crash/recovery and graph tests remain part of
the required workspace suite. This adds executable behavior; a complete ISO
conformance claim still requires the separate normative audit.
