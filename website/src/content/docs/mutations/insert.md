---
title: "INSERT"
description: "Create graph elements from each incoming row."
sidebar:
  order: 1
---

INSERT creates nodes and edges in the current open graph. It runs through `Session::run(...).await` or the CLI; the read-only query API rejects writes.

```gql test
MATCH (alice:Person {name: 'Alice'})
INSERT (alice)-[:Knows]->(dana:Person & Admin {name: 'Dana', age: 28})
RETURN dana.name AS name;
```

A bound node appears as a bare reference, such as `(alice)`. New nodes can have labels and properties; edge variables must be new. Incoming/outgoing edges, undirected connections, self-loops and parallel edges are supported.

Creation requires concrete labels: names joined by `&` are accepted, while `!`, `|` and `%` are rejected. Every input row creates its new elements; repeated input rows can therefore insert duplicates. Empty input inserts nothing. A null optional endpoint is an error.

A complete statement publishes atomically after its result pipeline succeeds. A later expression error rolls back that statement's inserts. Multiple semicolon-separated statements auto-commit independently unless enclosed in an [explicit transaction](/GraphFusion/storage/transactions/).
