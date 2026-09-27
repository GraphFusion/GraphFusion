---
title: "Shortest paths and selectors"
description: "Choose paths for each start/end pair and input row."
sidebar:
  order: 9
---

Selectors choose among candidate paths for each ordered start/end pair, separately for each incoming row. Path length is the number of edges; edge properties are not weights.

```gql test
MATCH p = ALL SHORTEST (a {name: 'Alice'})-[:Knows]->{0,3}(b)
RETURN b.name AS destination, PATH_LENGTH(p) AS hops ORDER BY destination;
```

| Selector | Selection |
| --- | --- |
| ALL | All eligible paths, retaining multiplicity |
| ANY / ANY k | Up to one / k candidates |
| SHORTEST / ANY SHORTEST | One shortest path |
| ALL SHORTEST | All shortest ties |
| SHORTEST k | Up to k paths in increasing edge count |
| SHORTEST k GROUPS | All paths in the first k distinct length groups |

Counts must be positive signed integer literals or integer parameters. Selection tie-breaking is not a promised result order; add ORDER BY for final presentation. ANY is allowed to choose short candidates.

## Predicates and cost

An inline or parenthesized predicate restricts candidates before an enclosing selector. Final MATCH WHERE filters already selected paths. If it removes a shortest result, selection does not restart to find a longer path.

The current executor enumerates eligible finite candidates before ranking. A shortest-path request can therefore be expensive on a dense graph. Unbounded repeatable WALK is supported for a single homogeneous quantified edge with suitable history-independent predicates and selective prefixes; more general unbounded patterns require finite bounds. See [limits](/GraphFusion/storage/limits/).
