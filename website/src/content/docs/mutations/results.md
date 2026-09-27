---
title: "Write pipelines and results"
description: "Understand atomicity, FINISH and affected-element counts."
sidebar:
  order: 5
---

Write pipelines can combine MATCH, OPTIONAL MATCH, LET, FOR and FILTER with INSERT, SET, REMOVE and DELETE, then end with RETURN or FINISH. A modifying statement without a result clause has an implicit FINISH.

```gql test
MATCH (p:Person) WHERE p.age >= 30
SET p.age = p.age + 1
RETURN p.name AS name, p.age AS age ORDER BY name;
```

The complete statement is one atomic unit, including its returned expressions. Catalog/graph changes are not published if a result expression fails. Semicolon-separated statements remain separate autocommit units unless you start a transaction.

`QueryResult::affected_elements` counts inserted/deleted elements and distinct targets per update item. Two update items can count the same element twice. It is an operation count, not a unique-element count for the whole program. FINISH reports it with a commit sequence in the CLI.

Composite writes using UNION/INTERSECT/EXCEPT/OTHERWISE and query NEXT continuations are not supported. Typed graph writes also require future constraint enforcement. [Limits](/GraphFusion/storage/limits/) describes full-graph materialization and rewrite costs.
