---
title: "UPPER and LOWER"
description: "Syntax for character-string case conversion."
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
RETURN UPPER('Alice') AS upper_name, LOWER('Bob') AS lower_name;
```

These calls have dedicated AST forms but no query binder implementation. Transform values before import when case conversion is needed today.

See [supported features](/GraphFusion/start/status/) for executable alternatives.
