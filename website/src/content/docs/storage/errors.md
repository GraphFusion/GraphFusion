---
title: "Errors and retries"
description: "Handle statement failure, conflicts and unknown commit outcomes."
sidebar:
  order: 7
---

Unsupported syntax at execution, invalid definitions/types, resource limits and storage failures are errors, not partially accepted clauses. A failing autocommit statement publishes no private graph/catalog changes, except that an I/O failure during publication can leave an unknown commit outcome.

| Situation | Caller action |
| --- | --- |
| Invalid or unsupported query | Correct the query; parsing alone does not prove execution support |
| Conflict | Re-read state and decide whether the entire operation can be retried |
| Busy checkpoint | Retry when active statements/transactions have released snapshots |
| Failed explicit transaction | ROLLBACK or SESSION CLOSE before further work |
| CommitUnknown | Reopen the database and inspect recovered state before retrying |

Graph conflict detection is coarse: two updates to different nodes in one graph can conflict. Automatic retry of INSERT or another non-idempotent operation can duplicate work after an uncertain outcome, so the engine does not do it implicitly.

A failed multi-statement request does not undo earlier autocommits. Put a logical unit of work inside START TRANSACTION/COMMIT when its statements must publish together. Successful session setting changes are not rolled back with database data.
