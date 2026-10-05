# Mutation storage and benchmark

Resident storage retains immutable Arrow batches, a sharded copy-on-write ID lookup,
per-batch edge adjacency, and snapshot-specific deletion bitmaps. Updates gather
target rows, mask their old versions and append replacements. Deletes mask positions;
publishing a durable layout prunes fully deleted resident batches. Surviving property
buffers remain shared with earlier snapshots.

New immutable Parquet files have checksummed ID-sorted `.index` sidecars. Node entries
contain `(ID, original file row)`; edge entries also contain both endpoints. The runtime
uses binary search for ID lookup and builds adjacency lazily when deleting nodes.
Each snapshot retains its own bitmap, which Parquet readers apply through physical
row selection. WAL records still store logical deletion IDs, preserving the v4 wire
format. Existing v4 files without sidecars read only ID/endpoint columns to build the
runtime lookup on first use.

These are mutation indexes. MATCH predicates and refreshed result bindings still
use DataFusion plans and can scan columns or join endpoints. Sidecar loading and the
first adjacency build cost time and memory. Resident masks avoid copying survivor
columns while applying a mutation; reads filter only their projected columns. Existing
Parquet files and deletion masks are not compacted by checkpoint.

## Reproducing the measurements

```sh
cargo run -p graphfusion --example mutation_bench --locked --offline -- 100000 3
# Select one storage mode and operation:
cargo run -p graphfusion --example mutation_bench --locked --offline -- 100000 3 parquet delete_edge
# Larger DETACH batches:
cargo run -p graphfusion --example mutation_bench --locked --offline -- 100000 3 memory detach_1000
```

The fixture contains 100,000 nodes, 99,998 directed chain edges and a final isolated
node. Each node and edge carries an integer and a 128-byte string. Each sample creates
a fresh graph; imports, checkpoints and count-query warmups are outside the timed
region. Durability modes include WAL commit and synchronization in their timings.
Parquet ID and adjacency caches are cold at the first mutation, so index loading is
included. Temporary database directories are removed when the benchmark exits.

The comparison below uses debug builds, four Tokio workers, warm data/filesystem
caches and the median of three samples on the same machine. The baseline used an
equivalent temporary harness before the storage changes; updated results use the
checked-in example. These are local comparisons, not release-build latency targets.

All values are milliseconds, **before → after**. `resident` is a durable database with unsealed Arrow batches; `memory` uses `Database::new()`.

| Operation | memory | resident | Parquet |
| --- | ---: | ---: | ---: |
| Update one node | 271.4 → 34.1 | 278.7 → 35.5 | 362.0 → 103.0 |
| Update one edge | 296.7 → 78.4 | 329.9 → 89.8 | 510.7 → 292.1 |
| Delete one edge | 377.4 → 62.1 | 535.2 → 65.5 | 682.3 → 241.9 |
| Delete one isolated node | 356.1 → 13.7 | 458.8 → 18.3 | 550.2 → 205.7 |
| DETACH DELETE one node and its edge | 242.2 → 13.8 | 410.0 → 18.8 | 474.5 → 206.2 |
| DELETE with no match | 322.4 → 10.4 | 434.4 → 10.3 | 523.4 → 13.6 |

The updated cases contain no `NestedLoopJoinExec` in the recorded deletion plans. No-match DELETE publishes no change and writes zero WAL bytes. Parquet deletion deltas in this fixture are 259–284 bytes for node/edge/DETACH operations; resident deletions also record fragment references and remain below 1.3 KiB.

Parquet results were repeated after the final lazy-adjacency change with verification tasks finished. Memory/resident results come from the preceding complete matrix; that final change affects only immutable-file index loading.

## Validation

The workspace suite passed 799 tests. After deferring file adjacency construction,
all 76 library tests and 12 mutation integration tests passed again. Clippy with
warnings denied, formatting and whitespace checks passed.

Regressions cover shared Arrow buffers, old snapshots, failed-statement rollback,
WAL recovery, sealing without resurrecting masked rows, legacy v4 lookup fallback,
multiple Parquet row groups, projections, nulls, counts, filters and limits. Crash
matrices now include index write/sync, and sidecar corruption and branch retention
are checked. Graph cases include parallel edges, self-loops, both directions and
overlapping node/edge ID spaces.
