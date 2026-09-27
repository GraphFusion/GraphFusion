# gql-parser

The GQL lexer, parser, and abstract syntax tree used by
[GraphFusion](https://github.com/GraphFusion/GraphFusion).

These examples target the unreleased 0.2.0 development version. Use the Git
dependency to try the current code:

```toml
[dependencies]
gql-parser = { git = "https://github.com/GraphFusion/GraphFusion.git" }
```

```rust
let program = gql_parser::parse("RETURN 42 AS answer").unwrap();
assert_eq!(program.statements.len(), 1);
```

Parsing checks syntax and produces an AST; it does not execute queries or imply
runtime support. Use the `graphfusion` crate to execute supported GQL programs.
See the [documentation](https://graphfusion.github.io/) for the supported runtime
surface and its limits.

The Rust library name is `gql_parser`, Cargo's default for the package name
`gql-parser`. When upgrading from 0.1.x, replace `graphfusion_gql_parser::` imports
with `gql_parser::`. Both workspace crates use the same version.

## AST inspection

Print a deterministic tree containing every stored AST field:

```rust
use gql_parser::{format_ast, parse};

let program = parse("RETURN 1 + 2 * 3 AS value").unwrap();
println!("{}", format_ast(&program));
```

The `parse_file` example validates files quietly by default. Pass `--dump-ast`
to print their trees, or use `-` to read standard input:

```sh
cargo run -p gql-parser --example parse_file -- --dump-ast query.gql
printf '%s\n' 'RETURN 1 + 2 * 3 AS value' | cargo run -p gql-parser --example parse_file -- --dump-ast -
```

The printer uses `visit::Visitor<'ast>`, which provides a typed callback and a
matching `walk_*` function for every AST type. Override a callback and call its
walker to recurse; omit the call to skip that subtree. Return
`ControlFlow::Break(value)` to terminate early. Use `Infallible` as the visitor's
`Break` type when early termination is unnecessary. Generic node, field, list,
and scalar hooks support tools such as `AstTreePrinter`; the printer can also
visit an individual expression or other subtree.

Traversal follows field declaration order and list order. It covers the stored
structure, including both compatibility and detailed path representations, so
collectors may encounter the same syntactic identifier more than once. Exit
hooks run only when traversal completes normally. Exhaustive field and variant
matching makes AST shape changes require corresponding walker updates.

Output includes defaults, `None`, and empty lists, distinguishing an absent
scope from an explicitly empty scope. Strings are escaped, and field names and
list indices identify each child. This is a diagnostic format whose changes
are reviewed with snapshots, rather than a versioned serialization format.

## Testing

Run the parser crate's tests from the workspace root:

```sh
cargo test -p gql-parser --locked
```

The integration tests use Rust's built-in test harness without additional test
dependencies:

| File under `tests/` | Checks |
| --- | --- |
| `lexer_tests.rs` | Token kinds, decoded text, UTF-8 byte offsets, and lexical errors |
| `parser_valid_tests.rs` | Valid syntax and statement counts |
| `parser_invalid_tests.rs` | Rejected syntax and diagnostic message fragments |
| `parser_error_tests.rs` | Exact error variants, byte offsets, and lexer error propagation |
| `parser_tests.rs` | AST structure and targeted regressions |
| `parser_invariant_tests.rs` | Deterministic generated cases for numeric bases, string escaping, trivia, keyword case, and truncated input |
| `visitor_tests.rs` | Traversal order, borrowed callbacks, skipping, early termination, and balanced hooks |
| `ast_print_tests.rs` | Tree formatting and checked-in GQL/AST snapshot pairs under `fixtures/ast/` |

The `parse_file` example's argument, stdin, and output-error tests also run with
the crate's default test command.

Each valid or invalid fixture expands to an independent named `#[test]`, so a
failure does not stop the remaining fixtures. Run one fixture or suite with:

```sh
cargo test -p gql-parser --test parser_valid_tests basic_match_return -- --exact
cargo test -p gql-parser --test lexer_tests
```

Add syntax acceptance/rejection cases to the corresponding fixture macro. Add
AST assertions for changes to the parsed representation, and structured error
assertions when error kind or location matters. Offsets are byte positions, so
include multibyte input when testing locations.

For an intentional AST change, generate a candidate snapshot and inspect the
diff before replacing the checked-in `.ast` file. Tests never update snapshots
automatically:

```sh
cargo run -q -p gql-parser --example parse_file -- --dump-ast crates/gql-parser/tests/fixtures/ast/precedence.gql > /tmp/precedence.ast
diff -u crates/gql-parser/tests/fixtures/ast/precedence.ast /tmp/precedence.ast
```

The existing workspace CI command, `cargo test --workspace --locked`, discovers
all of these test files automatically.

Rust 1.94 or newer is required. Licensed under Apache-2.0.
