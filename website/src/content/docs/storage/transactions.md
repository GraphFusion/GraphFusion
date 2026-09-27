---
title: "Transactions"
description: "Commit several catalog and graph statements together."
sidebar:
  order: 1
---

START TRANSACTION captures one snapshot and begins private changes. COMMIT validates and publishes them together; ROLLBACK discards them. The default access mode is READ WRITE. Transactions can span Rust API calls.

```gql test
START TRANSACTION;
MATCH (p:Person {name: 'Alice'}) SET p.age = 31;
MATCH (p:Person {name: 'Bob'}) SET p.age = 41;
COMMIT;
MATCH (p:Person) RETURN p.name AS name, p.age AS age ORDER BY name;
```

Reads see the starting snapshot plus the transaction's own earlier writes. Other sessions see committed snapshots only. START TRANSACTION READ ONLY rejects database writes, including writes that would affect zero rows; session settings remain allowed.

## Failures and state

An execution, parse or transaction-control error fails the active transaction and discards pending changes. Cancelling/dropping an active query future also fails it. ROLLBACK acknowledges the abort and returns the session to idle; SESSION CLOSE ends it. COMMIT cannot publish a failed transaction.

There are no nested transactions or savepoints. Starting while active is an error that fails the existing transaction. COMMIT/ROLLBACK while idle is an error. Successful session-setting changes survive rollback.

## Conflicts and durability

Write commit validates recorded graph and catalog dependencies against the latest committed state. Graph checks are at graph granularity: independent nodes in one graph may conflict. Disjoint graph writes can merge if their read dependencies remain valid. Catalog validation includes absent names, membership and dependency scans.

Results inside a transaction are provisional (`transaction_pending = true`). The separate COMMIT result acknowledges durability. A `CommitUnknown` error requires reopening and inspecting recovered state before deciding whether to retry; rollback cannot undo an outcome that may already be durable.

The one-shot CLI requires the transaction to finish before its file/query ends. Earlier autocommits survive a later failure. See [error handling](/GraphFusion/storage/errors/).
