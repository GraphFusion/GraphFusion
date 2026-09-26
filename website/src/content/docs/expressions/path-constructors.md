---
title: "PATH constructors"
description: "Syntax for constructing a path from element expressions."
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
MATCH (a)-[e]->(b) RETURN PATH [a, e, b] AS path;
```

The constructor records alternating element expressions in the AST. Use a matched path variable such as `MATCH p = (a)-[e]->(b) RETURN p` to obtain an executable path value today.

See [supported features](/GraphFusion/start/status/) for executable alternatives.
