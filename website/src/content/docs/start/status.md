---
title: "Supported features"
description: "A practical boundary between executable GQL, accepted syntax and planned capabilities."
sidebar:
  order: 4
---

These pages describe the implementation in the repository's `main` branch. They are not a claim of complete ISO GQL conformance.

**Executable** means the database runs the documented form. **Partial** means specific forms or data types are supported. **Syntax only** means the parser builds an AST, but the database does not execute that feature.

## Database execution

| Capability | Status | Reference |
| --- | --- | --- |
| Scalar queries, parameters, LET, FOR, FILTER | Executable | [Query clauses](/GraphFusion/query/let/) |
| RETURN, SELECT, ordering and pagination | Executable | [RETURN](/GraphFusion/query/return/) |
| MATCH and OPTIONAL MATCH | Executable | [Patterns](/GraphFusion/patterns/match/) |
| Quantified edges, path modes, shortest selectors | Partial: resource and unbounded-search restrictions | [Quantifiers](/GraphFusion/patterns/quantifiers/) |
| Repeated groups, alternatives, questioned paths | Partial: binding restrictions | [Complex patterns](/GraphFusion/patterns/groups/) |
| Aggregation, set operations and independent nested queries | Executable | [Aggregates](/GraphFusion/expressions/aggregates/) |
| Whole-element values, aliases and property lookups | Executable | [Element references](/GraphFusion/patterns/element-values/) |
| INSERT, SET, REMOVE, DELETE | Executable on open graphs | [Writes](/GraphFusion/mutations/insert/) |
| Catalog DDL and graph type definitions | Partial: typed graph data access is not implemented | [Graph types](/GraphFusion/catalog/graph-types/) |
| Sessions and declared parameters | Partial: restricted initializer evaluation | [Session parameters](/GraphFusion/catalog/parameters/) |
| Explicit read-only/read-write transactions | Executable | [Transactions](/GraphFusion/storage/transactions/) |
| Arrow import and local Parquet persistence | Executable | [Import](/GraphFusion/storage/import/) |

## Syntax ahead of execution

The parser also accepts casts, CASE, most string/numeric functions, temporal expressions, record/path constructors, type/normalization/source/destination predicates, EXISTS/VALUE subqueries, typed LET, procedure calls, NEXT chains and graph-pattern YIELD. Their dedicated reference pages state the restriction next to the syntax.

`PATH_LENGTH`, `ELEMENTS`, `COALESCE`, `NULLIF`, aggregate functions and the documented element predicates **are** executable; broad function-family names alone are not a reliable support test.

## Operational boundaries

- Query results are fully materialized; there is no streaming or prepared-statement API.
- Updates rewrite the affected graph; there are no indexes or incremental Parquet deltas.
- Durable storage targets a local Linux/macOS filesystem, not network filesystems or object storage.
- Format v3 has no migration from older formats.
- There is no server protocol, authentication service or distributed transaction coordinator.

Unsupported operations return errors. Successful parsing alone is not an execution or conformance test. See [limits](/GraphFusion/storage/limits/) before running expensive path searches.
