# gql-parser

The GQL lexer, parser, and abstract syntax tree used by
[GraphFusion](https://github.com/GraphFusion/GraphFusion).

```toml
[dependencies]
gql-parser = "0.2.0"
```

```rust
let program = gql_parser::parse("RETURN 42 AS answer").unwrap();
assert_eq!(program.statements.len(), 1);
```

Parsing checks syntax and produces an AST; it does not execute queries or imply
runtime support. Use the `graphfusion` crate to execute supported GQL programs.
See the [documentation](https://graphfusion.github.io/) for the supported runtime
surface and its limits.

Version 0.2 uses the Rust library name `gql_parser` (previously
`graphfusion_gql_parser`) and includes AST and grammar changes. Parser versions
are independent of the GraphFusion database version.

Rust 1.94 or newer is required. Licensed under Apache-2.0.
