---
title: "OFFSET, SKIP and LIMIT"
description: "Select a window of ordered results."
sidebar:
  order: 9
---

`OFFSET` skips rows; `SKIP` is its synonym. `LIMIT` caps the number of returned rows. Counts must be nonnegative integers, and may use session parameters.

```gql test
MATCH (p:Person)
RETURN p.name AS name ORDER BY name OFFSET 1 LIMIT 1;
```

This returns Bob. Pair pagination with a complete ordering; an unordered subset is not a stable page.

A limit or offset in an earlier clause is applied before a later `ORDER BY`. Sorting afterward reorders that window and does not bring the skipped rows back.

```gql test
FOR n IN [10, 20, 30]
ORDER BY n ASC
OFFSET 1
RETURN n ORDER BY n DESC;
```

```gql test
SESSION SET VALUE $size INTEGER = 2;
MATCH (p:Person) RETURN p.name AS name ORDER BY name LIMIT $size;
```

`LIMIT 0` returns an empty result with its schema. A result limit is not a path-search work limit: the engine may still build many candidate paths before returning a few rows. Use finite quantifiers and [query limits](/storage/limits/) to bound path work.
