---
title: "IS TYPED"
description: "Syntax for testing a value against a type."
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
MATCH (n) WHERE n.age IS TYPED INTEGER RETURN n;
```

The parser supports IS NOT TYPED and the IS :: type spelling, including list, record, reference and union types. Runtime queries cannot yet use these predicates to refine types.

See [supported features](/GraphFusion/start/status/) for executable alternatives.
