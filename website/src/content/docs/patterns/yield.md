---
title: "Graph-pattern YIELD"
description: "Syntax for explicitly yielding graph-pattern variables."
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
MATCH (n)-[e]->(m) YIELD n, e RETURN n;
```

The parser records the explicit graph-pattern variable list. Runtime graph-pattern YIELD and KEEP are not implemented. An ordinary RETURN can project bound graph elements after an executable MATCH.

See [supported features](/GraphFusion/start/status/) for executable alternatives.
