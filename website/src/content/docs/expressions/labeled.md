---
title: "IS LABELED"
description: "Test labels on an element or reference value."
sidebar:
  order: 10
---

Use `IS LABELED` to test a label expression outside the pattern declaration. Negated and colon predicate spellings are recognized as well.

```gql test
MATCH (p:Person)
RETURN p.name AS name, p IS LABELED Person AS person ORDER BY name;
```

The predicate supports direct bindings and reference values, including aliases and elements expanded from path lists. A null element yields null. [Label expressions](/GraphFusion/patterns/labels/) explain conjunction, disjunction, negation and wildcard matching.
