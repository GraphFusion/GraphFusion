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

An explicit upper bound must be positive. Unbounded forms require the planner to prove a finite complete search for the chosen mode/selector; unsupported cases return an error. [Search limits](/storage/limits/) are checked against the required bound, rather than silently truncating paths.

A quantifier after a simplified path repeats the whole slash pattern. `-/E F/->{2}` is two E-then-F sequences, not one E and two F edges. `-/L|R/->{1,2}` may mix the labels from one repetition to the next. A quantifier written on a label stays on that edge and does not discard the outer quantifier. Repeating a slash pattern whose minimum length is zero is rejected, because the repeated primary would not make progress.

A selective search (`ANY`, `SHORTEST`, or a shortest-length group) of one homogeneous unbounded edge stops at a complete hop bound derived from the node count and the number of paths requested. That search does not enumerate every trail out to the edge count. `ALL` keeps the match mode's own limit.

An anonymous unconstrained start of a quantifier whose lower bound is at least one, such as `()-[:Knows]->+()`, reads that edge directly. It does not first cross-join every node to recover a source the edge already stores. Naming, labeling, or otherwise constraining the start still reads the node tables. A quantifier that allows zero repetitions, including `*` and `{0,n}`, still anchors on every node so a node with no matching edge can produce the empty walk.

The `edges` variable above is an ordered list outside the repeated segment. Within an edge predicate it denotes the current edge. Zero repetitions expose an empty list. Group declarations must be fresh.

```gql test
MATCH (a {name: 'Alice'})-[edges:Knows]->{2}(b)
FOR edge IN edges
RETURN ELEMENT_ID(edge) AS id;
```
