---
title: "Session parameters"
description: "Declare values and graph or binding-table references."
sidebar:
  order: 8
---

`SESSION SET VALUE` declares a parameter with an optional type and initializer. Queries refer to it using `$name`.

```gql test
SESSION SET VALUE $minimum INTEGER NOT NULL = 30;
MATCH (p:Person) WHERE p.age >= $minimum RETURN p.name AS name ORDER BY name;
```

Parameter initializers have a restricted evaluator: literals, existing parameter references, graph references, supported unary expressions, lists and records. They do not execute arbitrary query expressions or nested queries.

```gql test
SESSION SET GRAPH $g = social;
USE GRAPH $g MATCH (p:Person) RETURN COUNT(*) AS people;
```

The parser represents `[PROPERTY] GRAPH <graph expression>` and `[BINDING] TABLE <binding table expression>` reference values, including VARIABLE and parenthesized forms. Graph references resolve through the catalog. Driver-provided binding tables can be assigned through supported variable expressions, but nested-query binding-table initialization is not executed.

From Rust, `set_parameter("minimum", Value::Integer(30))` supplies a decoded name without `$`. Only supported scalar/list families bind into ordinary query value expressions; a session Value variant does not imply general query support for that variant.

Regular, extended numeric and delimited parameter names are documented under [identifiers](/GraphFusion/query/identifiers/).

[Graph reference values](/GraphFusion/catalog/graph-references/) · [Binding-table references](/GraphFusion/catalog/binding-tables/)
