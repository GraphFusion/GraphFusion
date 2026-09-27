---
title: "Path alternatives"
description: "Choose between path branches with set or bag semantics."
sidebar:
  order: 11
---

`|` combines alternative paths and deduplicates equal complete paths with equal exposed bindings for each input row. `|+|` retains overlapping occurrences.

```gql test
MATCH p = (a {name: 'Alice'})-[:Knows]->(b)
        | (a {name: 'Alice'})-[:Knows]->(b)
RETURN b.name AS destination;
```

The duplicated alternative still returns Bob once. Replacing `|` with `|+|` returns two occurrences.

Branches align variables by name and pad absent bindings with typed nulls. Incompatible element kinds or group/singleton exposures are rejected. Equal incoming rows remain separate; deduplication does not collapse the entire input table.

A variable absent from one branch is conditional. Its implicit reuse inside the same MATCH is restricted, but it can appear in expressions or in a later MATCH. See [questioned paths](/GraphFusion/patterns/questioned/) for another source of conditional bindings.
