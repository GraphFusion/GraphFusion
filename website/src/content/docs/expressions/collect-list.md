---
title: "COLLECT_LIST"
description: "Collect non-null values or graph references into a list."
sidebar:
  order: 19
---

COLLECT_LIST builds an Arrow list for each group. Null inputs are omitted; an empty group produces an empty list. ALL retains duplicate values and DISTINCT removes them.

```gql test
FOR value IN [1, 1, 2, NULL]
RETURN COLLECT_LIST(DISTINCT value) AS values;
```

The list contains 1 and 2, but its order is not guaranteed. Do not depend on the displayed order of an aggregate list.

Whole elements can be collected while preserving graph identity, then exported through an independent nested query and expanded with FOR for property access. See [element values](/GraphFusion/patterns/element-values/).
