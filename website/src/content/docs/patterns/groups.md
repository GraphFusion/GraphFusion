---
title: "Parenthesized and repeated paths"
description: "Compose subpaths with local predicates and repeated groups."
sidebar:
  order: 10
---

Parenthesized subpaths let you constrain a segment before an enclosing selector. Repeating a group joins each iteration's endpoint to the next start.

```gql test
MATCH p = (a {name: 'Alice'})((x)-[edges:Knows]->(y)){0,2}(b)
RETURN b.name AS destination, PATH_LENGTH(p) AS hops ORDER BY hops;
```

New element declarations inside a repeated group become ordered lists outside it. Zero repetitions expose empty lists. Nested groups concatenate their lists. Variables must be fresh relative to the outer bindings; declaration capture is rejected.

```gql test
MATCH p = ALL SHORTEST
  ((a {name: 'Alice'})-[:Knows]->(via)-[:Knows]->(b)
   WHERE via.name = 'Bob')
RETURN b.name AS destination, PATH_LENGTH(p) AS hops;
```

This prefilters the two-edge segment before selecting paths. A repeated group must have positive minimum edge length; otherwise repeated zero-length matches would not make progress. Adjacent node patterns constrain the same node, and consecutive edges introduce an implicit intermediate node.

The implementation composes DataFusion relations and recursive queries. General repeatable unbounded groups and full history-dependent selection are not implemented; provide a finite bound.
