---
title: "MIN and MAX"
description: "Find the lowest and highest supported input values."
sidebar:
  order: 18
---

MIN and MAX ignore nulls. Empty/all-null input produces null.

```gql test
MATCH (p:Person)
RETURN MIN(p.age) AS youngest, MAX(p.age) AS oldest;
```

This returns 25 and 40 for the quickstart graph. Aggregate input must have a supported comparable type; arbitrary mixed families are not coerced into a common string or number. ALL/DISTINCT syntax is accepted, though duplicates do not change a minimum or maximum.
