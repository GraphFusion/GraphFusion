---
title: "REMOVE"
description: "Remove graph properties and labels."
sidebar:
  order: 3
---

REMOVE changes an element's property layout or label set. Other properties and labels are retained.

```gql test
MATCH (p:Person {name: 'Alice'})
REMOVE p.age
RETURN p.name AS name, PROPERTY_EXISTS(p, age) AS has_age;
```

The result reports `has_age = false`. Removing a label uses `REMOVE p IS Label`:

```gql test
MATCH (p:Person {name: 'Alice'}) SET p IS Admin
REMOVE p IS Admin
RETURN p IS LABELED Admin AS admin;
```

Null optional targets are skipped. SET/REMOVE changes remain private until the whole statement succeeds, or until COMMIT inside an explicit transaction. See [write results](/GraphFusion/mutations/results/) for affected-element counts.
