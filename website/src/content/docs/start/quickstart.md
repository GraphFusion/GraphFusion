---
title: "Your first graph"
description: "Create a graph, insert relationships and query friends of friends."
sidebar:
  order: 3
---

This example creates Alice → Bob → Cara, then finds Alice's friend of a friend. Run it from a source checkout after [building the CLI](/GraphFusion/start/installation/).

## Create and query a graph

Save the following as `first-graph.gql`:

```gql test=standalone
CREATE GRAPH social ANY GRAPH;
SESSION SET GRAPH social;

INSERT (alice:Person {name: 'Alice', age: 30})-[:Knows]->
       (bob:Person {name: 'Bob', age: 40})-[:Knows]->
       (cara:Person {name: 'Cara', age: 25});

MATCH (a:Person {name: 'Alice'})-[:Knows]->(b)-[:Knows]->(c)
RETURN c.name AS friend;
```

```sh
cargo run -p graphfusion --locked -- run --file first-graph.gql
```

The final table contains `friend = Cara`. The program runs in memory and its database disappears when the process exits. `ANY GRAPH` creates an open graph: you can introduce labels and properties without declaring a graph type first.

## Keep data between runs

Use a new database directory:

```sh
cargo run -p graphfusion --locked -- run \
  --database ./demo-db --create --file first-graph.gql
cargo run -p graphfusion --locked -- run --database ./demo-db \
  --query "USE GRAPH social MATCH (p:Person) RETURN p.name AS name ORDER BY name"
```

The second process reopens the graph and returns Alice, Bob and Cara. Session settings do not persist between processes, so the query selects `social` again with `USE GRAPH`.

Do not rerun the create program against the same directory unchanged: `CREATE GRAPH social` will find an existing graph. Reopen the directory to query it, or use a different directory to repeat the tutorial.

## Change a property

```gql test
MATCH (p:Person {name: 'Alice'})
SET p.age = p.age + 1
RETURN p.name AS name, p.age AS age;
```

This returns Alice and 31. [INSERT](/GraphFusion/mutations/insert/), [SET](/GraphFusion/mutations/set/) and [DELETE](/GraphFusion/mutations/delete/) explain how statements publish changes.

## Continue

- [MATCH](/GraphFusion/patterns/match/): find connected elements.
- [Shortest paths](/GraphFusion/patterns/selectors/): search routes.
- [Transactions](/GraphFusion/storage/transactions/): commit several statements together.
- [Rust API](/GraphFusion/rust/embedding/): consume Arrow batches in an application.
