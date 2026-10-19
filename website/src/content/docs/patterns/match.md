---
title: "MATCH"
description: "Find nodes and relationships with property-graph patterns."
sidebar:
  order: 1
---

MATCH binds graph elements to variables. Parentheses describe nodes; brackets describe edges. A direction arrow restricts the edge orientation.

```gql test
MATCH (a:Person {name: 'Alice'})-[knows:Knows]->(b)
RETURN a.name AS person, b.name AS friend;
```

This finds Alice → Bob. Chain patterns to traverse a fixed number of edges, or use comma-separated patterns to constrain multiple parts of the same match. Reusing a node variable constrains its identity rather than creating another independent node scan.

Labels, property maps and inline WHERE predicates restrict candidates. Ordinary MATCH predicates can refer to elements declared later in that same MATCH; a subsequent MATCH does not bring its variables into the earlier scope. A predicate runs as soon as every variable it reads is bound, so a start-node filter is applied before a later variable-length expansion. A forward reference stays pending until that element is matched. A comparison uses the element's stored property type, including when `OR`, `XOR`, or `NOT` combines comparisons of different elements. A fragment that cannot compare the value is unknown.

```gql test
MATCH (a:Person)-[:Knows]->(b:Person)
WHERE a.age < b.age
RETURN a.name AS younger, b.name AS older;
```

Parallel edges retain distinct identities and can yield duplicate-looking rows. Add DISTINCT only when your application wants to collapse those projected values. [Match modes](/patterns/match-modes/) control edge reuse.
