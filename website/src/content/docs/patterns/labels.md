---
title: "Labels and property maps"
description: "Restrict patterns with label expressions and property values."
sidebar:
  order: 3
---

A node or edge can have several labels. Patterns support conjunction (`&`), alternative (`|`), negation (`!`) and wildcard (`%`) label expressions.

```gql test
MATCH (p:Person & !Admin {name: 'Alice'})
RETURN p.name AS name;
```

A property map matches specified values. Inline WHERE allows more general supported expressions:

```gql test
MATCH (p:Person WHERE p.age >= 30)
RETURN p.name AS name ORDER BY name;
```

A missing property is read as a typed null when layouts are combined. It does not equal an ordinary value. [PROPERTY_EXISTS](/GraphFusion/expressions/property-exists/) distinguishes a non-null property from a missing or null one.

INSERT labels have a narrower meaning: creation needs concrete labels, so only names joined by `&` are accepted. A pattern expression such as `Person | Company` cannot tell INSERT which label to create.
