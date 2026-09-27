---
title: "Catalog objects"
description: "Understand directories, schemas, graph types and graphs."
sidebar:
  order: 1
---

The catalog names and versions database objects. It is separate from the labels and properties inside a graph.

```text
/                         directory
└── main                  schema
    ├── social            graph
    └── PersonGraph       graph type (optional definition)
```

Directories contain directories and schemas. Schemas contain graphs and graph types. The bootstrap root directory and `/main` schema always exist and cannot be dropped. `Database::create_directory` provisions directory paths; there is no GQL CREATE DIRECTORY command.

Object names are indexed by container, kind and exact decoded spelling. A graph and graph type can occupy separate kind namespaces. Stable object IDs distinguish a live object from another later created with the same name.

Graph types hold node/edge definitions and dependencies. A graph can be open, have an inline type or reference a named graph type. Data operations currently support open graphs; typed definitions can be stored but do not yet enable typed graph scans or writes.

See [paths](/GraphFusion/catalog/paths/), [schemas](/GraphFusion/catalog/schemas/) and [graphs](/GraphFusion/catalog/graphs/).
