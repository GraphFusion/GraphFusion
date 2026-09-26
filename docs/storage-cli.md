# Parquet storage and CLI

GraphFusion persists open property graphs in immutable Parquet files and uses
DataFusion `ListingTable` providers to scan them. GQL planning produces the same
DataFusion relational plan for memory and durable graphs. No SQL translation or
separate row executor is involved.

## Run a GQL program

```sh
cargo run -p graphfusion --locked -- run --database /tmp/my-graph --create \
  --query "CREATE GRAPH social ANY GRAPH; SESSION SET GRAPH social; RETURN 42 AS answer"
cargo run -p graphfusion --locked -- run --database /tmp/my-graph --file query.gql --explain
cargo run -p graphfusion --locked -- checkpoint --database /tmp/my-graph
```

Without `--database`, `run` uses a fresh in-memory database. Existing directories
are opened without creation unless `--create` is supplied. `--file` and `--query`
are mutually exclusive. The whole input is parsed before execution; semicolons
inside strings and comments are handled by the GQL parser. Errors return a
nonzero exit code. Successful queries print Arrow tables; `--explain` also prints
the logical and physical plans. It executes the query, so it is not a plan-only
operation. Empty result tables retain column names and types in the Rust API.

The matching Rust entry point is `Session::run(...).await`, returning a sequence
of `StatementOutput::{Command, Query}`. Top-level statements commit individually.
An execution error leaves earlier successful statements committed and returns an
error rather than their buffered results. Explicit transactions are rejected
before execution. Catalog `NEXT` groups remain atomic; query continuations,
procedures, and GQL mutations are unsupported. A single query can use `AT SCHEMA`;
program-level schema context for mixed commands remains unsupported.

## Import existing Parquet tables

Create the destination open graph first. An import manifest uses paths relative
to the manifest's directory (absolute paths are also accepted):

```json
{
  "nodes": [
    {"labels": ["Person"], "file": "people.parquet"}
  ],
  "edges": [
    {"labels": ["Knows"], "directed": true, "file": "knows.parquet"}
  ]
}
```

```sh
cargo run -p graphfusion --locked -- import --database /tmp/my-graph \
  --graph social --manifest import.json
cargo run -p graphfusion --locked -- run --database /tmp/my-graph \
  --query "USE GRAPH social MATCH (a)-[:Knows]->(b) RETURN b.name AS name" --explain
```

Each table has one label set and property layout. Nodes require non-null UInt64
`__gf_id`; edges also require non-null UInt64 `__gf_source` and
`__gf_destination`. [Arrow import rules](graphs.md) apply, including globally
unique IDs within each element kind, valid endpoints, consistent property types,
and non-null required fields. `NodeTable::read_parquet` and
`EdgeTable::read_parquet` expose the same external-file loader to Rust callers.

Imports materialize external files into Arrow for graph-wide validation and then
call `Session::replace_graph_data`. They replace the complete current graph,
copying its tables into owned files. The original files may then be removed.
The importer currently requires memory proportional to the full input and is not
a streaming bulk loader. Empty manifests replace the graph with an empty graph;
empty tables retain their property schema. Typed graph import awaits schema
enforcement and is rejected before staging files.
Persistent graph imports require a UTF-8 database path without ASCII control
characters. Unsupported paths are rejected before staging graph files.

For a self-contained example, use a new directory:

```sh
cargo run -p graphfusion --example social --locked -- /tmp/graphfusion-social
cargo run -p graphfusion --locked -- run --database /tmp/graphfusion-social \
  --query "USE GRAPH social MATCH (a {name: 'Alice'})-[:Knows]->(b)-[:Knows]->(c) RETURN c.name AS friend"
```

The second command runs in a separate process and returns `Cara` from Parquet.

## Publication, snapshots, and recovery

1. A statement takes a shared lifecycle lease and a catalog/storage snapshot.
2. File IDs are reserved through the durable ID allocator. Each validated table
   is written to a new `graph-ID.parquet` file using `create_new`, closed, and
   fsynced. The database directory is fsynced before any references are committed.
3. Optimistic validation checks the graph's catalog identity and data version.
   One WAL commit publishes catalog deltas and the complete graph manifest
   together. The manifest records schemas, labels, directedness, sizes, row
   counts, and file IDs. A failed/aborted import can leave unreferenced files.
4. Readers plan and scan exact immutable files through DataFusion while retaining
   their lifecycle lease. Replacing or dropping a graph cannot change an active
   reader's files. A checkpoint reports Busy while any process has an active
   statement, including an import that is still staging files.
5. After publishing and syncing a new checkpoint, under an exclusive lease,
   cleanup removes unreferenced managed Parquet files. This includes replaced,
   dropped, aborted, and crash-orphaned files. Interrupted/failed cleanup is
   retried by a later checkpoint. Files are never reclaimed before the checkpoint
   manifest is durable.

Recovery validates metadata frames and replays the WAL before checking the final
referenced files. Missing files, changed sizes, invalid Parquet footers, schema
changes, and row-count changes fail closed. File checks do not hash every data
page; this is not a guarantee against arbitrary same-size external data edits.
Database files must be managed exclusively by GraphFusion. The storage contract
requires local Linux/macOS filesystems with working locks, fsync, and atomic
rename; cloud object stores and Windows are not yet supported by the coordinator.

This task introduces database format version 2. Older formats are rejected and
there is no migration command. Metadata documents retain their 64 MiB bound;
graph rows are separate Parquet files. Imports currently write one file per table
without compaction or partitioning. Recovery replays the full WAL and checks
file footers at statement start; large-catalog optimization is still future work.

## Verification

Tests exercise crashes after file write/fsync/directory sync, incomplete and
complete graph WAL frames, publication, and checkpoint cleanup. They verify joint
catalog/data recovery, conflict-orphan reclamation, a separate process retaining
an old scan across replacement and DROP, and missing/truncated/corrupt files.
Integration tests compare Arrow and Parquet query results for multi-label layouts,
null/missing properties, parallel edges, directed and undirected traversal, and
multi-hop joins. CLI tests use independent processes for import, query,
checkpoint, and reopen, including paths with spaces, URL-special characters,
and literal glob characters. Unscannable paths are rejected before import.

These checks establish the implemented storage and CLI behavior. Full GQL
conformance, graph mutations, and explicit transactions remain separate tasks.
