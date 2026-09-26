---
title: "ELEMENTS"
description: "Expand a path into alternating node and edge references."
sidebar:
  order: 14
---

ELEMENTS(path) produces `[node, edge, node, ...]` in traversal order. It retains graph identity and the node/edge kind of each reference.

```gql test
MATCH p = (a {name: 'Alice'})-[:Knows]->(b)
FOR element IN ELEMENTS(p) WITH ORDINALITY position
RETURN position, ELEMENT_ID(element) AS id ORDER BY position;
```

This produces three rows. The references can be returned, counted and used for supported property and label lookups. Ordinary scalar/list values are not paths and are rejected. [FOR](/GraphFusion/query/for/) explains null-list and ordinal behavior.
