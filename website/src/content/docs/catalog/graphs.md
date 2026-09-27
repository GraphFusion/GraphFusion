---
title: "CREATE and DROP GRAPH"
description: "Create open graph containers and manage their catalog identity."
sidebar:
  order: 4
---

Create an open graph with ANY GRAPH, then select it for later statements.

```gql test=standalone
CREATE GRAPH social ANY GRAPH;
SESSION SET GRAPH social;
INSERT (:Person {name: 'Alice'});
MATCH (p:Person) RETURN p.name AS name;
```

The parser and catalog also support PROPERTY GRAPH spelling, OR REPLACE, inline graph definitions, named graph types and graph-type reference parameters. Runtime scans/imports/writes currently require an open graph.

DROP GRAPH retires its storage. Checkpoint reclaims obsolete files after snapshots can no longer reference them. Replacing a graph creates new graph and storage identities; stale session references do not silently follow the reused name.

`LIKE` can copy a graph's type definition. Graph data copying through `AS COPY OF <graph>` is not implemented. Do not interpret accepted copy grammar as a data-cloning operation.

[Graph type definitions →](/GraphFusion/catalog/graph-types/)
