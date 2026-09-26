# GraphFusion

**An embedded graph database in Rust, powered by Apache DataFusion.**

Write property-graph queries in GQL. GraphFusion builds DataFusion plans for
pattern matching, recursive paths, aggregation and updates, returns Apache Arrow
batches, and persists graph data in Apache Parquet.

[Documentation](https://graphfusion.github.io/GraphFusion/) ·
[Quickstart](https://graphfusion.github.io/GraphFusion/start/quickstart/) ·
[Supported features](https://graphfusion.github.io/GraphFusion/start/status/)

```gql
MATCH (person:Person {name: 'Alice'})-[:Knows]->(friend)-[:Knows]->(next)
RETURN next.name AS friend_of_friend;
```

## Try it

With Rust installed, clone this repository and run the complete social-graph
example. It creates Alice → Bob → Cara, updates Alice, and finds Cara:

```sh
cargo run -p graphfusion --locked -- run --file examples/social.gql
```

Add `--database ./demo-db --create` to keep the graph in a new local database
directory. Use the [Rust API](https://graphfusion.github.io/GraphFusion/rust/embedding/)
to embed GraphFusion and consume Arrow results directly.

## What works today

- MATCH/OPTIONAL MATCH, quantified and shortest paths, and element references.
- Scalar/list expressions, grouping, aggregates and query composition.
- INSERT/SET/REMOVE/DELETE on open graphs, catalog DDL and isolated sessions.
- Explicit transactions, Arrow/Parquet import and durable local storage.

GraphFusion is under active development. The parser accepts more GQL than the
runtime executes; reference pages distinguish executable features from syntax-only
support. Full ISO GQL conformance is not claimed. See the
[limits](https://graphfusion.github.io/GraphFusion/storage/limits/) before evaluating
a workload.

## Development

`crates/gql-parser` owns the lexer, AST and parser. `crates/graphfusion` owns the
database, sessions, query execution and storage. `website/` contains the
[documentation source](website/src/content/docs/development/website.md).

The minimum supported Rust version is 1.94; `rust-toolchain.toml` pins the
development and lint toolchain. CI checks Linux, macOS and the minimum version.

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

[Architecture](https://graphfusion.github.io/GraphFusion/development/architecture/) ·
[Contributing](https://graphfusion.github.io/GraphFusion/development/contributing/)
