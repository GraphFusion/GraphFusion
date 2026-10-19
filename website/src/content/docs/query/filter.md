---
title: "WHERE and FILTER"
description: "Keep rows whose predicate evaluates to true."
sidebar:
  order: 6
---

`WHERE` attaches a predicate to MATCH or SELECT. `FILTER` filters the current working table; `FILTER WHERE` is an accepted spelling. Only true passes: false and unknown/null do not.

Arithmetic and concatenation in predicates use each source fragment's stored property type. An incompatible operator contributes NULL before truth classification or coalescing; an operator unsupported by every selected fragment remains a type error.

A `FILTER` on a bound element's property compares each source fragment in its stored type, including when those fragments disagree. A fragment that cannot compare the value is unknown. A comparison that no selected fragment can evaluate is a type error.

A `MATCH` `WHERE` comparison combined with `TRUE`, `FALSE`, `UNKNOWN`, or a boolean parameter is still evaluated on each fragment's stored type. `OR TRUE` keeps a row when that fragment cannot compare the other side. `OR FALSE` does not.

A `MATCH` `WHERE` that compares an element property with a bound value uses each source fragment's stored type. A string binding does not match an integer that was cast to text, and a fragment that cannot compare the value is unknown.

```gql test
MATCH (p:Person) WHERE p.age >= 30
FILTER p.name <> 'Bob'
RETURN p.name AS name;
```

The result is Alice. Predicates can use bound properties, parameters, scalar operators and the supported element predicates. On an ordinary MATCH, each top-level AND conjunct runs once every variable it reads is bound, so a start-node condition is applied before a later variable-length expansion. A conjunct that names an element declared later stays pending until that element is matched. A WHERE on a selective path still runs after selection.

A property stored as different non-numeric types is compared on each fragment, including inside `AND`, `OR`, `XOR`, `NOT`, `FILTER`, and `LET`. A fragment that cannot compare the value is unknown, so an integer does not match the text it would show after the columns are unified. A parenthesized path predicate compares a property with a literal on each fragment's stored type. An integer comparison stays numeric when another fragment stores that property as text, and a fragment that cannot compare the literal does not match. A property comparison, including one inside `IS [NOT] TRUE`, `IS [NOT] FALSE` or `IS [NOT] UNKNOWN`, uses each fragment's stored type. An integer compared with text is unknown: `IS TRUE` does not keep that row, and `IS UNKNOWN` does. `NOT` still leaves unknown unknown. `COALESCE` and `NULLIF` of a property comparison use each fragment's stored type. An integer compared with the same digits stored as text is unknown, so `COALESCE(n.v = '10', FALSE)` and `NULLIF(n.v = '10', FALSE)` do not keep that integer. `NULLIF(n.v, 10) IS NULL` is true when `v` is the integer 10 or is missing, and unknown when `v` is text. `COALESCE(n.v, 'zz') = '10'` keeps only a fragment that can compare its stored value with the fallback.

A property stored as different non-numeric types is compared on each fragment in `RETURN`, `SELECT`, `SELECT WHERE`, `HAVING`, and `ORDER BY`. A fragment that cannot compare the value is unknown, so an integer does not match the text it would show after the columns are unified. `AND`, `OR`, `XOR`, and `NOT` keep that result.

A comparison between two properties uses each fragment's stored type, including a parenthesized path predicate. An integer is not equal to the same digits stored as text, and integer order is not lexicographic, when another fragment stores that name as a different type.

Arithmetic and concatenation in `FILTER` use each source fragment's stored type. An integer property is not concatenated as its decimal text, and adding zero does not fail merely because another label stores that property as text. A fragment that cannot apply the operator is unknown.

An OPTIONAL MATCH predicate participates in matching before null padding. A later FILTER runs after padding and can remove unmatched rows. Put conditions on the optional neighbor inside OPTIONAL MATCH when you want to keep input rows that have no such neighbor.

A name bound on only some alternatives, or only on the present side of a questioned segment, is null on the other rows. A comparison with that null is unknown and does not keep the row. A repeated group exposes a list, so a property comparison on that name is rejected; write the comparison on the node inside the group to test each repetition.

For path selectors, a MATCH-level WHERE is applied after selection; an inline pattern predicate restricts candidates before selection. This can change shortest-path results. See [selectors](/patterns/selectors/).
