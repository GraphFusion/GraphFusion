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

Ordering can refer to output aliases, including element aliases and their properties. With DISTINCT, ordering expressions must depend on projected results. The same arithmetic expression can be repeated:

```gql test
FOR n IN [1, 1, 2]
RETURN DISTINCT n + 1 AS value ORDER BY n + 1 DESC;
```

This returns 3 then 2. Aggregate queries may order by available grouping and aggregate expressions, including an arithmetic grouping value written again in `ORDER BY`. Another input reference is an invalid query.

A float aggregate can also be repeated:

```gql test
FOR n IN [1, 2, 1]
RETURN DISTINCT n AS k, AVG(n) AS average ORDER BY AVG(n) DESC;
```

This returns 2/2.0 then 1/1.0. Sums, minimums, maximums, standard deviations and percentiles follow the same rule.

ORDER BY can also appear in the linear pipeline before the result clause. A later operation can affect row order; put the final ORDER BY with the result when ordering matters to the caller.

An earlier `LIMIT` or `OFFSET` selects its window before a later `ORDER BY`. That later sort reorders only the selected rows.

```gql test
FOR n IN [10, 20, 30]
ORDER BY n ASC
OFFSET 1
ORDER BY n DESC
RETURN n;
```
