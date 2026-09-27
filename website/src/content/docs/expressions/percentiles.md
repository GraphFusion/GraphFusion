---
title: "PERCENTILE_CONT and PERCENTILE_DISC"
description: "Compute interpolated or observed numeric percentiles."
sidebar:
  order: 21
---

The input value comes first and the fraction second. Fractions must be row-independent numeric expressions in [0, 1]. A null fraction returns null.

```gql test
FOR value IN [10, 20, 30, 40]
RETURN PERCENTILE_CONT(value, 0.5) AS continuous,
       PERCENTILE_DISC(value, 0.5) AS discrete;
```

The continuous result interpolates (25 here) and has Float64 type. The discrete result chooses an input value (20 here) and preserves its numeric type, including exact Int64 values. Null inputs are omitted; ALL/DISTINCT controls duplicate contribution.

Out-of-range or row-dependent fractions are errors. Use [GROUP BY](/GraphFusion/query/group-by/) for per-group percentiles.
