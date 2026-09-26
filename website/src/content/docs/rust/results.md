---
title: "Arrow query results"
description: "Read schemas, batches, identities and execution diagnostics."
sidebar:
  order: 2
---

QueryResult contains a schema, materialized Arrow record batches and execution metadata. `row_count()` sums rows across batches; the schema is available even when there are no rows or batches.

| Field | Meaning |
| --- | --- |
| `schema`, `batches` | Result columns and their Arrow data |
| `commit_seq` | Snapshot/commit sequence associated with the result |
| `transaction_pending` | Result came from an open transaction and remains provisional |
| `affected_elements` | Write-operation count; zero for ordinary reads |
| `logical_plan`, `physical_plan` | Diagnostics for executed DataFusion plans and materialization barriers |

A result's pending flag does not change after a later COMMIT. The separate COMMIT command result acknowledges publication. Plan strings are diagnostic, not a stable serialization format, and can contain bound parameter values.

## Graph values

Element references are Arrow structs with `__gql_element_graph: UInt64`, `__gql_element_kind: Utf8` (`n` or `e`) and `__gql_element_id: UInt64`. Optional absence is a null struct. These fields are driver encoding, not GQL property names.

Paths are structs with `__gql_path_graph: UInt64`, `__gql_path_nodes: List<UInt64>` and `__gql_path_edges: List<UInt64>`. IDs are in traversal order; a path has one more node than edge. Group variables and ELEMENTS return lists of reference values.

Keep application logic on the documented identity/property operations where possible. The encoding is not a supported way to inject arbitrary reference structs through Value parameters.

## Lifetime

A statement retains its snapshot lease through planning and materialization. Explicit transactions retain it across calls. Holding returned batches alone does not pin old Parquet files. There is no streaming-result API with a longer-lived database cursor.
