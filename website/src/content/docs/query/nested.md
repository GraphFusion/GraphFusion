---
title: "Nested queries"
description: "Compose independent query results with braces and SELECT sources."
sidebar:
  order: 13
---

A `{ ... }` query primary forwards the nested query's result unless it explicitly ends with FINISH. SELECT can consume that result as a source.

```gql test
SELECT n, n * 10 AS scaled
FROM { FOR n IN [1, 2, 3] RETURN n }
WHERE n > 1
ORDER BY n;
```

Nested results can retain element references and their graph identity, so outer queries can look up their properties.

```gql test
SELECT person.name AS name
FROM { MATCH (p:Person) RETURN p AS person }
ORDER BY name;
```

Only independent nested queries are executable. Outer bindings are not implicitly captured. Scalar `VALUE { ... }`, EXISTS subqueries and inline CALL have separate grammar but are not executable substitutes for correlated queries.
