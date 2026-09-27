# GraphFusion

An embedded graph database in Rust, powered by Apache DataFusion. Query property
graphs with GQL, consume Apache Arrow results, and persist data in Apache Parquet.

[Documentation](https://graphfusion.github.io/) ·
[Quickstart](https://graphfusion.github.io/start/quickstart/) ·
[Supported features](https://graphfusion.github.io/start/status/)

## Library

```toml
[dependencies]
graphfusion = "0.1.1"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

```rust
use graphfusion::{arrow::util::pretty::pretty_format_batches, Database};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let db = Database::new();
    let mut session = db.session();
    let result = session.query("RETURN 6 * 7 AS answer").await?;
    println!("{}", pretty_format_batches(&result.batches)?);
    Ok(())
}
```

## CLI

```sh
cargo install graphfusion --version 0.1.1 --locked
graphfusion run --query 'RETURN 6 * 7 AS answer'
```

Rust 1.94 or newer is required. Durable databases currently require Linux or
macOS and a local filesystem. GraphFusion supports graph queries, mutations,
catalog/session commands, explicit transactions, and durable local storage;
it does not claim full ISO GQL conformance. The parser accepts more syntax than
the execution engine supports. Consult the
[documented limits](https://graphfusion.github.io/storage/limits/).

Licensed under Apache-2.0.
