---
title: "Session state"
description: "Use independent graph, schema, timezone and parameter contexts."
sidebar:
  order: 7
---

Each `Database::session()` has its own state: current/home schema, current/home graph, time zone, parameters and query limits. Defaults are `/main`, no graph, UTC and no parameters. State is private to the session and not persisted.

```gql test
SESSION SET GRAPH social;
SESSION SET VALUE $minimum INTEGER = 30;
MATCH (p:Person) WHERE p.age >= $minimum RETURN p.name AS name ORDER BY name;
```

SESSION SET validates into a private copy and publishes the new session state only on success. Graph/schema references store stable object IDs. If an object is dropped or an uncommitted creation is rolled back, its reference becomes stale; a later object with the same name does not repair it.

Successful session-setting and driver-parameter changes survive database ROLLBACK: they are session state, not transactional graph/catalog data. A failed explicit transaction restricts further operations to ROLLBACK or SESSION CLOSE.

[Parameters](/GraphFusion/catalog/parameters/) · [RESET and CLOSE](/GraphFusion/catalog/reset/) · [Time zones](/GraphFusion/catalog/time-zone/)
