---
title: "Command-line reference"
description: "Run GQL, import Parquet graphs and checkpoint storage."
sidebar:
  order: 4
---

Run `graphfusion --help` for supported options. The 0.2.0 CLI provides an interactive REPL and one-shot commands. It runs locally and does not listen on a network port.

## Interactive mode

```sh
cargo run -p graphfusion --locked -- ./demo-db
```

Run `graphfusion` for an in-memory session, or `graphfusion ./demo-db` to open or create a persistent database directory. No subcommand or `--create` is needed. `:memory:` explicitly selects in-memory mode. `--database DIR` is an alternative to the positional directory; specify only one. Add `--explain` before or after the directory to print execution plans. Use `graphfusion -- -data` for a directory starting with `-`, or `graphfusion -- run` for one named like a command. `run`, `import`, `checkpoint` and `help` are reserved when they are the first argument. The prompt retains one session: current graph, parameters and explicit transactions survive between inputs. End GQL with a top-level semicolon; strings, comments and nested query bodies may contain semicolons and span lines.

```text
graphfusion> CREATE GRAPH social ANY GRAPH;
graphfusion> SESSION SET GRAPH social;
graphfusion> INSERT (:Person {name: 'Alice'});
graphfusion> MATCH (p:Person) RETURN p.name AS name;
```

| Command | Action |
| --- | --- |
| `\help` | Show help |
| `\graphs` | List committed graphs in the current schema |
| `\status` | Show database path, schema/graph IDs and transaction state |
| `\read FILE` | Execute a file in this session; spaces in paths need no quotes |
| `\explain on` / `\explain off` | Toggle plans; queries and writes still execute |
| `\checkpoint` | Checkpoint this database when no snapshot is active |
| `\clear` | Discard unfinished input, including an unclosed string |
| `\quit` / `\q` | Exit and roll back any uncommitted transaction |

Enter REPL commands on their own line, without a semicolon and with no pending GQL (`\clear` also works during unfinished input). Up/Down and Ctrl-R access in-memory input history. Ctrl-C clears pending input or cancels a running query; cancellation of a statement in an explicit transaction leaves it failed and requires ROLLBACK. Completed autocommits remain committed. Ctrl-D exits and discards unfinished input. History is not written to disk.

The prompt shows `[tx]` in an explicit transaction and `[failed]` when ROLLBACK is required. Query errors keep the REPL running. `SESSION CLOSE;` also exits. Piped REPL input uses the same framing without prompts; errors, unfinished input or an uncommitted transaction at exit return a nonzero status. For whole-file execution without requiring a final semicolon, use `run --file`.

The REPL holds an exclusive process lock on its persistent database even while idle. Another process attempting to open the same directory fails immediately with `DatabaseInUse`. Exit before using a separate `run`, `import` or `checkpoint` command on that directory; `\read` and `\checkpoint` work in the owning REPL.

## run

```text
graphfusion run [--database DIR] [--create]
  (--file FILE | --query GQL) [--explain]
graphfusion run --dump-ast (--file FILE | --query GQL)
```

Without --database, each invocation gets a fresh in-memory database. --create permits creating a database directory and requires --database. Exactly one of --file and --query is required; stdin is not an input mode.

The whole program parses before execution. Catalog/session/query/write statements execute in source order. Semicolon-separated statements auto-commit individually unless an explicit transaction encloses them. An unfinished transaction at the end is rolled back and reported as an error.

--explain executes the program and prints its actual plan diagnostics, including writes. It is not a dry run. Success outputs are buffered; if a later statement fails, earlier autocommits can remain durable even though the invocation returns an error without those buffered outputs.

## Inspect an AST

Use `run --dump-ast` to parse a query or file and print its syntax tree without executing it:

```sh
cargo run -p graphfusion --locked -- run --dump-ast --query 'MATCH (p:Person) RETURN p.name AS name;'
cargo run -p graphfusion --locked -- run --dump-ast --file examples/social.gql
```

The tree includes the complete program, field names and literal values, using the parser's AST visitor. Graphs, labels and variables do not need to exist: this checks syntax, not name binding or runtime support. Queries, writes, transactions and session commands are never executed. This option is available only with `run`, requires exactly one of `--query` or `--file`, and cannot be combined with `--database`, `--create` or `--explain`. It does not open or create a database.

`run --dump-ast` parses the entire query or file before printing, with no final semicolon required. A syntax error produces a diagnostic on stderr, a nonzero exit status and no tree. AST output goes to stdout and can be redirected to a file.

## import

```text
graphfusion import --database DIR --graph EXPR --manifest FILE
```

Replaces an existing open graph with validated external Parquet tables. See [manifest and table layout](/storage/import/).

## checkpoint

```text
graphfusion checkpoint --database DIR
```

Attempts a checkpoint and reclamation. Active snapshots can make it busy. Errors return a nonzero process status and a diagnostic on stderr. There is no automatic retry of uncertain writes.
