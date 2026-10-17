---
title: "Scalar operators"
description: "Arithmetic, comparisons, boolean logic and string concatenation."
sidebar:
  order: 2
---

Queries execute arithmetic (`+`, `-`, `*`, `/`), unary signs, comparisons, AND/OR/XOR/NOT and string concatenation with `||`. `<>` is the standard not-equal spelling.

```gql test
RETURN 6 * 7 AS answer, 7 / 2 AS quotient,
       'Graph' || 'Fusion' AS name, 1 <> 2 AS different;
```

Arithmetic uses checked kernels. Integer overflow (`integer overflow`), division by zero (`division by zero`) and non-finite floating results raise execution errors in both constant expressions and batch evaluation. Dividing the minimum integer by `-1` overflows. A null dividend divided by zero is null. The binder rejects implicit text-to-number and boolean-to-number conversions.

Comparisons require compatible families, with supported numeric widening. Null operands follow nullable semantics; `NULL = NULL` is not a test for null. Use [IS NULL](/expressions/null/) or a [truth predicate](/expressions/boolean/).

The numeric function `MOD(a, b)` has parser support only; there is no executable remainder operator. Other named numeric functions have their own syntax-only pages.
