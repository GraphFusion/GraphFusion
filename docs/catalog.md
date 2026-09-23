# Catalog, sessions, and commit protocol

GraphFusion implements a persistent catalog and a session executor for catalog
DDL. It also coordinates [in-memory Arrow graph snapshots](graphs.md) for MATCH;
GQL graph data modifications remain unimplemented. The
internal storage participant stores versioned test rows to verify that catalog
and data changes share one transaction; it is not a graph storage engine.

## Public API

```rust
use graphfusion::Database;

fn main() -> graphfusion::Result<()> {
let db = Database::new(); // in-memory; parse_gql remains available
let mut session = db.session();
session.execute("CREATE GRAPH social ANY GRAPH")?;
session.execute("SESSION SET GRAPH social")?;
session.execute("SESSION SET VALUE $limit INTEGER = 10")?;
Ok(())
}
```

Use `Database::open(path, OpenOptions { create_if_missing: true })` to create or
open a database directory. The default options open an existing database.
Persistent mode supports Linux and macOS on local filesystems providing file
locks, atomic rename and fsync. Other processes must use the same protocol.
Network filesystems and reusing inherited database handles after `fork` are not
supported. The standard-library file locks require Rust 1.89; the workspace now
requires Rust 1.94 to integrate DataFusion 55.1.

`Database::with_catalog` inspects a statement snapshot. `create_directory` is an
administrative operation for provisioning directory paths. `checkpoint` attempts
a checkpoint, returning `Error::Busy` if a statement in any process pins a
snapshot. There is no automatic checkpoint scheduler yet; callers should retry
checkpointing at idle points. `statistics` reports commits, conflicts, lock wait,
recovery/checkpoint time, busy checkpoints, and log bytes. Counts are process local.

## Catalog and session semantics

Catalog entries have stable IDs, immutable definitions and a commit version.
Directories contain directories and schemas. Schemas contain graphs and graph
types. Bootstrap objects are root directory ID 1 and `/main` schema ID 2;
neither can be dropped. A name is indexed by container ID, object kind, and its
exact decoded identifier, including case. Separators inside delimited identifiers
are not interpreted as path components. Names in different object kinds have
independent namespaces.

Graph types bind node and edge definitions, labels, properties, nullability,
value types and endpoint constraints into engine-owned types. These definitions
are serialized directly; reopening does not depend on reparsing GQL. Graphs are
open, have an inline definition, or reference a named graph type. `LIKE` and
`AS COPY OF` graph-type operations copy definitions. Graph-type references are
tracked as dependencies. Nonempty schemas and referenced graph types cannot be
dropped or replaced. Graph replacement allocates a new graph and storage ID.

A session has current/home schema and graph, timezone, typed parameters and
closed state. Defaults are `/main`, no graph, UTC and no parameters. Session
state is private and not persisted. `SESSION SET` evaluates and validates into a
private copy, publishing only after successful execution. `SESSION RESET` and
`SESSION CLOSE` manage that state. Graph/schema parameters retain IDs and are
validated when used; deleting and recreating a name cannot rebind an old ID.

Supported initializers include literals, parameter references, graph references,
unary expressions, lists and records. Driver-provided binding tables can be
assigned through variable expressions. UTC and numeric timezone offsets are
supported; named timezone evaluation, general expression/query evaluation,
external graph-type imports and graph data copying return `UnsupportedFeature`.
Typed closed graph references currently require an exact bound definition match.

Top-level statements auto-commit individually. `NEXT` continuations and linear
catalog statements share one atomic statement transaction, including staged
session changes. A batch stops on error; earlier successful top-level statements
remain committed. Explicit transaction controls are rejected before executing any
top-level effects. The parser still accepts transaction syntax. This runtime
scope is not a claim that GQL lacks explicit transactions.

## Snapshots and optimistic validation

Each statement owns a lifecycle lease and an `Arc<PublishedState>` containing:

- one commit sequence;
- the immutable catalog root;
- the immutable storage root;
- the ID reservation high watermark.

Catalog changes use copy-on-write mappings and shared immutable entries. Writes
and read dependencies remain private until commit. Lookup checks the private
catalog view first, so a compound statement sees its own DDL.

The read set tracks object versions, name bindings (including absent-name
versions), directory membership scans, and reverse dependency scans. Commit
compares those dependencies against the newest committed state. It then applies
only the transaction's delta to that state, preserving disjoint concurrent
changes. Membership changes do not change the container object's own version, so
two independent creates in the same schema can both commit.

The storage participant checks write-write conflicts on individual row keys and
retains tombstones. Data reads use the starting storage root, while writes also
record the graph definition dependency. Thus data uses snapshot isolation while
catalog dependencies are validated for serializability; the entire database is
not claimed to be serializable.

