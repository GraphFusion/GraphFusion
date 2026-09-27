---
title: "USE GRAPH and AT SCHEMA"
description: "Choose the graph and catalog context for one query."
sidebar:
  order: 2
---

`USE GRAPH` selects a graph for a query without changing the session's current graph. `AT SCHEMA` supplies a schema context for resolving its catalog references.

```gql test
USE GRAPH social
MATCH (p:Person)
RETURN p.name AS name ORDER BY name;
```

Names can be relative, schema-qualified or absolute. `CURRENT_GRAPH` / `CURRENT_PROPERTY_GRAPH` and `HOME_GRAPH` / `HOME_PROPERTY_GRAPH` resolve through session state. Graph reference parameters are also supported.

```gql test
SESSION SET GRAPH $g = social;
USE GRAPH $g MATCH (p:Person) RETURN COUNT(*) AS people;
```

Graph expressions additionally have `VARIABLE <value expression>` and parenthesized value-expression grammar. Execution depends on the expression resolving to a supported graph reference; this is not arbitrary query evaluation.

`AT SCHEMA` recognizes schema names, `CURRENT_SCHEMA`, `HOME_SCHEMA`, `.`, `/`, directory-relative paths and reference parameters. The resolved catalog object must be a schema. [Paths](/GraphFusion/catalog/paths/) explains the distinction between directories, schemas and graphs.

Use `SESSION SET GRAPH` when subsequent statements should share a current graph. Selecting a graph for one branch of a composite query does not change another branch's ambient graph or the session.

## Schema context

```gql test
AT SCHEMA /main USE GRAPH social
MATCH (p:Person) RETURN COUNT(*) AS people;
```

The query resolves `social` in `/main` without changing the session's current schema.
