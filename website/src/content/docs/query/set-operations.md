---
title: "UNION, INTERSECT and EXCEPT"
description: "Combine query results with set or bag semantics."
sidebar:
  order: 11
---

These operators default to DISTINCT. Add ALL when duplicate occurrences matter.

```gql test
FOR n IN [1, 1, 2] RETURN n
UNION ALL
RETURN 2 AS n;
```

For a complete row that appears L times on the left and R times on the right:

| Operator | Occurrences |
| --- | --- |
| UNION ALL | L + R |
| INTERSECT ALL | min(L, R) |
| EXCEPT ALL | max(L − R, 0) |

Null fields compare equal for matching result rows. Branch schemas must have the same column names, count and order. Types must match or permit supported null/numeric widening; differently named columns are not automatically aligned.

Each branch has an independent binding scope. A branch-local graph selection does not modify another branch or the session. Only one conjunction kind is accepted at a composite level; use nested query braces to combine levels. Composite writes are rejected before branch execution.
