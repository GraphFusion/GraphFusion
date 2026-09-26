---
title: "Arrow and Parquet import"
description: "Load graph tables with explicit identities, labels and endpoints."
sidebar:
  order: 3
---

Build `NodeTable` and `EdgeTable` values, combine them with `GraphData::try_new`, then call `Session::replace_graph_data`. This replaces the selected open graph atomically; it is not an append operation. In an explicit transaction it stages the replacement until COMMIT.

Each table has one exact label set and property layout. Edge tables also have a `directed` flag. Multi-label elements occupy one table row, retaining one identity.

| Column | Required type and meaning |
| --- | --- |
| `__gf_id` | Non-null UInt64 ID, unique per graph and node/edge kind |
| `__gf_source` | Non-null UInt64 source node ID on an edge table |
| `__gf_destination` | Non-null UInt64 destination node ID on an edge table |
| Property columns | Boolean, Int64, Float64, Utf8 or Binary |

Import validates IDs across batches/tables, endpoints, labels, column names, schemas and property types. The `__gf_` prefix is reserved. A property name must have one consistent scalar type within the same graph and element kind. Nodes and edges may use the same numeric ID because their identity kinds differ.

## CLI manifest

```json
{
  "nodes": [{"labels": ["Person"], "file": "people.parquet"}],
  "edges": [{"labels": ["Knows"], "file": "knows.parquet", "directed": true}]
}
```

Paths are relative to the manifest file. After creating an open graph:

```sh
cargo run -p graphfusion --locked -- import --database ./demo-db \
  --graph social --manifest ./data/graph.json
```

Persistent import reads and validates the external files, then writes database-owned immutable Parquet files. The database does not rely on the caller keeping the original files unchanged. Import materializes input; it is not a streaming ingest API. Persistent database paths must be UTF-8 without ASCII control characters.

The [Rust social example](https://github.com/GraphFusion/GraphFusion/blob/main/crates/graphfusion/examples/social.rs) builds actual Arrow tables. CSV/JSON graph-table import and typed graph import are not implemented.
