---
title: "RETURN and FINISH"
description: "Project values, name result columns and suppress returned rows."
sidebar:
  order: 3
---

`RETURN` projects expressions into an Arrow result table. A variable keeps its name; give computed expressions an explicit `AS` alias to make result schemas stable. Otherwise scalar expressions receive names such as `column_1`. A comparison of a property that has different non-numeric types uses each stored type, not the unified text column.

```gql test
MATCH (p:Person)
RETURN p.name AS name, p.age + 1 AS next_age
ORDER BY name;
```

`RETURN ALL` preserves duplicate rows and is the default. `RETURN DISTINCT` removes duplicate projected rows. `RETURN *` expands available bindings, including whole elements where present. An `INSERT` that declares a node or edge variable exposes that variable to `RETURN *`.

```gql test
FOR n IN [1, 1, 2] RETURN DISTINCT n ORDER BY n;
```

Use `FINISH` to complete a pipeline without returning its working bindings. Write statements with no explicit result have an implicit `FINISH`; the CLI reports their affected-element count and commit sequence.

```gql test
MATCH (p:Person {name: 'Alice'}) SET p.age = 31 FINISH;
```

Inside an explicit transaction, result rows describe the pending transaction snapshot. They do not acknowledge durability until COMMIT succeeds. See [result handling](/rust/results/).
