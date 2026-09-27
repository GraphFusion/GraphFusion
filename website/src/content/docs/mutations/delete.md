---
title: "DELETE and DETACH DELETE"
description: "Delete elements with explicit endpoint handling."
sidebar:
  order: 4
---

DELETE (also spelled NODETACH DELETE) removes named elements. Deleting a node is rejected if a remaining edge still references it. You can delete the node and its incident edges together in one DELETE.

```gql test
MATCH (a:Person {name: 'Alice'})-[edge:Knows]->(b)
DELETE edge;
```

DETACH DELETE removes the node and its incident edges:

```gql test
MATCH (p:Person {name: 'Bob'}) DETACH DELETE p;
```

Null targets are skipped. Deleted non-null identities are invalid for later reference/property access in that statement, including through aliases. If a variable contains both live and deleted identities, the current implementation invalidates the entire variable.

Published element IDs are not recycled after deletion within the storage generation. Aborted, never-published IDs can be reused. A failure in a later clause or result rolls back the statement's deletes.
