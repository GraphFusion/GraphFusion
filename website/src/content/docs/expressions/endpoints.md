---
title: "Source and destination predicates"
description: "Syntax for testing the endpoint role of a node."
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
MATCH (a)-[e]->(b) WHERE a IS SOURCE OF e AND b IS DESTINATION OF e RETURN e;
```

IS NOT SOURCE OF and IS NOT DESTINATION OF are accepted as well. Executable directed MATCH patterns already constrain endpoint roles; use pattern direction for current queries.

See [supported features](/GraphFusion/start/status/) for executable alternatives.
