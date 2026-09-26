---
title: "SAME and ALL_DIFFERENT"
description: "Compare graph element identities."
sidebar:
  order: 9
---

`SAME(a, b [, ...])` checks whether the supplied elements share one identity. `ALL_DIFFERENT(a, b [, ...])` checks that their identities are pairwise different.

```gql test
MATCH (a:Person {name: 'Alice'})-[:Knows]->(b)
LET alias = a
RETURN SAME(a, alias) AS same, ALL_DIFFERENT(a, b) AS distinct_people;
```

Both results are true. Identity includes graph, node/edge kind and row ID; matching properties or labels is not enough. Aliases preserve identity. Null element operands participate in nullable predicate semantics, so do not use these as replacements for IS NULL tests.
