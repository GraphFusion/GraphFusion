---
title: "Property access"
description: "Read a property from a node or edge value."
sidebar:
  order: 6
---

`element.property` reads a graph property. The base must be a supported element binding or reference alias; arbitrary nested record-field evaluation is not implemented.

```gql test
MATCH (p:Person {name: 'Alice'}) LET person = p
RETURN person.name AS name, person.age AS age;
```

Graph layouts may omit a property. When tables are combined, missing values are projected as typed nulls. All layouts of the same graph and element kind must agree on a property's scalar type.

Aliases, path elements and collected element values preserve their graph source for property lookup. A null element produces null. A deleted non-null identity is invalid for subsequent dereferencing in the same statement.

Use [PROPERTY_EXISTS](/GraphFusion/expressions/property-exists/) to test non-null presence, and [element values](/GraphFusion/patterns/element-values/) for identity and mutation behavior.
