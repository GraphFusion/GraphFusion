---
title: "OTHERWISE"
description: "Return the first nonempty query result."
sidebar:
  order: 12
---

OTHERWISE evaluates branches in order and returns the first result containing at least one row. A row containing null is still a row.

```gql test
MATCH (p:Person {name: 'Nobody'}) RETURN p.name AS name
OTHERWISE
RETURN 'No match' AS name;
```

The result is `No match`. Later branches are not evaluated once a nonempty result is found, but all branches must pass binding and schema validation. An unsupported function or incompatible result schema in a later branch is therefore still an error.

Like [set operations](/GraphFusion/query/set-operations/), branches must agree on column names, order and compatible types. This is a read-query composition feature, not an exception handler or a write retry mechanism.
