---
title: "CURRENT_USER"
description: "Syntax for the predefined current-user value."
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
RETURN CURRENT_USER AS user;
```

GraphFusion has no authentication service or user context exposed through this expression. It is a recognized predefined value specification, not the operating-system username returned at runtime.

See [supported features](/GraphFusion/start/status/) for executable alternatives.