Read-only statements finish against their starting snapshot. Session handles do
not pin snapshots between calls. Results are currently materialized DDL results;
no externally held data cursor can prolong a lease.

## Multi-process coordination

Coordinating files are stable inodes and must not be deleted or replaced while
the database is open. Each lease uses its own file descriptor, avoiding accidental
unlock through a shared descriptor. Handles to the same canonical database path
also share a process-local coordinator.

The lock order is:

1. Lifecycle shared lock for statements, exclusive lock for checkpoints.
2. Process-local state mutex.
3. Cross-process commit lock.

Opening and statement start refresh from durable files under the commit lock.
Statement execution releases that lock. Commit acquires it again, refreshes,
validates, persists, and publishes. A writer process can die at any point; OS
locks release when its descriptors close. No shared-memory pointer or process
local reference count is used as authority for another process's readers.

## Durable format and recovery

`MANIFEST` identifies the database, format, checkpoint generation, commit and ID
watermarks, and obsolete file generations. It names corresponding
`catalog-N.snapshot`, `data-N.snapshot`, and `wal-N.log` files. Snapshots and the
WAL header repeat identity and generation information to reject mixed files.
`INIT` identifies incomplete first-time creation; missing manifest without that
marker fails closed rather than initializing over existing files.

Documents and log records use JSON payloads inside binary frames with magic,
length, header CRC32, payload CRC32 and a final marker. The frame protects length
independently so length corruption cannot silently discard later committed data.
Each document/frame currently has a 64 MiB limit, including each complete
checkpoint snapshot. This format is intended for the initial catalog and test
storage participant, not large graph datasets.

A commit frame contains catalog deltas and storage deltas together. Object IDs
are reserved in durable blocks before exposure, permitting gaps but preventing
reuse across aborts and restarts. Reservations do not advance the commit sequence.

The commit path constructs and validates its candidate state before appending.
It writes the complete frame, fsyncs the WAL, and installs a single published
state. There is no separate catalog commit and storage commit. A complete frame
left by an interrupted writer is fsynced by recovery before any process exposes
it. An incomplete tail is removed; a complete corrupt record fails closed.
An I/O error after append begins returns `CommitUnknown`, poisons that database
coordinator, and requires reopening to resolve the outcome. Blind retries are
not automatic.

For now, refresh reloads the checkpoint and replays the whole current WAL. This
is deliberately a correctness-first implementation with O(snapshot + WAL) refresh
cost, not an incremental WAL index. Copy-on-write still copies affected maps.
Incremental replay and persistent map structures can replace these internals
without changing the visibility protocol.

## DROP, checkpoint, and physical reclamation

`DROP GRAPH` publishes removal of the catalog object and retirement of its
entire storage generation in one commit. It never immediately deletes data.
Old readers retain the old catalog and storage roots. A concurrent writer using
the dropped graph fails catalog validation if DROP commits first. If the writer
commits first, DROP retires the whole latest generation, including those writes.
Same-name recreation uses different identities.

Checkpoint obtains an exclusive lifecycle lock, proving there are no active
statements in any process. It refreshes the latest committed state, removes
retired generations and obsolete tombstones, then:

1. Writes catalog and data snapshots for the same commit sequence and a new WAL.
2. Fsyncs all files and their directory entries.
3. Writes and fsyncs a temporary manifest, renames it, and fsyncs the directory.
4. Deletes obsolete snapshot/log generations and fsyncs the directory.

The manifest records obsolete generations until later checkpoints confirm that
cleanup completed. Unlink is idempotent and failures are retried. A crash can
leave unnecessary files; it cannot justify falling back to an old manifest that
references already reclaimed data. Checkpoint never truncates the old WAL before
the replacement manifest is durable. Retired rows cannot fall through to a stale
checkpoint because catalog and data snapshot generations move together.

A future physical graph engine must generate its data changes through this
coordinator. If logs reference external newly created files, those files and
their directory entries must be durable before committing the references.
Removing properties or indexes must retire their resources using the same
snapshot and checkpoint gates; it must not mutate old readers' layouts in place.

## Verification

Unit tests include independent threads and OS processes, barriers around
concurrent commits, namespace/dependency phantoms, old readers across DROP,
row tombstone recovery, ID reservations, same-name replacement, and session
isolation. Child-process failure injection terminates without destructors during
initialization, WAL append, publication, checkpoint installation, and file
cleanup. Injected I/O failure checks unknown commit recovery and coordinator
poisoning. These simulate process crashes and I/O errors, not actual power loss
or every filesystem's persistence implementation.

Run:

```sh
cargo test --workspace --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
```

The catalog PR builds on the CI foundation, including workspace-wide strict
Clippy and Linux/macOS tests.
