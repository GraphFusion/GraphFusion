---
title: "Catalog paths"
description: "Resolve absolute, schema-relative and directory-relative names."
sidebar:
  order: 2
---

Catalog references support dot-separated names and solidus paths. The current schema supplies context for an unqualified graph name.

| Reference | Intent |
| --- | --- |
| `social` | Graph in the current schema |
| `main.social` or `main/social` | Schema-qualified graph reference |
| `./social` | Explicit current-schema graph |
| `../main/social` | Resolve through the parent directory |
| `/main/social` | Absolute graph path |

```gql test
USE GRAPH /main/social MATCH (p:Person) RETURN COUNT(*) AS people;
```

Schema contexts accept CURRENT_SCHEMA, HOME_SCHEMA and directory references such as `.` and `/`, but the resolved target must have the required kind. Directories are catalog containers, not operating-system directories.

A slash or dot inside a delimited identifier remains part of that one name. Decoding is separate from component splitting. Graph/schema parameters carry stable IDs; dropping and recreating a named object does not rebind an old reference.
