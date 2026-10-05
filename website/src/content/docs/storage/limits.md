---
title: "Resource limits and current boundaries"
description: "Plan bounded path searches and understand memory and write costs."
sidebar:
  order: 6
---

Sessions default to **256 path hops** and **256 MiB of DataFusion tracked operator memory per statement**. Configure subsequent statements with `Session::set_query_limits`.

```rust
use graphfusion::QueryLimits;
session.set_query_limits(QueryLimits {
    max_path_hops: 64,
    memory_limit_bytes: 128 * 1024 * 1024,
})?;
```

The memory budget is not a process RSS cap. It does not account for every imported graph buffer, optional/input barrier or final materialized result. Resource exhaustion is an error, following normal statement/transaction rollback behavior.

## Path completeness

A finite path bound above the hop limit fails during planning. The planner does not substitute the configured limit and return a silently truncated answer. It combines bounds across concatenation, alternatives, repetition and whole-path modes.

Some unbounded forms obtain complete finite bounds from graph cardinality or restricted selective-WALK proofs. General history-dependent or compositional repeatable unbounded patterns are unsupported. Use a finite upper bound when those proofs do not apply.

Candidate enumeration currently happens before path ranking. Dense graphs, parallel edges and repeated groups can produce many candidates even for a shortest-path request. RETURN LIMIT caps output, not enumeration work.

## Storage and API

[MemTable thresholds](/storage/persistence/) default to 65,536 resident rows or 16 MiB of Arrow array buffers per graph. Sealing happens synchronously at commit. Limits do not bound private transaction memory or historical snapshots; multiple graphs each have their own budget. WAL retains committed deltas until a successful checkpoint. Resident updates log removals and replacement batches; append-only writes record only new batches. Mutation plans can still scan/filter existing data in memory.

- Inserts append to resident fragments without reading previously sealed files. Updates and deletes currently materialize the affected graph to evaluate their plans, but preserve sealed files using deletion masks and resident replacement rows.
- Results are materialized; no streaming/prepared query API is exposed.
- Metadata records have a 64 MiB limit; unsupported definition nesting is rejected before publication.
- Persistent startup replays committed WAL records and validates referenced files once. Subsequent statements share the published snapshot; failed checkpoint/publication requires recovery.
- Typed graph data operations, indexes, background sealing, automatic checkpoint scheduling, deletion-mask compaction and format migrations are not implemented.

The [support matrix](/start/status/) separates these execution limits from parser coverage.
