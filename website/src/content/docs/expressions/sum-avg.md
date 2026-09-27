---
title: "SUM and AVG"
description: "Compute totals and averages over numeric inputs."
sidebar:
  order: 17
---

SUM and AVG omit null inputs and support ALL/DISTINCT. Use aliases to name result columns.

```gql test
FOR value IN [10, 20, NULL]
RETURN SUM(value) AS total, AVG(value) AS average;
```

The total is 30 and average is 15. Empty/all-null numeric input produces null. Integer SUM uses wide accumulation followed by a checked Int64 conversion; a value outside the result range is an error. Non-finite floating aggregate results are also rejected.

[GROUP BY](/GraphFusion/query/group-by/) applies these aggregates independently to each group. Nested aggregates are not supported.
