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

[MemTable thresholds](/storage/persistence/) default to 65,536 resident rows or 16 MiB of Arrow array buffers per graph. Sealing happens synchronously at commit. Limits do not bound private transaction memory or historical snapshots; multiple graphs each have their own budget. WAL retains committed deltas until a successful checkpoint. Resident updates log removals and replacement batches; append-only writes record only new batches. Target matching and refreshing result bindings can still scan/filter existing data. Resident ID/adjacency indexes and deletion bitmaps are outside the DataFusion operator memory budget.

- A label pattern reads only fragments whose label set can match. An unlabeled pattern still unions every fragment of that element kind. Integers and floats on the selected fragments widen to float. Other mixed scalar types are returned as strings. Mixed binary values are rejected. One label set keeps a single stored type for each property: an insert or `SET` that would change it is rejected, and refreshing a bound element does not adopt another type's schema. A one-label-apart fragment is still scanned so a label edit can find the moved row; its extra property names and types stay off the binding. A reference that spans element kinds still rejects a non-numeric clash on one property name. A `LET` copy of a label-constrained element reads that label set, so its stored property types stay intact; an unlabeled element reference still unions every fragment of the kind. Filters compare each fragment in its stored type before that unification, including `AND`, `OR`, `XOR`, and `NOT` with a closed boolean, and the same comparison in `FILTER` or `LET`, including comparisons against a working-table value. `IS NULL` and `IS NOT NULL` stay with the stored-type comparison. `PROPERTY_EXISTS` stays with the stored-type comparison. `IS LABELED` stays with the stored-type comparison. `IS DIRECTED` stays with the stored-type comparison. Same-element `SAME` and `ALL_DIFFERENT` stay with the stored-type comparison. `ELEMENT_ID` stays with the stored-type comparison. Cross-element `AND`, `OR` and `XOR` combine stored-type comparison results after the elements are joined. A fragment whose type cannot be compared is unknown and does not match. A parenthesized path predicate does the same when it compares a property with a literal. `IS [NOT] TRUE`, `IS [NOT] FALSE` and `IS [NOT] UNKNOWN` classify that per-fragment result, so an integer is not true against the same digits stored as text. `COALESCE` and `NULLIF` of those comparisons, `NULLIF(property, value) IS [NOT] NULL`, and a comparison of `COALESCE(property, fallback)` do the same. A comparison in `RETURN`, `SELECT`, `SELECT WHERE`, `HAVING`, or `ORDER BY` uses those stored types as well. Comparisons between two properties also use the participating fragments’ stored types.
- Inserts append to resident fragments without reading previously sealed files. Updates gather indexed target rows; deletes mask row positions and use adjacency indexes for incident edges. They preserve sealed files and unaffected resident buffers. Runtime index loading has a one-time cost per immutable file.
- Results are materialized; no streaming/prepared query API is exposed.
- Metadata records have a 64 MiB limit; unsupported definition nesting is rejected before publication.
- Persistent startup replays committed WAL records and validates referenced files once. Subsequent statements share the published snapshot; failed checkpoint/publication requires recovery.
- Typed graph data operations, property/range indexes, background sealing, automatic checkpoint scheduling, deletion-mask compaction and format migrations are not implemented.

The [support matrix](/start/status/) separates these execution limits from parser coverage.

Arithmetic and concatenation on mixed properties use each fragment's stored type. Incompatible operators contribute NULL before classification or coalescing, while label, identity and property-existence metadata retain their original bindings.

`RETURN`, `LET`, and `ORDER BY` evaluate arithmetic and string concatenation on stored types. A fragment that cannot apply the operator yields null; the expression is a type error when no selected fragment can apply it.
