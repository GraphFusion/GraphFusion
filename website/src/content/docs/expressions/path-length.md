---
title: "PATH_LENGTH"
description: "Count the edges of a matched path."
sidebar:
  order: 13
---

PATH_LENGTH returns an Int64 count of edges, not nodes or weighted distance.

```gql test
MATCH p = (a {name: 'Alice'})-[:Knows]->{0,2}(b)
RETURN b.name AS destination, PATH_LENGTH(p) AS hops ORDER BY hops;
```

A zero-hop path has length 0. A missing optional path returns null. Scalar values and ordinary lists are rejected. For path construction and encoding, see [path variables](/GraphFusion/patterns/path-values/).
