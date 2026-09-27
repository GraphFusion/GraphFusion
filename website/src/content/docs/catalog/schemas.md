---
title: "CREATE and DROP SCHEMA"
description: "Organize graph objects in schema namespaces."
sidebar:
  order: 3
---

CREATE SCHEMA adds a schema under an existing catalog directory. It does not create a filesystem directory.

```gql test=standalone
CREATE SCHEMA app;
SESSION SET SCHEMA app;
CREATE GRAPH social ANY GRAPH;
DROP GRAPH social;
SESSION SET SCHEMA /main;
DROP SCHEMA app;
```

A schema must be empty before it can be dropped. The bootstrap `/main` schema cannot be dropped. Current/home schema settings belong to a session; other sessions retain their own context.

Creating deeper directory hierarchies uses the administrative `Database::create_directory` API before creating the schema. See [catalog objects](/GraphFusion/catalog/objects/).
