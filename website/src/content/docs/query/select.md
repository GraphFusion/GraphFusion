---
title: "SELECT"
description: "Use SELECT with graph patterns, nested sources and aggregate filters."
sidebar:
  order: 4
---

`SELECT` provides a projection-first query form. A graph source supplies the graph for its MATCH pattern.

```gql test
SELECT p.name AS name, p.age AS age
FROM social MATCH (p:Person)
WHERE p.age >= 30
ORDER BY name;
```

`FROM { query }` reads an independent nested result. `FROM graph { query }` also supplies a graph context. The nested query exports its projected names to the outer query.

```gql test
SELECT score, COUNT(*) AS frequency
FROM { FOR score IN [10, 10, 20] RETURN score }
GROUP BY score
HAVING COUNT(*) > 1;
```

`WHERE` filters source rows; `HAVING` filters aggregate groups. `ALL`/`DISTINCT`, `GROUP BY ()`, output aliases and ordering are supported. A nested query cannot reference bindings from the outer query: correlated evaluation is not implemented.

See [grouping](/GraphFusion/query/group-by/) and [nested queries](/GraphFusion/query/nested/).
