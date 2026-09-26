---
title: "Identifiers"
description: "Name variables, properties, catalog objects and parameters."
sidebar:
  order: 1
---

Regular identifiers accept Unicode letters and digits. Use delimited identifiers for spaces, punctuation or keyword-like names. Identifier spelling is preserved; catalog names are case-sensitive after decoding.

```gql test
LET `display name` = 'Alice'
RETURN `display name` AS name;
```

The parser accepts both backtick and double-quoted delimited identifiers, escape sequences and no-escape prefixes. Character strings use single quotes. A dot in a delimited identifier belongs to the name; it is not a property-access separator.

Parameter names can be regular (`$name`), extended (`$123`) or delimited (`$"tenant id"`). The leading `$` is GQL syntax; the Rust `set_parameter` API takes the decoded name without it.

```gql test
SESSION SET VALUE $"tenant id" STRING = 'acme';
RETURN $"tenant id" AS tenant;
```

[Catalog paths](/GraphFusion/catalog/paths/) resolve directory/schema names separately from query binding variables.

## Escapes and Unicode

An `@` prefix disables backslash escaping in a delimited identifier. Unicode regular names need no delimiters when they satisfy identifier rules.

```gql test
LET 名字 = 'Alice', @`raw\nname` = 'Bob'
RETURN 名字 AS name, @`raw\nname` AS raw_name;
```

Here the raw identifier contains the literal characters backslash and `n`, not a newline.
