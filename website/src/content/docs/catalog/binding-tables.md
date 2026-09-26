---
title: "Binding-table references"
description: "Understand parser support and driver-provided table values."
sidebar:
  order: 12
---

Binding tables hold records of named values. The AST recognizes `[BINDING] TABLE <binding table expression>`, including nested queries, VARIABLE expressions and parenthesized value expressions.

```gql parse
SESSION SET BINDING TABLE $rows = { RETURN 1 AS id };
```

:::caution[Restricted runtime support]
Nested-query binding-table initializers are not executed. The example above is syntax only. Driver-provided binding-table values can be assigned through supported variable expressions, but general binding-table query sources are not exposed by the runtime.
:::

`Value::BindingTable` is a session value representation, not an assertion that arbitrary structured values can be bound as DataFusion scalar parameters. For executable relational composition, use [SELECT FROM an independent nested query](/GraphFusion/query/nested/).
