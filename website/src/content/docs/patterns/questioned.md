---
title: "Questioned paths"
description: "Make a path segment occur zero or one time."
sidebar:
  order: 12
---

`?` includes both zero- and one-occurrence alternatives. Variables introduced by the segment are null on the absent branch.

```gql test
MATCH p = (a {name: 'Alice'})(-[edge:Knows]->(maybe))?(destination)
RETURN maybe.name AS optional_stop, destination.name AS destination
ORDER BY destination;
```

This returns an absent segment ending at Alice and a present segment ending at Bob. Unlike OPTIONAL MATCH, the zero-occurrence result remains even when a one-occurrence match exists.

`?` also differs from `{0,1}`: questioned declarations are nullable singleton values, whereas quantified declarations are lists. Conditional singletons cannot be implicitly reused within the same MATCH. They can be read by expressions; a later MATCH applies ordinary bound-element identity constraints.
