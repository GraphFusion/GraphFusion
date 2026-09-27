---
title: "ELEMENT_ID"
description: "Obtain the opaque identity of a node or edge."
sidebar:
  order: 7
---

`ELEMENT_ID(element)` returns an opaque string that includes graph identity, element kind and row identity. Equal numeric node and edge IDs do not identify the same element.

```gql test
MATCH (p:Person {name: 'Alice'})
RETURN ELEMENT_ID(p) AS id;
```

It works on direct matches, aliases and references expanded from lists. A null optional element produces null. Applications should compare or store the returned value as opaque data rather than parse its textual encoding.

IDs are scoped to the graph's catalog identity. Dropping and recreating a graph under the same name creates a different graph identity. `ELEMENT_ID` is not a supported client-side constructor for reference parameters.
