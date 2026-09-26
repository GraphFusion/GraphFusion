---
title: "Catalog inspection and statistics"
description: "Use administrative Rust APIs for directories, snapshots and process-local metrics."
sidebar:
  order: 3
---

`Database::create_directory(&["app", "team"])` provisions a catalog directory hierarchy. Directories organize schemas; they are not storage paths and are not graph nodes.

`Database::with_catalog` exposes a catalog view inside a statement snapshot. Catalog entries carry stable object IDs, kinds, immutable definitions and commit versions. Inspecting the catalog does not mutate session selection.

`Database::statistics()` reports commits, conflicts, lock-wait time, recovery/checkpoint time, busy checkpoints and log bytes. Counts are process-local rather than a cluster-wide monitoring history.

`Database::checkpoint()` requests an idle checkpoint and may return Busy while a statement or transaction in any cooperating process holds a snapshot. See [checkpointing](/GraphFusion/storage/checkpoint/).

Generate Rust API documentation from the checkout with `cargo doc --workspace --no-deps --locked --open`. This uses the exact code revision you are building instead of assuming a published crate version.
