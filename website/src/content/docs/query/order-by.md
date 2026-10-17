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

A property read from several fragments must use one comparable family. Integers and floats stay one family and sort numerically. Boolean, string, and byte-string values do not sort as one another or as numbers. Mixing those families is an invalid query, rather than ordering the string form of each cell. A fragment that does not store the property contributes null and does not add a family.

ORDER BY can also appear in the linear pipeline before the result clause. A later operation can affect row order; put the final ORDER BY with the result when ordering matters to the caller.
