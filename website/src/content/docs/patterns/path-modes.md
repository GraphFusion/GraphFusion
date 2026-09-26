---
title: "WALK, TRAIL, SIMPLE and ACYCLIC"
description: "Control which element identities a path may repeat."
sidebar:
  order: 8
---

A path mode constrains the entire path, including its fixed and repeated segments.

| Mode | Repetition rule |
| --- | --- |
| WALK | Nodes and edges may repeat, subject to MATCH-level constraints |
| TRAIL | No repeated edge |
| SIMPLE | No repeated node, except the start may reappear as the final node of a cycle |
| ACYCLIC | No repeated node |

```gql test
MATCH REPEATABLE ELEMENTS p = ACYCLIC (a {name: 'Alice'})-[:Knows]-{1,3}(b)
RETURN b.name AS destination, PATH_LENGTH(p) AS hops ORDER BY hops;
```

WALK is the default path mode, but the default DIFFERENT EDGES match mode still forbids edge reuse within one MATCH. Specify REPEATABLE ELEMENTS when the distinction matters.

A mode inside a repeated parenthesized group constrains each iteration. It does not automatically make an unbounded outer WALK finite. Choose an explicit finite upper bound when composing patterns whose termination cannot be proven.
