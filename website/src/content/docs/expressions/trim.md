---
title: "String and byte trimming"
description: "Syntax for TRIM, BTRIM, LTRIM and RTRIM."
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
RETURN TRIM(BOTH 'x' FROM 'xxnamexx') AS name, BTRIM('xynameyx', 'xy') AS trimmed;
```

TRIM supports simple and standard LEADING/TRAILING/BOTH forms. BTRIM/LTRIM/RTRIM support optional trimming character sets, including multi-character forms in the parser. Byte-string forms are also recognized. This is separate from TRIM(list, count).

See [supported features](/GraphFusion/start/status/) for executable alternatives.
