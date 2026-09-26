---
title: "Duration functions"
description: "Syntax for DURATION, ABS(duration) and DURATION_BETWEEN."
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
RETURN DURATION('P1D') AS duration, DURATION(RECORD {days: 1, hours: 2}) AS record_duration, ABS(DURATION 'P1D') AS magnitude, DURATION_BETWEEN(DATE '2026-06-29', DATE '2026-06-01') AS difference;
```

Duration strings and records are preserved in the AST. Arithmetic/calendar semantics and a query result representation remain to be implemented.

See [supported features](/GraphFusion/start/status/) for executable alternatives.
