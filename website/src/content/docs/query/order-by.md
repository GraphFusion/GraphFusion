---
title: "ORDER BY"
description: "Order results by bindings, expressions or result aliases."
sidebar:
  order: 8
---

Without ORDER BY, row order is not guaranteed. Supply all needed tie-breakers when you require a reproducible sequence.

```gql test
MATCH (p:Person)
RETURN p.name AS name, p.age AS age
ORDER BY age DESC, name ASC;
```

`ASCENDING` and `DESCENDING` are synonyms for `ASC` and `DESC`. Nulls compare as greatest by default; override that with `NULLS FIRST` or `NULLS LAST`.

```gql test
FOR n IN [2, NULL, 1]
RETURN n ORDER BY n ASC NULLS LAST;
```

Ordering can refer to output aliases, including element aliases and their properties. With DISTINCT, ordering expressions must depend on projected results. Aggregate queries may order by available grouping and aggregate expressions.

ORDER BY can also appear in the linear pipeline before the result clause. A later operation can affect row order; put the final ORDER BY with the result when ordering matters to the caller.
