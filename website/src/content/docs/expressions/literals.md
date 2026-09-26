---
title: "Literals"
description: "Write scalar values with the supported GQL literal forms."
sidebar:
  order: 1
---

Executable query literals include integers, floating-point numbers, booleans, strings, bytes and null. UNKNOWN is represented as a nullable boolean.

```gql test
RETURN 1_000 AS count, 0xFF AS hex, 0o755 AS octal,
       0b1010 AS binary, .5 AS fraction, 1. AS decimal;
```

Integer literals use Int64 and approximate values use Float64 in query execution. `M`, `F` and `D` suffixes are parsed, but an `M` literal is not an exact-decimal runtime value: decimal literals are currently represented as f64.

Character strings support backslash escapes, newline-separated chunks and no-escape literal forms. Use single quotes for strings in examples to avoid ambiguity with delimited names.

```gql test
RETURN 'hello '
'world' AS greeting, 'line\nnext' AS escaped, X'0A ff 10' AS bytes;
```

Byte literals contain hexadecimal pairs; whitespace is allowed between pairs. Newline-separated chunks are also accepted.

Typed DATE/TIME/DATETIME/DURATION and INTERVAL literals are [syntax only](/GraphFusion/expressions/temporal-literals/). A type recognized by the parser is not necessarily representable in a query result.

## No-escape strings

Prefix a single-quoted string with `@` to keep backslashes literally:

```gql test
RETURN @'C:\data\graph' AS location, X'CA'
'FE' AS chunked_bytes;
```

Without `@`, sequences such as `\n` are decoded as escapes. Byte-string chunks contain hex digits, not character-string bytes.
