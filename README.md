# GraphFusion

GraphFusion is a Rust graph database project. The repository is organized as a
Cargo workspace so database components can evolve independently while sharing a
single build and test surface.

The target architecture implements GQL over Apache DataFusion, with Arrow and
Parquet storage. See the [architecture](docs/architecture.md) and the
[PR delivery plan and capability ledger](docs/roadmap.md). The current main branch
provides parsing; runtime features are delivered in separate reviewed PRs.

## Development and CI

The minimum supported Rust version is 1.94 (required by DataFusion 55.1).
`rust-toolchain.toml` pins the
development toolchain and format/lint versions; CI additionally tests stable Rust
on Linux and macOS and the minimum version on Linux. All Cargo checks use the
committed lockfile.

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

The [CI workflow](.github/workflows/ci.yml) runs on pull requests, main pushes,
development branch pushes (`codex/**`), and manual dispatch. It uses read-only
permissions and never merges PRs. Maintainer approval is required for merging.

## DataFusion Queries

`Session::query(...).await` executes GQL queries through DataFusion and
returns Arrow batches, their schema, and logical/physical plans. It supports
parameters, LET/FOR/FILTER, RETURN/SELECT, scalar operators, pagination,
fixed-length MATCH/OPTIONAL MATCH, grouping/aggregates and composite queries over
Arrow or Parquet graph tables. Run
`cargo run -p graphfusion --example scalar --locked` for an executable example.
See [query execution](docs/query.md), [graph import and MATCH](docs/graphs.md), and
[relational queries](docs/relational.md). The [analytics example](examples/analytics.gql)
shows optional aggregation, percentiles and UNION ALL.
`cargo run -p graphfusion --example social --locked` runs a two-hop social graph
query. Pass a new database directory after `--` to persist the example in Parquet.
`Session::run(...).await` also executes [GQL graph mutations](docs/mutations.md).
The [pure GQL social example](examples/social.gql) creates and updates a graph:

```sh
cargo run -p graphfusion --locked -- run --database /tmp/gql-social --create --file examples/social.gql
```

## CLI and Parquet storage

The CLI runs a GQL file or query, imports external Parquet tables, and checkpoints
a persistent database. For example:

```sh
cargo run -p graphfusion --example social --locked -- /tmp/graphfusion-social
cargo run -p graphfusion --locked -- run --database /tmp/graphfusion-social \
  --query "USE GRAPH social MATCH (n:Person) RETURN n.name AS name ORDER BY name" --explain
cargo run -p graphfusion --locked -- checkpoint --database /tmp/graphfusion-social
```

`Session::replace_graph_data` writes immutable Parquet files in durable databases;
DataFusion scans those files directly. `Session::run(...).await` executes mixed
catalog/session/query programs. See [storage and CLI usage](docs/storage-cli.md)
for the import manifest, transaction boundaries, crash recovery, and limits.

## Catalog and Sessions

The main crate implements in-memory and persistent catalogs for directories,
schemas, graph types, and graphs. Independent sessions support catalog DDL and
`SESSION SET`, `SESSION RESET`, and `SESSION CLOSE`. Statements auto-commit unless
`START TRANSACTION` opens an [explicit transaction](docs/transactions.md).
READ ONLY/READ WRITE, COMMIT and ROLLBACK work across API calls. INSERT, SET,
REMOVE, DELETE and DETACH DELETE execute on open graphs through the async API.

```rust
use graphfusion::Database;

let db = Database::new();
let mut session = db.session();
session.execute("CREATE GRAPH social ANY GRAPH")?;
session.execute("SESSION SET GRAPH social")?;
session.execute("SESSION SET VALUE $limit INTEGER = 10")?;
```

Use `Database::open(path, OpenOptions { create_if_missing: true })` for a durable
database directory. Linux/macOS processes can share it through OS file locks.
Catalog and data snapshots publish together after a checksummed logical log is
durable. Optimistic validation permits independent writes while detecting name,
object, and dependency conflicts. DROP retires storage; checkpoint reclaims it
only when no active statement or transaction can use an older snapshot.

See [the catalog design and implementation notes](docs/catalog.md) for the
format, APIs, crash recovery, supported session expressions, and current limits.
The internal row store verifies joint catalog/data commits and is not a graph
storage engine.

## Layout

- `crates/graphfusion`: main database crate and public facade.
- `crates/gql-parser`: GQL lexer, AST, and parser. This crate should stay
  focused on turning GQL text into syntax trees; planning, execution, storage,
  and catalog state belong in other database crates or modules.

## Current Parser Scope

The GQL parser currently covers the core statement forms needed to bootstrap the
database front end:

- `USE GRAPH ... MATCH ... RETURN ...`
- regular identifiers with Unicode letters and digits, plus delimited
  identifiers including double-quoted and backtick forms with standard escapes
  and no-escape prefixes
