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

`COALESCE` and `NULLIF` of a property comparison use each fragment's stored type. An integer compared with the same digits stored as text is unknown, so `COALESCE(n.v = '10', FALSE)` and `NULLIF(n.v = '10', FALSE)` do not keep that integer. `NULLIF(n.v, 10) IS NULL` is true when `v` is the integer 10 or is missing, and unknown when `v` is text. `COALESCE(n.v, 'zz') = '10'` keeps only a fragment that can compare its stored value with the fallback.

An OPTIONAL MATCH predicate participates in matching before null padding. A later FILTER runs after padding and can remove unmatched rows. Put conditions on the optional neighbor inside OPTIONAL MATCH when you want to keep input rows that have no such neighbor.

For path selectors, a MATCH-level WHERE is applied after selection; an inline pattern predicate restricts candidates before selection. This can change shortest-path results. See [selectors](/patterns/selectors/).
