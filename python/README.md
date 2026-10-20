# GraphFusion for Python

An embedded graph database powered by Rust, Apache DataFusion, and Apache Arrow.
Use GQL to create graphs, insert nodes and edges, traverse relationships, and
update or delete data. No database server is required.

Python 3.10+ is required. Persistent storage supports Linux and macOS on local
filesystems with working file locks and atomic rename. Windows persistence and
inheriting database/session handles across `fork` are unsupported.

## Install

Once published to PyPI:

```sh
pip install graphfusion
# During development releases:
pip install --pre graphfusion
# Optional Pandas conversion:
pip install 'graphfusion[pandas]'
```

The package includes a compiled Rust extension and requires PyArrow. Installation
from source requires the Rust toolchain pinned in `rust-toolchain.toml`, a C
compiler/linker, and maturin. Binary wheels do not require Rust.

## Graph operations

```python
from graphfusion import Database

with Database() as db:  # in-memory; use Database("my-db", create=True) to persist
    db.run("""
        CREATE GRAPH social ANY GRAPH;
        SESSION SET GRAPH social;
        INSERT (:Person {name: 'Alice'})-[:Knows]->(:Person {name: 'Bob'});
    """)
    result = db.query("""
        MATCH (a:Person)-[:Knows]->(b:Person)
        RETURN a.name AS source, b.name AS target
    """)
    assert result.to_pylist() == [{"source": "Alice", "target": "Bob"}]
    table = result.to_arrow()  # typed pyarrow.Table
    rows = result.fetchall()  # [("Alice", "Bob")]
    # frame = result.to_pandas()  # requires pandas
```

`run()` accepts scripts and returns a `Result` per statement, including catalog
commands and mutations. `query()` accepts one read-only query. Outside an explicit
transaction, each statement commits independently; a later failure does not undo
prior statements. Results are fully materialized in memory. Arrow IPC preserves
64-bit integers, nulls, binary and nested values without JSON conversion. Arrow
conversion copies serialized buffers; this is not a zero-copy API. Graph element
and path values retain the engine's Arrow struct representation. Prefer explicit
property projections such as `RETURN p.name AS name` for portable row data. Use
unique aliases with `to_pylist()`; `fetchall()` preserves positional columns.

## Parameters, independent sessions and transactions

```python
with Database() as db:
    db.run("CREATE GRAPH social ANY GRAPH")
    with db.session() as session:
        session.run("SESSION SET GRAPH social")
        session.set_parameter("name", "O'Reilly")  # never interpolate user input
        with session.transaction():
            session.run("INSERT (:Person {name: $name})")
        assert session.query(
            "MATCH (p:Person) WHERE p.name = $name RETURN p.name AS name"
        ).fetchall() == [("O'Reilly",)]
```

Parameters persist in their session and support `None`, `bool`, signed 64-bit
`int`, `float`, `str`, `bytes`, lists/tuples, and dictionaries with string keys,
subject to GQL engine type support. The current query engine cannot project
record parameters (`RETURN $record`); this raises `GraphFusionError`. Scalar
and list parameters are supported in queries. `set_query_limits()` controls maximum path
hops and the per-statement tracked operator memory budget (not total process RSS).

A transaction context commits on success and attempts rollback on exceptions.
Transactions cannot be nested. Results marked `transaction_pending` are
provisional until commit succeeds. Engine failures raise `GraphFusionError`;
invalid Python argument types raise the usual Python exceptions. If a commit
reports an unknown outcome, close all handles and reopen before deciding whether
to retry; rollback cannot establish whether that commit was durable.

Each session has independent graph selection, parameters, and transaction state.
The database's `run()`/`query()` methods use one retained convenience session.
Rust execution releases the GIL. Separate sessions may run from Python threads;
calls to the same session are serialized. Do not share an active transaction
across threads: serialization applies per call, not to a whole context block.
There is no asyncio API. Close explicit
sessions before closing/reopening persistent databases: each session owns a
handle, and closing its parent database does not invalidate it. Closing a session
rolls back unfinished work; context managers release handles deterministically.

## Build and test locally

From the repository root, using the existing checkout:

```sh
python3 -m venv .venv
. .venv/bin/activate
pip install 'maturin>=1.9,<2' pytest pyarrow
maturin develop --locked
python -m pytest python/tests
maturin build --release --locked --out dist
maturin sdist --out dist
```

The extension uses CPython's stable ABI (3.10+) and ships as `graphfusion._native`.
The distribution version is defined in `pyproject.toml`; keep it synchronized with
`graphfusion.__version__` and Rust release versions. The initial Python version
is `0.3.0.dev0`, a prerelease. Platform wheels must be built and tested on each
supported platform; Linux release wheels need manylinux/musllinux compatibility
checks. A wheel built directly on a development Linux host is only a local
validation artifact, not a portable release wheel.

## Prepare a PyPI release

The Python workflow builds and tests Linux/macOS wheels and an sdist, and uploads
artifacts without publishing them. Before release, confirm ownership/availability
of the `graphfusion` name on PyPI and TestPyPI, choose the release version, and
inspect metadata and supported wheel platforms. Test source and wheel installs.
Do not overwrite an existing published version.

Download the workflow artifacts into a single directory. Check them with
`python -m twine check dist/*`, then upload to TestPyPI first using trusted
publishing or credentials configured securely outside the repository. Validate
installation in a clean environment with PyArrow from PyPI. Only after review,
upload the same artifacts to PyPI:

```sh
python -m twine upload --repository testpypi dist/*
# After validating the TestPyPI install and reviewing the same artifacts:
python -m twine upload dist/*
```

These commands require publisher credentials configured securely in your release
environment. No upload runs automatically in this project.
