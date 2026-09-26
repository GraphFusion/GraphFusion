---
title: "SESSION RESET and CLOSE"
description: "Restore session characteristics or end the session."
sidebar:
  order: 9
---

RESET restores the targeted characteristic to its initial session value or clears parameters. Targets include graph, schema, time zone, a parameter name, ALL PARAMETERS and ALL CHARACTERISTICS.

```gql test
SESSION SET VALUE $limit INTEGER = 10;
SESSION RESET ALL PARAMETERS;
SESSION RESET TIME ZONE;
```

Resetting graph selection can leave no current graph, matching a new session's initial state. Select a live graph before the next graph query.

SESSION CLOSE discards any pending transaction and closes the session. Further work on that session fails; create another session from the database handle if needed. Dropping a session also discards its pending transaction.

Use ROLLBACK when you want to abandon a transaction while keeping the session open. Successful session changes before rollback remain in place.
