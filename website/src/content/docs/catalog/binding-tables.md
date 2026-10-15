---
title: "Binding-table references"
description: "Understand parser support and driver-provided table values."
sidebar:
  order: 12
---

Binding tables hold records of named values. A query reads one with `TABLE` or `BINDING TABLE`. Each column becomes a variable in the working table, which later `MATCH` and `FILTER` clauses can use.

```gql test
TABLE { MATCH (p:Person) RETURN p.name AS name }
RETURN name ORDER BY name;
```

A nested query inside `TABLE` executes as an independent binding table. `SESSION SET BINDING TABLE` still accepts a parameter or variable, not a nested query. Pass a driver-owned table by setting `Value::BindingTable` and reading `TABLE $rows`. Columns are scalars: boolean, integer, float, string, bytes or null. An empty table produces no rows.

```gql parse
SESSION SET BINDING TABLE $rows = { RETURN 1 AS id };
```
