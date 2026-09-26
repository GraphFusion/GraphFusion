---
title: "NORMALIZE"
description: "Syntax for Unicode string normalization."
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
RETURN NORMALIZE('cafe', NFC) AS composed, NORMALIZE('cafe', NFD) AS decomposed;
```

The normalization form is optional. Accepted form names are grammar-level support; the database does not normalize strings through this function yet.

See [supported features](/GraphFusion/start/status/) for executable alternatives.
