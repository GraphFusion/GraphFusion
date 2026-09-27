---
title: "SET"
description: "Assign properties, replace a property map and add labels."
sidebar:
  order: 2
---

SET items are evaluated in order. Later items and clauses see preceding changes, including through aliases of the same element.

```gql test
MATCH (p:Person {name: 'Alice'})
SET p.age = p.age + 1, p IS Admin
RETURN p.name AS name, p.age AS age, p IS LABELED Admin AS admin;
```

Replace all properties with a map:

```gql test
MATCH (p:Person {name: 'Alice'})
SET p = {name: 'Alicia', active: TRUE}
RETURN p.name AS name, PROPERTY_EXISTS(p, age) AS has_age;
```

The second form removes properties not listed in the replacement map. An untyped NULL assignment removes the property from the affected layout. Property types must remain consistent across layouts of the graph and element kind.

Identical updates from repeated input rows apply once per identity/payload while preserving result multiplicity. Different values for the same target from different rows are rejected rather than choosing a row arbitrarily. Null targets are skipped. Updates currently rewrite the complete affected graph.
