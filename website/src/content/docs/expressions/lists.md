---
title: "List and array values"
description: "Construct lists and use compatible nested element types."
sidebar:
  order: 12
---

Plain bracket list constructors execute as Arrow lists. LIST and ARRAY prefixed constructors are accepted by the parser but are not executed.

```gql test
RETURN [1, 2, 3] AS numbers, [] AS empty,
       ['Alice', 'Bob'] AS names;
```

Lists can contain nulls and compatible numeric values. Nested lists find a common supported element type, including empty and all-null sublists. General heterogeneous union-valued lists are not executable.

[FOR](/GraphFusion/query/for/) expands lists. Paths and quantified groups produce lists of graph references; COLLECT_LIST can collect supported values. Explicit list/array type declarations, including maximum lengths, are described under [value types](/GraphFusion/catalog/value-types/).

`TRIM(list, count)` is parsed but not implemented by the query engine. See [list trimming](/GraphFusion/expressions/list-trim/).

## Prefixed constructor syntax

The following forms are **syntax only**. Use plain brackets in executable queries.

```gql parse
RETURN LIST [1, 2, 3] AS numbers, ARRAY [1, 2] AS values;
```
