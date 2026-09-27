---
title: "What is GraphFusion?"
description: "GQL graph queries on Apache DataFusion, embedded in a Rust application."
sidebar:
  order: 1
---

GraphFusion is an **embedded graph database written in Rust**. It connects GQL's property-graph model to Apache DataFusion's query engine and Arrow's columnar data model. Use it from Rust or run GQL programs with the command-line tool.

A graph contains labeled nodes and edges with properties. You write patterns such as `(person)-[:Knows]->(friend)`; GraphFusion builds DataFusion plans that scan, join, filter and aggregate the underlying tables. It does not translate GQL into SQL.

## The execution model

| Layer | Responsibility |
| --- | --- |
| GQL | Express graph patterns, paths, queries and updates |
| GraphFusion | Bind names and types; coordinate sessions, catalog and transactions |
| Apache DataFusion | Optimize and execute plans, including joins and recursive paths |
| Apache Arrow | Represent graph tables and query results in memory |
| Apache Parquet | Store immutable graph tables in durable databases |

## What you can do today

- Create open graphs and insert, update or delete nodes and edges using GQL.
- Match fixed and quantified paths, find shortest paths, and work with whole-element values.
- Combine graph results with optional matches, lists, aggregates and set operations.
- Run isolated sessions and explicit transactions, in memory or in a local database directory.
- Import Arrow/Parquet graph tables and consume Arrow result batches from Rust.

GraphFusion is under active development. The parser recognizes more of GQL than the execution engine supports. Pages marked **Syntax only** describe accepted grammar, not runnable database features. [Supported features](/GraphFusion/start/status/) lists the boundaries. Full ISO GQL conformance is not claimed.

## When to use it

Start here if you want to experiment with GQL over columnar graph data, embed graph queries in a Rust application, or contribute to a DataFusion-based graph engine. Expect materialized results, graph-wide rewrites for updates and explicit resource limits. There is no network server, distributed execution service or stable on-disk migration path yet.

[Run your first graph query →](/GraphFusion/start/quickstart/)