- graph expressions for `CURRENT_GRAPH`, `CURRENT_PROPERTY_GRAPH`,
  `HOME_GRAPH`, `HOME_PROPERTY_GRAPH`, `VARIABLE <value expression>`,
  parenthesized value expressions, and reference parameters
- regular, extended, and delimited parameter names such as `$name`, `$123`,
  and `$"tenant id"`
- graph reference value expressions with `[PROPERTY] GRAPH <graph expression>`
- binding table reference value expressions with `[BINDING] TABLE <binding table expression>`,
  including nested-query, `VARIABLE <value expression>`, and parenthesized
  value-expression binding-table expressions
- `AT SCHEMA ...` query context with schema names, `CURRENT_SCHEMA`,
  `HOME_SCHEMA`, `.`, `/`, absolute and parent-relative directory references,
  and reference parameters
- catalog object references using either dot-separated names or standard
  solidus parent references such as `app/social`, explicit current-schema
  references such as `./social`, parent-relative references such as
  `../main/social`, and absolute references such as `/app/social`
- `MATCH`, `OPTIONAL MATCH`, and `OPTIONAL { MATCH ... }` graph patterns
- graph pattern match modes with `REPEATABLE ELEMENT(S)` and
  `DIFFERENT EDGE(S)`
- node and relationship patterns with labels and property maps
- path variable declarations such as `p = (a)-[:E]->(b)`
- path mode prefixes with `WALK`, `TRAIL`, `SIMPLE`, and `ACYCLIC`
- path search prefixes with `ALL`, `ANY`, `SHORTEST`, and counted shortest
  forms
- path pattern unions with `|`
- node and relationship pattern predicates with inline `WHERE`
- graph pattern `YIELD` clauses with explicit graph pattern variables
- `WHERE`, `FILTER WHERE`, `RETURN`/`SELECT` set quantifiers, aliases,
  `ORDER BY`, `OFFSET`/`SKIP`, and `LIMIT`
- `SELECT` result clauses, `FROM <graph> MATCH ...`, select-level `WHERE`,
  `GROUP BY`, and `HAVING`, including empty grouping set `GROUP BY ()` and
  `FROM { ... }` / `FROM <graph> { ... }` nested query specifications
- aggregate function set quantifiers and standard aggregate names such as
  `COUNT(*)`, `AVG`, `SUM`, `COLLECT_LIST`, `STDDEV_*`, and `PERCENTILE_*`
- `CAST(value AS type)` expressions
- `ELEMENT_ID(element)` functions
- value type predicates with `IS [NOT] TYPED ...`
- null predicates with `IS [NOT] NULL`
- truth value literals `TRUE`, `FALSE`, and `UNKNOWN`, plus predicates with
  `IS [NOT] TRUE/FALSE/UNKNOWN`
- `PROPERTY_EXISTS(element, property)` predicates
- `EXISTS { ... }` and `EXISTS (...)` predicates over graph patterns or nested
  queries, plus match statement blocks
- `SAME(element, element [, ...])` predicates
- `ALL_DIFFERENT(element, element [, ...])` predicates
- source/destination predicates with `IS [NOT] SOURCE OF` and
  `IS [NOT] DESTINATION OF`
- normalized predicates with `IS [NOT] [normal form] NORMALIZED`
- directed predicates with `IS [NOT] DIRECTED`
- labeled predicates with `IS [NOT] LABELED` and colon label syntax
- standard comparison and string operators including `<>` and `||`
- string value functions such as `LEFT(value, length)`,
  `RIGHT(value, length)`, `TRIM(...)`, and multi-character
  `BTRIM`/`LTRIM`/`RTRIM`
- string fold, normalization, and length functions with `UPPER(...)`,
  `LOWER(...)`, `NORMALIZE(value [, normal_form])`,
  `CHAR_LENGTH(...)`, `CHARACTER_LENGTH(...)`, `BYTE_LENGTH(...)`, and
  `OCTET_LENGTH(...)`
- numeric value functions such as `ABS(...)`, `MOD(dividend, divisor)`,
  `FLOOR(...)`, `CEIL(...)`/`CEILING(...)`, `SQRT(...)`,
  `POWER(base, exponent)`, `LOG(base, value)`, `LOG10(...)`, `LN(...)`,
  `EXP(...)`, trigonometric functions, and `PATH_LENGTH(...)`
- list value functions with `TRIM(list, count)` and `ELEMENTS(path)`
- list value constructors with optional type names such as `LIST [...]` and
  `ARRAY [...]`
- path value constructors with `PATH [node, edge, node, ...]`
- record value constructors with `[RECORD] {field: value, ...}`
- scalar value query expressions with `VALUE { ... }`
- let value expressions with `LET ... IN ... END`
- `ORDER BY` ordering synonyms `ASC`/`ASCENDING` and `DESC`/`DESCENDING`,
  plus null ordering with `NULLS FIRST` and `NULLS LAST`
