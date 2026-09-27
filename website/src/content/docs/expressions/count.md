---
title: "COUNT"
description: "Count rows or non-null values, including graph elements."
sidebar:
  order: 16
---

COUNT(*) counts incoming rows. COUNT(value) counts non-null values; graph element references are valid values. DISTINCT removes duplicate values before counting.

```gql test
FOR value IN [1, 1, 2, NULL]
RETURN COUNT(*) AS rows, COUNT(value) AS present, COUNT(DISTINCT value) AS distinct_values;
```

This returns 4, 3 and 2. Counting an absent optional element returns zero for that group, while COUNT(*) still counts the padded row. An empty global aggregate returns one row with zero.

See [grouping](/GraphFusion/query/group-by/) and [OPTIONAL MATCH](/GraphFusion/patterns/optional-match/) for per-node counts.
