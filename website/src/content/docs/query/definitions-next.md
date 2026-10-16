---
title: "Definition blocks and NEXT"
description: "Chain query statements and rename the working table with YIELD."
sidebar:
  order: 50
---

A linear query passes its result binding table to the next statement. `NEXT` keeps every column. `NEXT YIELD` keeps and optionally renames the listed columns.

```gql test=standalone
RETURN 1 AS n, 2 AS m NEXT YIELD n AS kept RETURN kept * 3 AS value;
```

`FILTER`, `MATCH`, `LET` and `RETURN` in the continuation read those columns. The current graph carries from one statement in the chain to the next. Semicolon-separated statements stay independent.

```gql test
MATCH (p:Person) RETURN p.name AS name NEXT FILTER name = 'Alice' RETURN name;
```

Mutation statements and catalog commands still do not form a `NEXT` chain. Procedure-body definitions remain syntax:

```gql parse
VALUE answer INTEGER = 42 RETURN answer NEXT YIELD answer AS n RETURN n;
```
