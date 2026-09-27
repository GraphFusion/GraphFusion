---
title: "IS NORMALIZED"
description: "Syntax for testing Unicode normalization."
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
RETURN 'cafe' IS NORMALIZED AS plain, 'cafe' IS NOT NFC NORMALIZED AS nfc;
```

The grammar supports optional normalization forms and NOT. It does not currently perform Unicode normalization checks during query execution.

See [supported features](/GraphFusion/start/status/) for executable alternatives.
