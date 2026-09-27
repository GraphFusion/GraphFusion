---
title: "Embed GraphFusion in Rust"
description: "Run a GQL query and consume Arrow batches from a Rust application."
sidebar:
  order: 1
---

The library provides a database handle and independent mutable sessions. The caller supplies the async runtime.

Add the published library and an async runtime to your application:

```toml
[dependencies]
graphfusion = "0.1.1"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

For development against a local checkout, replace the version with
`graphfusion = { path = "../GraphFusion/crates/graphfusion" }`, adjusting the path for your layout.

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

Use `Database::open(path, OpenOptions { create_if_missing: true })` instead of `new` for persistence. No nested runtime is created by the synchronous command API. [Results](/rust/results/) explains schemas, batches and provisional transaction state.

## Share a database across threads

In 0.2.0, one process exclusively owns each persistent database directory. Clone
`Database` into each worker and create a separate mutable `Session` per worker:

```rust
let mut workers = Vec::new();
for n in 0..4 {
    let database = db.clone();
    workers.push(tokio::spawn(async move {
        let mut session = database.session();
        session.set_parameter("n", Value::Integer(n))?;
        session.query("RETURN $n AS worker").await
    }));
}
for worker in workers {
    let result = worker.await??;
    println!("{} rows", result.row_count());
}
```

Each session has independent graph/parameter/transaction state. Queries execute
concurrently, while commit publication is serialized and conflicting writes may
return `Error::Conflict`. Reopening the same canonical directory in this process
shares the coordinator. Another process receives `Error::DatabaseInUse`, even
while the owner is idle. Drop all database and session handles to release ownership.
