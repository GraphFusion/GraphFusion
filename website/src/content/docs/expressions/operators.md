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

Arithmetic uses checked kernels. Integer overflow (`integer overflow`), division by zero (`division by zero`) and non-finite floating results raise execution errors in both constant expressions and batch evaluation. Dividing the minimum integer by `-1` overflows. A null dividend divided by zero is null. A decimal literal that overflows to infinity is rejected as a bad number before execution. The binder rejects implicit text-to-number and boolean-to-number conversions.

`AND`, `OR`, and `XOR` decide each row before evaluating an operand that cannot change that row. A division by zero in a skipped operand does not fail the query. An operand the row still needs still raises the arithmetic error.

Comparisons require compatible families, with supported numeric widening. Null operands follow nullable semantics; `NULL = NULL` is not a test for null. Use [IS NULL](/expressions/null/) or a [truth predicate](/expressions/boolean/).

List comparisons use that same numeric widening. An empty list is comparable with a typed list: `[1] = [1.0]` is true, `[1] = []` is false, and `[] < [1]` is true. A list of strings is not comparable with a list of numbers.

List comparisons walk elements from the start. After an equal prefix, the shorter list is less than the longer one. A null element makes the result unknown when that element has to be compared. An earlier decisive element, or a length difference after an equal prefix, still yields true or false. `[1, NULL] = [1, NULL]` and `[1, NULL] = [1, 2]` are unknown; `[1, 3] = [1, 2]` is false, and `[1] < [1, NULL]` is true.

```gql test
RETURN [1, NULL] = [1, NULL] IS UNKNOWN AS unknown_element,
       [1, 3] = [1, 2] AS different,
       [1] < [1, 2] AS prefix;
```

The numeric function `MOD(a, b)` has parser support only; there is no executable remainder operator. Other named numeric functions have their own syntax-only pages.

On a mixed property, `RETURN`, `LET`, and `ORDER BY` apply arithmetic and concatenation to each fragment's stored type; a fragment that cannot apply the operator yields null.
