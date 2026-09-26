---
title: "EXISTS"
description: "Syntax for existence predicates over graph patterns and nested queries."
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
MATCH (n) WHERE EXISTS { (n)-[:Knows]->(:Person) } RETURN n;
```

Accepted bodies include graph patterns, nested queries and MATCH statement blocks, with brace or parenthesis forms. This is distinct from executable PROPERTY_EXISTS, which tests one property on an element. For current workloads, express supported relationships with MATCH or OPTIONAL MATCH and aggregate their results.

See [supported features](/GraphFusion/start/status/) for executable alternatives.
