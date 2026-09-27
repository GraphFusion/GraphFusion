---
title: "Graph type definitions"
description: "Store node/edge schemas and track graph-type dependencies."
sidebar:
  order: 5
---

Graph types describe labels, properties, value types and endpoint constraints. Definitions are bound into engine-owned types and persisted without reparsing their original GQL text on reopen.

```gql test=standalone
CREATE GRAPH TYPE PersonGraph { NODE Person {name STRING, age INTEGER} };
CREATE GRAPH people PersonGraph;
DROP GRAPH people;
DROP GRAPH TYPE PersonGraph;
```

Named and inline definitions, nested node/edge entries, LIKE sources and graph-type `AS COPY OF` sources are supported. Graph-type reference parameters retain catalog identity. Definitions referenced by graphs cannot be dropped or replaced while those dependencies remain.

:::caution[Catalog support]
Storing a typed graph definition does not yet enable its data path. Typed graph import, scans and mutations need further schema binding and constraint enforcement. Use ANY GRAPH for executable graph data examples.
:::

Closed graph parameter types currently require an exact bound-definition match. Type details such as FLOAT precision/scale and NOT NULL participate in that definition; external graph-type imports and data copying are unsupported.

[Value type declarations →](/GraphFusion/catalog/value-types/)
