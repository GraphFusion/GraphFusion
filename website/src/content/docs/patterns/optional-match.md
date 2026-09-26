---
title: "OPTIONAL MATCH"
description: "Keep input rows when a graph pattern has no complete match."
sidebar:
  order: 2
---

OPTIONAL MATCH preserves each incoming row. If its complete pattern has no match, newly introduced bindings become null; incoming bindings keep their original values.

```gql test
MATCH (p:Person)
OPTIONAL MATCH (p)-[:Knows]->(friend)
RETURN p.name AS name, friend.name AS friend ORDER BY name;
```

The results are Alice/Bob, Bob/Cara and Cara/null. Duplicate incoming rows stay distinct; optional matching does not merge them by value.

A WHERE attached to OPTIONAL MATCH filters candidate matches before null padding. A subsequent FILTER can remove the padded rows.

```gql test
MATCH (p:Person)
OPTIONAL { MATCH (p)-[:Knows]->(friend) MATCH (friend)-[:Knows]->(next) }
RETURN p.name AS name, next.name AS next ORDER BY name;
```

The block succeeds as a whole or pads its new bindings with nulls. Null element properties and ELEMENT_ID return null. COUNT(element) ignores missing elements; null mutation targets are skipped by SET/REMOVE/DELETE. INSERT rejects a null endpoint.

The implementation buffers optional inputs and matches. See [limits](/GraphFusion/storage/limits/) for memory-accounting boundaries.
