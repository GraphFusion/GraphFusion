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

Arithmetic uses checked kernels. Integer overflow, division by zero and non-finite floating results raise errors in both constant expressions and batch evaluation. The binder rejects implicit text-to-number and boolean-to-number conversions.

Comparisons require compatible families, with supported numeric widening. Null operands follow nullable semantics; `NULL = NULL` is not a test for null. Use [IS NULL](/expressions/null/) or a [truth predicate](/expressions/boolean/).

List comparisons use that same numeric widening. An empty list is comparable with a typed list: `[1] = [1.0]` is true, `[1] = []` is false, and `[] < [1]` is true. A list of strings is not comparable with a list of numbers.

The numeric function `MOD(a, b)` has parser support only; there is no executable remainder operator. Other named numeric functions have their own syntax-only pages.
