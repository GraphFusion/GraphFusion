---
title: "PROPERTY_EXISTS"
description: "Check whether an element has a non-null property value."
sidebar:
  order: 8
---

The second argument is a property name, not a string-valued expression.

```gql test
MATCH (p:Person)
RETURN p.name AS name, PROPERTY_EXISTS(p, age) AS has_age ORDER BY name;
```

The predicate is false for a missing property or a property whose value is null. For a null element introduced by OPTIONAL MATCH, the result is null rather than false.

Graph storage distinguishes layout membership from a present null value, but this predicate deliberately tests non-null property presence. Use `p.age IS NULL` when you need a null test on the projected value.
