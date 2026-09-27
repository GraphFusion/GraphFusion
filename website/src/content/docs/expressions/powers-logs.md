---
title: "Powers, roots and logarithms"
description: "Syntax for SQRT, POWER, LOG, LOG10, LN and EXP."
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
RETURN SQRT(9) AS root, POWER(2, 3) AS power, LOG(10, 100) AS logarithm, LOG10(100) AS common, LN(1) AS natural, EXP(0) AS exponential;
```

LOG takes its base before its value. Function arity is represented by the parser; numeric domain and overflow behavior are not promised without an execution implementation.

See [supported features](/GraphFusion/start/status/) for executable alternatives.
