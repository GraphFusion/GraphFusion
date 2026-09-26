---
title: "IS DIRECTED"
description: "Inspect whether an edge value is directed."
sidebar:
  order: 11
---

`edge IS DIRECTED` and `edge IS NOT DIRECTED` inspect an edge's stored direction flag.

```gql test
MATCH ()-[edge:Knows]->()
RETURN edge IS DIRECTED AS directed;
```

The predicate accepts edges and edge references, not nodes. A null optional edge produces null. Direction is separate from which orientation a pattern used to traverse the edge; see [edge directions](/GraphFusion/patterns/directions/).
