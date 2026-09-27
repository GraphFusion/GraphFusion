---
title: "NULL and null predicates"
description: "Test for missing values without equality comparisons."
sidebar:
  order: 3
---

`NULL` represents a missing value. Test it with `IS NULL` or `IS NOT NULL`.

```gql test
RETURN NULL IS NULL AS missing, NULL = NULL AS equality;
```

`missing` is true and `equality` is null. Missing graph properties and absent optional bindings can produce nulls. A property on a null element evaluates to null.

The engine retains typed nulls where context or a declared parameter provides a type. Null-only values are compatible with supported typed expressions; arbitrary incompatible families are still rejected.

[COALESCE](/GraphFusion/expressions/coalesce/) supplies a fallback. [PROPERTY_EXISTS](/GraphFusion/expressions/property-exists/) checks whether a particular element property has a non-null value.
