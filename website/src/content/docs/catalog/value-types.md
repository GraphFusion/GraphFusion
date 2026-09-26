---
title: "Value type declarations"
description: "Separate declared type validation from physical query types."
sidebar:
  order: 6
---

Value types appear in graph definitions, session declarations and parser-only casts/type predicates. The parser's type vocabulary is broader than the executable value representation.

| Family | Grammar examples |
| --- | --- |
| Boolean | BOOLEAN, BOOL |
| Integers and numerics | INTEGER, BIG INTEGER, INT(64), DECIMAL(8,2), FLOAT(24,2), DOUBLE |
| Characters and bytes | STRING, VARCHAR(64), BYTES(16), VARBINARY(512) |
| Temporal | DATE, TIME, TIMESTAMP, LOCAL TIME, TIME WITH TIME ZONE |
| Lists and arrays | LIST&lt;STRING&gt;, STRING LIST, INTEGER ARRAY[3] |
| Records | RECORD, RECORD {name STRING}, {name STRING} |
| Dynamic unions | INTEGER \| STRING, ANY&lt;STRING \| INTEGER&gt; |
| Graph values | PATH, ANY NODE, EDGE, GRAPH, BINDING TABLE |
| General property values | PROPERTY VALUE |

Declarations can apply NOT NULL. A union accepts null only when its members permit null; combining exclusively non-null types does not create a nullable union.

```gql test
SESSION SET VALUE $limit INTEGER NOT NULL = 10;
SESSION SET VALUE $names LIST<STRING> = ['Alice', 'Bob'];
RETURN $limit AS limit, $names AS names;
```

Query scalars currently normalize integer and approximate numeric values to Int64/Float64 after supported declaration validation. Decimal literals are f64, not an exact decimal implementation. Temporal values, arbitrary unions and structured references do not automatically become executable query scalars because their types parse.

Graph definitions preserve constraints including approximate-numeric scale. Unsupported nesting is rejected before durable publication so a committed definition remains recoverable.
