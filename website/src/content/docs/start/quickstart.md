---
title: "Your first graph"
description: "Create a graph, insert relationships and query friends of friends."
sidebar:
  order: 3
---

This example creates Alice → Bob → Cara, then finds Alice's friend of a friend. Run it from a source checkout after [building the CLI](/start/installation/).

## Start an interactive session

From the 0.2.0 source checkout:

```sh
cargo run -p graphfusion --locked -- repl --database ./demo-db --create
```

At the prompt, load the bundled example once with `\read examples/social.gql`. Then enter `MATCH (p:Person) RETURN p.name AS name, p.age AS age;`. End each GQL input with a semicolon; use `\help` for commands and `\quit` to exit. Reopen with the same command and enter `SESSION SET GRAPH social;` to select the existing graph. Do not reload the create script into a database that already contains it.

The steps below demonstrate the separate one-shot `run` command. Use a different database directory if you already created `demo-db` in the REPL, and close the REPL before opening its database from another process.

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

This returns Alice and 31. [INSERT](/mutations/insert/), [SET](/mutations/set/) and [DELETE](/mutations/delete/) explain how statements publish changes.

## Continue

- [MATCH](/patterns/match/): find connected elements.
- [Shortest paths](/patterns/selectors/): search routes.
- [Transactions](/storage/transactions/): commit several statements together.
- [Rust API](/rust/embedding/): consume Arrow batches in an application.