- character string literals, including newline-separated literal chunks,
  standard backslash escapes, and single-quoted no-escape literals
- numeric literals with `_` digit separators, dot-leading/trailing decimals,
  `M`/`F`/`D` suffixes, and `0x`/`0o`/`0b` integer radices
- signed numeric expressions with unary `+` and `-`
- typed temporal literals: `DATE`, `TIME`, `DATETIME`, and `DURATION`,
  including SQL interval duration literals such as `INTERVAL '1' DAY` and
  range qualifiers such as `DAY TO SECOND`
- byte string literals such as `X'0Aff'`, with whitespace between hex digits
  and newline-separated literal chunks
- byte string `LEFT`/`RIGHT` substring and `TRIM` functions
- duration value functions with `DURATION(duration_string_or_record)` and
  `ABS(duration)`, plus `DURATION_BETWEEN(left_datetime, right_datetime)`
- datetime value functions such as `CURRENT_DATE`, `CURRENT_TIME`,
  `CURRENT_TIMESTAMP`, `LOCAL_TIME`, and `LOCAL_TIMESTAMP`, plus standard
  constructors such as
  `DATE(...)`, `ZONED_TIME(...)`, `ZONED_DATETIME(...)`, `LOCAL_TIME(...)`,
  and `LOCAL_DATETIME(...)` with string-literal or record-constructor
  parameters
- predefined value specifications such as `CURRENT_USER`
- searched and simple `CASE ... WHEN ... THEN ... ELSE ... END` expressions
- case abbreviations with `COALESCE(...)` and `NULLIF(...)`
- property references such as `n.name` and `record.field`
- linear query `FILTER`, `LET` with simple assignments or typed `VALUE`
  definitions, `FOR`, `[OPTIONAL] CALL ... YIELD`, and inline `CALL [(...)]
  { ... }` clauses, including `FOR ... WITH ORDINALITY/OFFSET`
- linear query `ORDER BY`/`OFFSET`/`SKIP`/`LIMIT` clauses before the result
  statement
- `FINISH` primitive result statements
- composite query conjunctions with `UNION`, `EXCEPT`, `INTERSECT`, and
  `OTHERWISE`
- procedure-body binding variable definition blocks with `GRAPH`, `[BINDING]
  TABLE`, and `VALUE` variable definitions
- procedure-body `NEXT [YIELD ...] <statement>` statement chains
- standalone named `CALL` procedure statements and procedure reference
  parameters such as `CALL $proc(...)`
- label expressions and path pattern quantifiers in graph patterns
- `INSERT`, including insert graph pattern variable restrictions, standard
  `SET` property/all-properties/label items, standard `REMOVE` property/label
  items, `DELETE`, `DETACH DELETE`, and `NODETACH DELETE`
- linear data-modifying query bodies that mix query clauses with `INSERT`,
  `SET`, `REMOVE`, and `DELETE` before an explicit result statement, or use
  the standard implicit `FINISH` result when omitted
- `CREATE GRAPH`, `DROP GRAPH`, `CREATE SCHEMA`, and `DROP SCHEMA`
- `CREATE [PROPERTY] GRAPH` with `OR REPLACE`, open graph types,
  graph type references including reference parameters, `LIKE <graph expression>`,
  nested graph type specifications, and `AS COPY OF <graph expression>` sources
- `CREATE [PROPERTY] GRAPH TYPE` and `DROP [PROPERTY] GRAPH TYPE` with
  nested node/edge type entries, `LIKE <graph expression>`, and
  `AS COPY OF <graph type reference>` sources including reference parameters,
  plus named, list, array, postfix list/array, max-length list/array, path,
  open and closed record, dynamic union, property value,
  graph/node/edge/binding-table reference, multi-word predefined, and
  character string, boolean, temporal, byte string, exact/approximate numeric,
  and `NOT NULL` property value types
- `START TRANSACTION`, `COMMIT`, and `ROLLBACK`, including transaction access
  mode
- session commands for `SESSION SET`, `SESSION RESET`, and `SESSION CLOSE`;
  `SESSION SET TIME ZONE <string value expression>`, `SESSION SET` graph,
  binding-table, and value parameter declarations, nested-query binding-table
  initializers, and standard `SESSION RESET` targets including `ALL PARAMETERS`,
  `ALL CHARACTERISTICS`, `TIME ZONE`, and
  parameter names are represented in the AST

The implementation follows the same broad shape as turso's parser: tokenization,
AST definitions, parser entrypoints, and tests are separated into focused
modules. GQL standard coverage should be expanded inside `crates/gql-parser`
without moving database responsibilities into that crate.

Parser tests follow turso's broad pattern: table-driven valid/invalid fixture
cases provide wide grammar coverage, while focused tests assert important AST
shape and error details.
