---
title: "Build and install"
description: "Build the GraphFusion CLI and Rust examples from the repository."
sidebar:
  order: 2
---

GraphFusion currently builds from source. The repository's lockfile selects the tested dependency versions.

## Requirements

- Git and a Rust installation managed by rustup.
- The development toolchain specified by `rust-toolchain.toml` (currently Rust 1.98).
- Linux or macOS for durable databases. Persistence relies on local filesystem locking, atomic rename and directory synchronization.

The minimum supported Rust version is 1.94. The pinned development version also fixes formatting and lint behavior; it is separate from the minimum compiler supported by the library.

## Build the CLI

```sh
 git clone https://github.com/GraphFusion/GraphFusion.git
 cd GraphFusion
 cargo build -p graphfusion --locked
 ./target/debug/graphfusion run --query 'RETURN 6 * 7 AS answer'
```

The result is one row with `answer = 42`. A release build is available with `cargo build -p graphfusion --release --locked`.

To install the CLI from your checkout:

```sh
cargo install --path crates/graphfusion --locked
 graphfusion run --query 'RETURN 42 AS answer'
```

## Use the library

For development against a checkout, add `graphfusion` as a path dependency in your application's `Cargo.toml`. See [embedding in Rust](/GraphFusion/rust/embedding/) for a complete application. This guide does not assume a published crate or downloadable binary release.

[Create a graph →](/GraphFusion/start/quickstart/)
