---
title: "Temporal and interval literals"
description: "Syntax for date, time, datetime, duration and interval literals."
sidebar:
  order: 50
  badge:
    text: Syntax
    variant: caution
---

:::caution[Syntax only]
The GQL parser accepts this form. The database does not currently execute it. The example below demonstrates grammar, not a runnable query feature.
:::

```gql parse
RETURN DATE '2026-06-29' AS day, TIME '12:30:00' AS time, DATETIME '2026-06-29T12:30:00' AS instant, DURATION 'P1D' AS duration, INTERVAL '1 02:03:04' DAY(2) TO SECOND(6) AS interval;
```

TIMESTAMP is also recognized. SQL interval qualifiers include single fields and ranges such as DAY TO SECOND with precisions. There is no general temporal query execution or timezone conversion implied by this syntax.

See [supported features](/GraphFusion/start/status/) for executable alternatives.
