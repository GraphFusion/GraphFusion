---
title: "Edge directions"
description: "Match outgoing, incoming and undirected relationships."
sidebar:
  order: 4
---

Edge direction is part of both the stored edge table and the pattern. GraphFusion supports directed edges, undirected connections, parallel edges and self-loops.

```gql test
MATCH (b)<-[:Knows]-(a {name: 'Alice'})
RETURN a.name AS source, b.name AS destination;
```

Common pattern forms include:

| Form | Meaning |
| --- | --- |
| `-[]->` | Directed edge from left to right |
| `<-[]-` | Directed edge from right to left |
| `~[]~` | Undirected connection |
| `<-[]->` | Directed edge in either orientation |
| `<~[]~` | Leftward directed edge or undirected connection |
| `~[]~>` | Undirected connection or rightward directed edge |
| `<~[]~>` | Leftward, undirected, or rightward |
| `-[]-` | Any supported orientation |

`<~/label/~>` is the same three-way pattern in simplified form. `<~/label/~` stays left-or-undirected, and `~/label/~>` stays undirected-or-right.

Mixed directed/undirected pattern forms are also parsed and executed. Reversing an orientation does not produce a second copy of a self-loop; parallel edges still produce separate matches. Use [IS DIRECTED](/expressions/directed/) to inspect an edge value.
