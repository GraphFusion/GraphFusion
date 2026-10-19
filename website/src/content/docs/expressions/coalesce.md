---
title: "COALESCE and NULLIF"
description: "Replace missing values and turn a sentinel into null."
sidebar:
  order: 5
---

`COALESCE(a, b, ...)` returns the first non-null argument. `NULLIF(a, b)` returns null when a equals b, otherwise a. Arguments must have compatible types.

In a `MATCH` `WHERE` or an inline pattern predicate, a property is evaluated in each fragment's stored type before those types are unified. A fragment that cannot compare the property with the other argument is unknown and does not match. A fragment that does not store the property treats it as null: `NULLIF` of that null is null, and `COALESCE` continues with the fallback.

```gql test
RETURN COALESCE(NULL, 'fallback') AS value,
       NULLIF('deleted', 'deleted') AS missing;
```

Both functions execute through DataFusion. Null-only inputs retain a null type until context supplies a concrete family. A null argument beside a byte string uses that byte-string type, just as a null beside a number or character string uses that family. Unlike these case abbreviations, general [CASE expressions](/expressions/case/) are currently syntax only.

```gql test
RETURN COALESCE(NULL, X'0A') AS value, COALESCE(X'0B', NULL) AS first;
```

`COALESCE` evaluates a fallback only on rows that still need it. Mixing integer and float arguments returns a float; a selected integer must be exactly representable as a float. `NULLIF` compares integer/float pairs by their exact values and keeps the left argument's type.

```gql test
RETURN COALESCE(1, 1 / 0) AS guarded,
       COALESCE(1, 1.5) AS widened,
       NULLIF(9007199254740993, 9007199254740992.0) AS kept;
```
