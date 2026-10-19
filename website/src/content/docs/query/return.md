---
title: "RETURN and FINISH"
description: "Project values, name result columns and suppress returned rows."
sidebar:
  order: 3
---

`RETURN` projects expressions into an Arrow result table. A variable keeps its name; give computed expressions an explicit `AS` alias to make result schemas stable. Otherwise scalar expressions receive names such as `column_1`. A comparison of a property that has different non-numeric types uses each stored type, not the unified text column.

A comparison of two properties in `RETURN`, `SELECT`, `HAVING`, or `ORDER BY` uses each fragment's stored type, just as `WHERE` does.

```gql test
MATCH (p:Person)
RETURN p.name AS name, p.age + 1 AS next_age
ORDER BY name;
```

`RETURN ALL` preserves duplicate rows and is the default. `RETURN DISTINCT` removes duplicate projected rows. `RETURN *` expands available bindings, including whole elements where present. An `INSERT` that declares a node or edge variable exposes that variable to `RETURN *`. In a later `NEXT` statement it expands the incoming working table, including after `FILTER`, `ORDER BY`, or `LIMIT`.

```gql test
FOR n IN [1, 1, 2] RETURN DISTINCT n ORDER BY n;
```

Use `FINISH` to complete a pipeline without returning its working bindings. Write statements with no explicit result have an implicit `FINISH`; the CLI reports their affected-element count and commit sequence.

```gql test
MATCH (p:Person {name: 'Alice'}) SET p.age = 31 FINISH;
```

Inside an explicit transaction, result rows describe the pending transaction snapshot. They do not acknowledge durability until COMMIT succeeds. See [result handling](/rust/results/).

Arithmetic and concatenation on a mixed property use each source fragment's stored type. A fragment that cannot apply the operator contributes null. Wrappers such as `COALESCE` can supply a fallback for that null.

`DISTINCT` rejects values that combine incompatible stored families, such as numbers and text. The check follows scalar aliases, including those expanded by `RETURN *`. Integers and floats remain comparable, and nulls do not add a family.
