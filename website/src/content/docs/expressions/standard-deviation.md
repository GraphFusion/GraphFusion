---
title: "STDDEV_POP and STDDEV_SAMP"
description: "Compute population or sample standard deviation."
sidebar:
  order: 20
---

STDDEV_POP computes population standard deviation; STDDEV_SAMP uses the sample form. Both accept ALL/DISTINCT and omit null inputs.

```gql test
FOR value IN [10, 20, 30]
RETURN STDDEV_POP(value) AS population, STDDEV_SAMP(value) AS sample;
```

These are numeric aggregates with floating results. Insufficient input for a sample estimate produces null. Empty input produces null, and non-finite aggregate results are errors. See [aggregate behavior](/GraphFusion/expressions/aggregates/).
