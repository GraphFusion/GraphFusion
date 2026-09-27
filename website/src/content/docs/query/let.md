---
title: "LET"
description: "Bind reusable values in a linear query."
sidebar:
  order: 5
---

An untyped `LET` clause evaluates expressions for each incoming row and introduces named bindings. Bindings may contain scalars, lists or graph element references.

```gql test
LET base = 6, answer = base * 7
FILTER answer > 40
RETURN answer;
```

An element alias preserves graph, element kind and identity. Subsequent property lookups resolve that element.

```gql test
MATCH (p:Person {name: 'Alice'})
LET person = p
RETURN person.name AS name;
```

Typed `LET VALUE` definitions are accepted by the parser but not executed in query pipelines. The scalar `LET ... IN ... END` expression is also syntax only; it is distinct from this executable clause. Use [session parameter declarations](/GraphFusion/catalog/parameters/) when you need supported type validation.
