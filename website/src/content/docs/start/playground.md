---
title: "Browser playground"
description: "Run GraphFusion locally in your browser and explore query results as a graph."
sidebar:
  order: 2
---

[Open the Playground](/playground/) to try GraphFusion without installing Rust or starting a database server. The real GQL parser, graph engine and DataFusion executor run as WebAssembly in a browser worker. After the engine loads, queries and data stay in this tab; querying does not contact a remote service.

The sample graph connects people, projects and organizations. Choose an example, edit it, and press **Run query** or **Ctrl/⌘ + Enter**.

## Explore a result

**Graph** displays returned nodes, relationships and paths. Drag nodes, scroll to zoom, fit or rearrange the graph, and search within the result. Click an element or use the inspector dropdown to see its labels and properties. Returned relationships include their endpoints; returning only nodes does not add relationships you did not request.

```gql parse
MATCH (a)-[relationship]->(b)
RETURN a, relationship, b
LIMIT 100
```

**Table** displays the returned columns, including scalars and nested path/list values. **Plan** shows the logical and physical plans. **Export JSON** downloads the selected statement's result with its query. A program with several statements has a result selector.

## Change the graph

`INSERT`, `SET`, `REMOVE` and `DELETE` execute against the same local database. Explicit transactions support `START TRANSACTION`, `COMMIT` and `ROLLBACK`; pending results are marked. Top-level statements otherwise commit separately, so earlier successful statements can remain applied if a later statement fails.

**Reset sample data** replaces the current database with the original fixture. **Stop & reset** terminates the running worker, discards the session and starts a fresh sample database.

## Current limits

- This version uses memory. Reloading, closing the tab, resetting or stopping a query clears database changes. Recent query text is kept in local browser storage (up to ten queries).
- The first visit downloads the engine. Queries and normal sample resets work offline after initialization. Full page reloads and worker restarts still need the static application assets.
- Results display at most 1,000 rows and 500 returned graph elements, with relationship endpoints added. Use `LIMIT` for larger datasets; these display caps do not bound query execution.
- Browser statements have a tracked operator memory budget of 128 MiB and a maximum finite path bound of 16 hops. This is not a limit on total browser memory.
- Local Parquet directories, OPFS persistence and file imports are not exposed in this version. See the [support matrix](/start/status/) for the GQL subset.

Modern browsers with WebAssembly and module workers are required. Desktop and mobile Chromium are covered by the browser tests.
