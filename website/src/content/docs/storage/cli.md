---
title: "Command-line reference"
description: "Run GQL, import Parquet graphs and checkpoint storage."
sidebar:
  order: 4
---

The CLI is a one-shot program runner, not an interactive server. Run `graphfusion --help` for its supported options.

## run

```text
graphfusion run [--database DIR] [--create]
  (--file FILE | --query GQL) [--explain]
```

Without --database, each invocation gets a fresh in-memory database. --create permits creating a database directory and requires --database. Exactly one of --file and --query is required; stdin is not an input mode.

The whole program parses before execution. Catalog/session/query/write statements execute in source order. Semicolon-separated statements auto-commit individually unless an explicit transaction encloses them. An unfinished transaction at the end is rolled back and reported as an error.

--explain executes the program and prints its actual plan diagnostics, including writes. It is not a dry run. Success outputs are buffered; if a later statement fails, earlier autocommits can remain durable even though the invocation returns an error without those buffered outputs.

## import

```text
graphfusion import --database DIR --graph EXPR --manifest FILE
```

Replaces an existing open graph with validated external Parquet tables. See [manifest and table layout](/GraphFusion/storage/import/).

## checkpoint

```text
graphfusion checkpoint --database DIR
```

Attempts a checkpoint and reclamation. Active snapshots can make it busy. Errors return a nonzero process status and a diagnostic on stderr. There is no automatic retry of uncertain writes.
