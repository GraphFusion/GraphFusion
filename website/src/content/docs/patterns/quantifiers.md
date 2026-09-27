---
title: "Quantified edges"
description: "Match a range of path lengths and expose ordered edge groups."
sidebar:
  order: 7
---

An edge quantifier repeats the edge pattern. Bounds count edges, not nodes.

```gql test
MATCH p = (a {name: 'Alice'})-[edges:Knows]->{0,2}(b)
RETURN b.name AS destination, PATH_LENGTH(p) AS hops ORDER BY hops;
```

This returns Alice/0, Bob/1 and Cara/2. A zero-hop match applies both endpoint constraints to the same node. Intermediate nodes are not implicitly constrained by the endpoint's labels or properties.

| Quantifier | Repetitions |
| --- | --- |
| `{m}` | Exactly m |
| `{m,n}` | From m through n |
| `{m,}` | At least m |
| `{,n}` | Zero through n |
| `*` | Zero or more |
| `+` | One or more |

An explicit upper bound must be positive. Unbounded forms require the planner to prove a finite complete search for the chosen mode/selector; unsupported cases return an error. [Search limits](/GraphFusion/storage/limits/) are checked against the required bound, rather than silently truncating paths.

The `edges` variable above is an ordered list outside the repeated segment. Within an edge predicate it denotes the current edge. Zero repetitions expose an empty list. Group declarations must be fresh.

```gql test
MATCH (a {name: 'Alice'})-[edges:Knows]->{2}(b)
FOR edge IN edges
RETURN ELEMENT_ID(edge) AS id;
```
