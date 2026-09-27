---
title: "GROUP BY and HAVING"
description: "Group rows and filter aggregate results."
sidebar:
  order: 10
---

When a result contains aggregates without an explicit GROUP BY, nonaggregate projected expressions that refer to input values become grouping keys.

```gql test
MATCH (p:Person)
OPTIONAL MATCH (p)-[:Knows]->(friend)
RETURN p.name AS name, COUNT(friend) AS friends
GROUP BY name ORDER BY name;
```

This returns Alice: 1, Bob: 1 and Cara: 0. COUNT(friend) ignores the null optional binding.

Explicit GROUP BY accepts binding variables, including output aliases. Grouping an element groups its identity while retaining the columns needed for property access. `GROUP BY ()` chooses one global group.

```gql test
SELECT COUNT(*) AS people FROM social MATCH (p:Person)
GROUP BY () HAVING COUNT(*) > 0;
```

SELECT HAVING filters completed groups. Ungrouped input references and nested aggregate calls are rejected. An empty global aggregate still returns one row: COUNT is 0, COLLECT_LIST is an empty list, and numeric aggregates are null.

[Aggregate functions →](/GraphFusion/expressions/aggregates/)
