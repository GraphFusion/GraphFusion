---
title: "Example programs"
description: "Runnable programs for graph queries, analytics, paths and transactions."
sidebar:
  order: 5
---

The repository includes complete GQL programs. Each program creates its own graph, so use a fresh in-memory invocation or a new database directory for each run.

```sh
cargo run -p graphfusion --locked -- run --file examples/social.gql
cargo run -p graphfusion --locked -- run --file examples/analytics.gql
cargo run -p graphfusion --locked -- run --file examples/paths.gql
cargo run -p graphfusion --locked -- run --file examples/path-patterns.gql
cargo run -p graphfusion --locked -- run --file examples/element-references.gql
cargo run -p graphfusion --locked -- run --file examples/transactions.gql
```

| Program | What it demonstrates |
| --- | --- |
| [social.gql](https://github.com/GraphFusion/GraphFusion/blob/main/examples/social.gql) | Create a graph, insert a chain, update Alice and find Cara |
| [analytics.gql](https://github.com/GraphFusion/GraphFusion/blob/main/examples/analytics.gql) | OPTIONAL MATCH, aggregation, percentiles, UNION ALL |
| [paths.gql](https://github.com/GraphFusion/GraphFusion/blob/main/examples/paths.gql) | Tied shortest paths and quantified-edge lists |
| [path-patterns.gql](https://github.com/GraphFusion/GraphFusion/blob/main/examples/path-patterns.gql) | Repeated groups and conditional paths |
| [element-references.gql](https://github.com/GraphFusion/GraphFusion/blob/main/examples/element-references.gql) | Element aliases, property lookups and path elements |
| [transactions.gql](https://github.com/GraphFusion/GraphFusion/blob/main/examples/transactions.gql) | Commit and rollback across statements |

Add `--explain` to inspect the DataFusion plans. It executes the program, including writes; it is not a dry run.

## Reference-page examples

Unless a page creates its own graph, graph examples use the Alice → Bob → Cara graph from the [quickstart](/GraphFusion/start/quickstart/), with ages 30, 40 and 25 and the graph selected in the session. Run them after that setup in the same session, or prepend `USE GRAPH social` when reopening a durable database.

Rust examples are available with `cargo run -p graphfusion --example scalar --locked` and `cargo run -p graphfusion --example social --locked`.
