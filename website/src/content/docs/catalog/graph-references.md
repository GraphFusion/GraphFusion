---
title: "Graph reference values"
description: "Pass a graph selection through a session parameter."
sidebar:
  order: 11
---

A graph reference carries a catalog object ID, not a copy of graph data. Graph parameters select a graph without spelling its name in every query.

```gql test
SESSION SET GRAPH $g = social;
USE GRAPH $g MATCH (p:Person) RETURN COUNT(*) AS people;
```

The reference-value grammar supports `[PROPERTY] GRAPH <graph expression>`. Graph expressions include names, paths, CURRENT_GRAPH/HOME_GRAPH and their PROPERTY synonyms, reference parameters, VARIABLE expressions and parenthesized value expressions. Runtime resolution requires a supported initializer that yields a live graph reference; arbitrary expression evaluation is not implied.

References remain bound to the original graph identity. Replacing or dropping/recreating a graph makes old references stale. Closed graph declarations require an exact definition match; data access to typed graphs remains unimplemented.

See [parameters](/GraphFusion/catalog/parameters/) and [USE GRAPH](/GraphFusion/query/use-graph/).
