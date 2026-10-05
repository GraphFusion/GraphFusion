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
| Scalar queries, parameters, LET, FOR, FILTER | Executable | [Query clauses](/query/let/) |
| RETURN, SELECT, ordering and pagination | Executable | [RETURN](/query/return/) |
| MATCH and OPTIONAL MATCH | Executable | [Patterns](/patterns/match/) |
| Quantified edges, path modes, shortest selectors | Partial: resource and unbounded-search restrictions | [Quantifiers](/patterns/quantifiers/) |
| Repeated groups, alternatives, questioned paths | Partial: binding restrictions | [Complex patterns](/patterns/groups/) |
| Aggregation, set operations and independent nested queries | Executable | [Aggregates](/expressions/aggregates/) |
| Whole-element values, aliases and property lookups | Executable | [Element references](/patterns/element-values/) |
| INSERT, SET, REMOVE, DELETE | Executable on open graphs | [Writes](/mutations/insert/) |
| Catalog DDL and graph type definitions | Partial: typed graph data access is not implemented | [Graph types](/catalog/graph-types/) |
| Sessions and declared parameters | Partial: restricted initializer evaluation | [Session parameters](/catalog/parameters/) |
| Explicit read-only/read-write transactions | Executable | [Transactions](/storage/transactions/) |
| Arrow import and local Parquet persistence | Executable | [Import](/storage/import/) |

## Syntax ahead of execution

The parser also accepts casts, CASE, most string/numeric functions, temporal expressions, record/path constructors, type/normalization/source/destination predicates, EXISTS/VALUE subqueries, typed LET, procedure calls, NEXT chains and graph-pattern YIELD. Their dedicated reference pages state the restriction next to the syntax.

`PATH_LENGTH`, `ELEMENTS`, `COALESCE`, `NULLIF`, aggregate functions and the documented element predicates **are** executable; broad function-family names alone are not a reliable support test.

## Operational boundaries

- Query results are fully materialized; there is no streaming or prepared-statement API.
- Writes use WAL-backed MemTables and threshold-based Parquet sealing; updates preserve sealed files with deletion masks. ID/row-position and edge adjacency indexes support mutations. Property indexes and background compaction are not implemented.
- Durable storage targets a local Linux/macOS filesystem, not network filesystems or object storage.
- Format v4 has no migration from older formats.
- There is no server protocol, authentication service or distributed transaction coordinator.

Unsupported operations return errors. Successful parsing alone is not an execution or conformance test. See [limits](/storage/limits/) before running expensive path searches.
