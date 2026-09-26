---
title: "Match modes"
description: "Control edge reuse across the patterns of one MATCH."
sidebar:
  order: 5
---

`DIFFERENT EDGES` prevents the same edge identity from being used by multiple edge occurrences in one MATCH, including disconnected paths. It is the current default.

```gql test
MATCH DIFFERENT EDGES (a)-[first:Knows]->(b)-[second:Knows]->(c)
RETURN a.name AS start, c.name AS destination;
```

`REPEATABLE ELEMENTS` permits reuse at the MATCH level:

```gql test
MATCH REPEATABLE ELEMENTS p = WALK (a {name: 'Alice'})-[:Knows]-{1,3}(b)
RETURN b.name AS destination, PATH_LENGTH(p) AS hops ORDER BY hops, destination;
```

The singular spellings `DIFFERENT EDGE` and `REPEATABLE ELEMENT` are accepted. Every new MATCH starts a new reuse scope. A [path mode](/GraphFusion/patterns/path-modes/) adds its own constraints even under REPEATABLE ELEMENTS.

The default is an implementation compatibility choice, pending a complete normative GQL audit. Multiple paths using selective prefixes in a DIFFERENT EDGES clause are currently rejected: cross-path uniqueness and selection cannot safely be applied in an arbitrary order.
