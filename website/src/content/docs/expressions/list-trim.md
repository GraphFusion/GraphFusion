---
title: "TRIM for lists"
description: "Syntax for trimming a list by a count."
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
RETURN TRIM([1, 2, 3], 1) AS trimmed;
```

This two-argument list form is distinct from string TRIM. List construction and FOR expansion are executable, but list trimming is not.

See [supported features](/GraphFusion/start/status/) for executable alternatives.
