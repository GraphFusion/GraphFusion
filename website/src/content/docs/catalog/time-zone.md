---
title: "Session time zone"
description: "Set UTC or a supported numeric offset in session state."
sidebar:
  order: 10
---

The session starts in UTC. Supported settings are UTC and numeric time-zone offsets.

```gql test
SESSION SET TIME ZONE '+08:00';
SESSION RESET TIME ZONE;
```

IANA names such as Asia/Shanghai are not evaluated. The setting is session-local and nonpersistent. RESET restores the initial value.

Temporal query literals, constructors and current-date/time functions are still syntax only. Setting a time zone does not enable those operations or add a general timezone database.
