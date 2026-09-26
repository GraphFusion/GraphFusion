---
title: "String and byte lengths"
description: "Syntax for character and byte length functions."
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
RETURN CHAR_LENGTH('Alice') AS chars, CHARACTER_LENGTH('Bob') AS characters, BYTE_LENGTH(X'CAFE') AS bytes, OCTET_LENGTH(X'CAFE') AS octets;
```

Character count and encoded byte count are different operations. These names are parsed into dedicated function forms but are not executable. PATH_LENGTH is an unrelated, executable path function.

See [supported features](/GraphFusion/start/status/) for executable alternatives.
