---
title: "COALESCE and NULLIF"
description: "Replace missing values and turn a sentinel into null."
sidebar:
  order: 5
---

`COALESCE(a, b, ...)` returns the first non-null argument. `NULLIF(a, b)` returns null when a equals b, otherwise a. Arguments must have compatible types.

```gql test
RETURN COALESCE(NULL, 'fallback') AS value,
       NULLIF('deleted', 'deleted') AS missing;
```

Both functions execute through DataFusion. Null-only inputs retain a null type until context supplies a concrete family. A null argument beside a byte string uses that byte-string type, just as a null beside a number or character string uses that family. Unlike these case abbreviations, general [CASE expressions](/expressions/case/) are currently syntax only.

```gql test
RETURN COALESCE(NULL, X'0A') AS value, COALESCE(X'0B', NULL) AS first;
```
