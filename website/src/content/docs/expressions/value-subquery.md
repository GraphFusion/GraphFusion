---
title: "VALUE subqueries"
description: "Syntax for scalar values produced by a nested query."
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
RETURN VALUE { MATCH (n) RETURN COUNT(*) AS total } AS total;
```

The parser constrains scalar-query shapes, including aggregate results and supported single-row forms with LIMIT 1. The runtime does not evaluate scalar VALUE subqueries. Independent SELECT FROM { ... } sources are executable.

See [supported features](/GraphFusion/start/status/) for executable alternatives.
