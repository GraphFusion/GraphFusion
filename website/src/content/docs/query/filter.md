---
title: "WHERE and FILTER"
description: "Keep rows whose predicate evaluates to true."
sidebar:
  order: 6
---

`WHERE` attaches a predicate to MATCH or SELECT. `FILTER` filters the current working table; `FILTER WHERE` is an accepted spelling. Only true passes: false and unknown/null do not.

```gql test
MATCH (p:Person) WHERE p.age >= 30
FILTER p.name <> 'Bob'
RETURN p.name AS name;
```

The result is Alice. Predicates can use bound properties, parameters, scalar operators and the supported element predicates.

An OPTIONAL MATCH predicate participates in matching before null padding. A later FILTER runs after padding and can remove unmatched rows. Put conditions on the optional neighbor inside OPTIONAL MATCH when you want to keep input rows that have no such neighbor.

A name bound on only some alternatives, or only on the present side of a questioned segment, is null on the other rows. A comparison with that null is unknown and does not keep the row. A repeated group exposes a list, so a property comparison on that name is rejected; write the comparison on the node inside the group to test each repetition.

For path selectors, a MATCH-level WHERE is applied after selection; an inline pattern predicate restricts candidates before selection. This can change shortest-path results. See [selectors](/patterns/selectors/).
