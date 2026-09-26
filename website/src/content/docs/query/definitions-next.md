---
title: "Definition blocks and NEXT"
description: "Syntax for procedure-body definitions and statement chains."
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
VALUE answer INTEGER = 42 RETURN answer NEXT YIELD answer AS n RETURN n;
```

Procedure bodies can declare GRAPH, [BINDING] TABLE and VALUE variables. NEXT optionally renames yielded bindings before another statement. Runtime programs support semicolon-separated statements, but do not carry query bindings through NEXT; use supported nested sources or session parameters instead.

See [supported features](/GraphFusion/start/status/) for executable alternatives.
