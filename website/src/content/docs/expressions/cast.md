---
title: "CAST"
description: "Syntax for explicitly converting a value to a declared type."
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
RETURN CAST('42' AS INTEGER) AS answer;
```

The target uses the value-type grammar, including NOT NULL and compound types. Parsing a target type does not provide a runtime conversion implementation. Declared session parameters perform supported validation; they do not implicitly cast arbitrary strings to numbers.

See [supported features](/GraphFusion/start/status/) for executable alternatives.
