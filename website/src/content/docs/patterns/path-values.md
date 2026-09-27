---
title: "Path variables"
description: "Return a matched path and inspect its length and elements."
sidebar:
  order: 6
---

Prefix a pattern with `p =` to bind its complete path value. A path preserves traversal order and graph identity.

```gql test
MATCH p = (a {name: 'Alice'})-[:Knows]->(b)-[:Knows]->(c)
RETURN p, PATH_LENGTH(p) AS hops;
```

This returns a path of two edges. A zero-hop path has one node and no edges. A missing path from OPTIONAL MATCH is null, so its PATH_LENGTH is null.

[PATH_LENGTH](/GraphFusion/expressions/path-length/) counts edges. [ELEMENTS](/GraphFusion/expressions/elements/) returns alternating node/edge references that FOR can expand. Path result fields are described in [Arrow results](/GraphFusion/rust/results/).

The parser also recognizes explicit `PATH [...]` constructors; constructing a path that way is not implemented in query execution. Matched paths are the executable source of path values.
