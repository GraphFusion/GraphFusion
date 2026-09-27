---
title: "Datetime functions"
description: "Syntax for current date/time values and temporal constructors."
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
RETURN CURRENT_DATE AS day, CURRENT_TIME AS time, CURRENT_TIMESTAMP AS stamp, LOCAL_TIME AS local_time, LOCAL_TIMESTAMP AS local_stamp, DATE('2026-06-29') AS date_value, ZONED_TIME('12:30:00Z') AS zoned, ZONED_DATETIME('2026-06-29T12:30:00Z') AS datetime;
```

DATE, ZONED_TIME, ZONED_DATETIME, LOCAL_TIME and LOCAL_DATETIME constructor forms accept supported string or record syntax. Session time-zone state is a separate feature; setting it does not enable these query functions.

See [supported features](/GraphFusion/start/status/) for executable alternatives.
