---
title: "Build and contribute"
description: "Run the same formatting, lint and test checks used in CI."
sidebar:
  order: 2
---

Use the repository toolchain and committed Cargo lockfile. Rust 1.98 pins development/formatting/lints; the minimum compiler is 1.94 because of the DataFusion dependency. CI also exercises stable Rust on Linux and macOS.

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

Tests cover grammar/AST shape, actual query results, graph/path semantics, write atomicity, transaction conflicts, CLI processes and crash recovery. A parser fixture proves accepted syntax; it does not prove runtime execution. When adding a feature, document the actual supported forms and the explicit limits.

Keep parser responsibilities in `gql-parser`. Planning, catalog lookup, execution and storage belong in the database crate. Test new behavior at the layer where it becomes observable.

Use focused implementation descriptions in commits and pull requests. Maintainers decide when to merge; CI does not merge changes automatically.

[Document a feature](/GraphFusion/development/website/) · [Architecture](/GraphFusion/development/architecture/)
