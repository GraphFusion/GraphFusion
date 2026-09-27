---
title: "FLOOR, CEIL and CEILING"
description: "Syntax for rounding numeric values to integral boundaries."
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
RETURN FLOOR(1.5) AS down, CEIL(1.5) AS up, CEILING(1.5) AS also_up;
```

CEIL and CEILING are accepted spellings of the same function kind. These functions are separate from the supported basic arithmetic operators.

See [supported features](/GraphFusion/start/status/) for executable alternatives.
