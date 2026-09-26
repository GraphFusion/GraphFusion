---
title: "FOR and list expansion"
description: "Expand a list into rows with optional position bindings."
sidebar:
  order: 7
---

`FOR item IN list` produces one row per list element for each incoming row. Empty and null lists produce zero rows. A null entry in a nonempty list produces a row containing null.

```gql test
FOR name IN ['Alice', 'Bob', 'Cara'] WITH ORDINALITY position
RETURN position, name ORDER BY position;
```

`WITH ORDINALITY` counts from 1; `WITH OFFSET` counts from 0. Positions restart for each incoming row.

```gql test
FOR value IN [10, NULL, 30] WITH OFFSET position
RETURN position, value ORDER BY position;
```

Supported lists include homogeneous scalar lists, compatible numeric lists, nested lists and lists of graph references produced by paths or collection. Arbitrary heterogeneous union-valued lists are not represented by the query engine.

See [list values](/GraphFusion/expressions/lists/) and [ELEMENTS](/GraphFusion/expressions/elements/).
