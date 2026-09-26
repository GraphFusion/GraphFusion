---
title: "LET value expressions"
description: "Syntax for locally scoped LET ... IN ... END expressions."
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
RETURN LET x = 1, VALUE y INTEGER = 2 IN x + y END AS total;
```

This is an expression with local definitions and a result body. It differs from the executable untyped LET query clause. Typed VALUE definitions can be represented in the AST without providing runtime query evaluation.

See [supported features](/GraphFusion/start/status/) for executable alternatives.
