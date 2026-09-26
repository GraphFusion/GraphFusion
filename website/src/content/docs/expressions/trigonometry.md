---
title: "Trigonometric functions"
description: "Syntax for trigonometric, hyperbolic and angle-conversion functions."
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
RETURN SIN(0) AS sine, COS(0) AS cosine, TAN(0) AS tangent, COT(1) AS cotangent, SINH(0) AS sinh_value, COSH(0) AS cosh_value, TANH(0) AS tanh_value, ASIN(0) AS arcsine, ACOS(1) AS arccosine, ATAN(0) AS arctangent, DEGREES(1) AS degrees_value, RADIANS(180) AS radians_value;
```

These numeric function names are accepted in value expressions. Their presence in the grammar is not a claim that DataFusion functions are automatically exposed under GQL names.

See [supported features](/GraphFusion/start/status/) for executable alternatives.
