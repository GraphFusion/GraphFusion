---
title: "CASE expressions"
description: "Syntax for searched and simple conditional expressions."
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
RETURN CASE WHEN 18 >= 18 THEN 'adult' ELSE 'minor' END AS category;
```

Both searched CASE and simple CASE with match/predicate operands are parsed. The query engine executes COALESCE and NULLIF, but not general CASE expressions.

See [supported features](/GraphFusion/start/status/) for executable alternatives.
