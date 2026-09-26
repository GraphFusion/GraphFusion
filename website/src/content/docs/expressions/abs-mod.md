---
title: "ABS and MOD"
description: "Syntax for absolute value and named remainder operations."
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
RETURN ABS(-3) AS magnitude, MOD(7, 3) AS remainder;
```

Numeric ABS and MOD are not executed. There is no executable remainder operator. ABS also has duration grammar, whose runtime representation is not implemented.

See [supported features](/GraphFusion/start/status/) for executable alternatives.
