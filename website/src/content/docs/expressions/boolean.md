---
title: "Booleans and UNKNOWN"
description: "Use three-valued boolean logic and truth tests."
sidebar:
  order: 4
---

TRUE and FALSE are concrete booleans. UNKNOWN represents a missing truth value. AND, OR, XOR and NOT propagate unknown according to three-valued logic.

```gql test
RETURN NOT UNKNOWN AS unknown, FALSE AND UNKNOWN AS false_value,
       TRUE OR UNKNOWN AS true_value;
```

Truth predicates include `IS [NOT] TRUE`, `IS [NOT] FALSE` and `IS [NOT] UNKNOWN`. They test the truth category, rather than using ordinary equality with null.

```gql test
RETURN UNKNOWN IS UNKNOWN AS unknown,
       UNKNOWN IS NOT FALSE AS not_false;
```

WHERE and FILTER retain only true rows. Declared BOOLEAN session initializers also accept `NOT UNKNOWN`; the result is a null value subject to the declaration's nullability constraint.
