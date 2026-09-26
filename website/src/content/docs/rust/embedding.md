---
title: "Embed GraphFusion in Rust"
description: "Run a GQL query and consume Arrow batches from a Rust application."
sidebar:
  order: 1
---

The library provides a database handle and independent mutable sessions. The caller supplies the async runtime.

For an application beside your checkout, use a path dependency (adjust the path for your layout):

```toml
[dependencies]
graphfusion = { path = "../GraphFusion/crates/graphfusion" }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

```rust
use graphfusion::{arrow::util::pretty::pretty_format_batches, Database, Value};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let db = Database::new();
    let mut session = db.session();
    session.set_parameter("n", Value::Integer(6))?;
    let result = session
        .query("LET answer = $n * 7 FILTER answer > 40 RETURN answer")
        .await?;
    println!("{}", pretty_format_batches(&result.batches)?);
    Ok(())
}
```

The result is 42. Run the equivalent checked-in example with `cargo run -p graphfusion --example scalar --locked`. `graphfusion::arrow` re-exports the Arrow version matching the engine.

## Choose the execution method

| Method | Input | Result |
| --- | --- | --- |
| `session.execute(gql)` | Catalog/session commands and transaction control | Synchronous command results |
| `session.query(gql).await` | Exactly one read query | QueryResult |
| `session.run(gql).await` | A program of commands, reads and writes | Vec&lt;StatementOutput&gt; |

`query` rejects mixed programs and writes before executing them. `run` returns Command or Query outputs in order and stops at the first error. Earlier autocommits may survive an error; use an explicit transaction for joint publication.

Use `Database::open(path, OpenOptions { create_if_missing: true })` instead of `new` for persistence. No nested runtime is created by the synchronous command API. [Results](/GraphFusion/rust/results/) explains schemas, batches and provisional transaction state.
