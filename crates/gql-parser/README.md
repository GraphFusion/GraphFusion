# gql-parser

The GQL lexer, parser, and abstract syntax tree used by
[GraphFusion](https://github.com/GraphFusion/GraphFusion).

```toml
[dependencies]
gql-parser = "0.1.1"
```

```rust
let program = graphfusion_gql_parser::parse("RETURN 42 AS answer").unwrap();
assert_eq!(program.statements.len(), 1);
```

Parsing checks syntax and produces an AST; it does not execute queries or imply
runtime support. Use the `graphfusion` crate to execute supported GQL programs.
See the [documentation](https://graphfusion.github.io/) for the supported runtime
surface and its limits.

The Rust library name is `graphfusion_gql_parser`, matching the original 0.1.0
release. Both workspace crates use the same version.

Rust 1.94 or newer is required. Licensed under Apache-2.0.
