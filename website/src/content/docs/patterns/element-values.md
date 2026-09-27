---
title: "Element values and references"
description: "Preserve graph identity through aliases, lists and query results."
sidebar:
  order: 13
---

Nodes and edges can be returned as values, assigned to LET aliases, collected in lists and used by later patterns. A value identifies its graph, element kind and row identity.

```gql test
MATCH (p:Person {name: 'Alice'}) LET person = p
MATCH (person)-[:Knows]->(friend)
RETURN person, friend.name AS friend;
```

Property lookups and element predicates work on direct bindings, aliases, values expanded from path/group lists and references exported by independent nested queries. Output aliases are resolved in ordering and grouping contexts.

```gql test
MATCH (p:Person) RETURN p AS person ORDER BY person.name;
```

The engine resolves references through DataFusion joins against the statement's graph snapshot. A reference does not turn into a detached copy of its properties. Updates refresh aliases before later expressions; a scalar captured earlier with LET keeps its already computed value.

Deleted identities cannot be dereferenced later in the same statement. Null references behave as nulls. Driver encoding is described in [Arrow results](/GraphFusion/rust/results/); it is not a supported way to manufacture persistent reference parameters from arbitrary client structs.
