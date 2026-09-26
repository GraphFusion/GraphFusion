---
title: "Record constructors"
description: "Syntax for record-valued query expressions."
sidebar:
  order: 50
  badge:
    text: Syntax
    variant: caution
---

:::caution[Syntax only]
The GQL parser accepts this form. The database does not currently execute it. The example below demonstrates grammar, not a runnable query feature.
:::

```gql parse
RETURN RECORD {name: 'Alice', age: 30} AS person, {active: TRUE} AS flags;
```

RECORD is optional before a record constructor, and records may nest. Query projection and arbitrary record.field evaluation are not implemented. Restricted session initializers do support record values and type validation; that separate evaluator does not make general record queries executable.

See [supported features](/GraphFusion/start/status/) for executable alternatives.
