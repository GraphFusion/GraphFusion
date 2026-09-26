---
title: "LEFT and RIGHT"
description: "Syntax for string and byte-string substring functions."
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
RETURN LEFT('GraphFusion', 5) AS prefix, RIGHT(X'010203', 2) AS bytes;
```

LEFT(value, length) and RIGHT(value, length) accept value expressions in the AST. Neither string nor byte substring execution is implemented.

See [supported features](/GraphFusion/start/status/) for executable alternatives.
