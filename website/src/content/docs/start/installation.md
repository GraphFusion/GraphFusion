---
title: "Build and install"
description: "Install GraphFusion from crates.io or build the CLI and examples from source."
sidebar:
  order: 2
---

GraphFusion 0.1.1 is available as a Rust library and CLI on crates.io. Use the packaged lockfile when installing the CLI to select the tested dependency versions.

## Requirements

- Rust 1.94 or newer, managed by rustup.
- For repository development: Git and the toolchain specified by `rust-toolchain.toml` (currently Rust 1.98).
- Linux or macOS for durable databases. Persistence relies on local filesystem locking, atomic rename and directory synchronization.

The minimum supported Rust version is 1.94. The pinned development version also fixes formatting and lint behavior; it is separate from the minimum compiler supported by the library.

## Install from crates.io

```sh
cargo install graphfusion --version 0.1.1 --locked
graphfusion run --query 'RETURN 6 * 7 AS answer'
```

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

Add `graphfusion = "0.1.1"` to your application's `Cargo.toml`. For development against a checkout, use a path dependency instead. See [embedding in Rust](/rust/embedding/) for a complete application.

[Create a graph →](/start/quickstart/)
