---
title: "CALL and procedures"
description: "Syntax for named, optional and inline procedure calls."
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
CALL graph.expand($start) YIELD node AS n RETURN n;
```

Named calls, procedure reference parameters (`CALL $proc(...)`), OPTIONAL CALL and inline `CALL [(bindings)] { ... }` bodies are represented in the AST. There is no procedure registry or procedure execution API yet. YIELD here projects procedure outputs and is distinct from graph-pattern YIELD.

See [supported features](/GraphFusion/start/status/) for executable alternatives.
