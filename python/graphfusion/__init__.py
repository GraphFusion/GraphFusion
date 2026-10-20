"""Embedded GraphFusion databases with GQL queries and Arrow results."""
from __future__ import annotations

from contextlib import contextmanager
from dataclasses import dataclass
from os import PathLike, fspath
from typing import Any, Iterator

import pyarrow as pa

from . import _native
from ._native import GraphFusionError

__all__ = ["Database", "Session", "Result", "GraphFusionError"]
__version__ = "0.3.0.dev0"


@dataclass(frozen=True)
class Result:
    """A materialized statement result; command results have no Arrow table.

    ``affected`` counts catalog objects for commands and graph elements for
    mutations. Results with ``transaction_pending`` are provisional until commit.
    """

    kind: str
    table: pa.Table | None
    commit_seq: int
    affected: int
    transaction_pending: bool

    @classmethod
    def _from_native(cls, result: Any) -> Result:
        ipc = result.arrow_ipc()
        table = None if ipc is None else pa.ipc.open_stream(ipc).read_all()
        return cls(result.kind, table, result.commit_seq, result.affected,
                   result.transaction_pending)

    @property
    def columns(self) -> list[str]:
        return [] if self.table is None else self.table.column_names

    def to_arrow(self) -> pa.Table:
        """Return the typed Arrow table (queries only)."""
        if self.table is None:
            raise ValueError("command results have no table")
        return self.table

    def to_pylist(self) -> list[dict[str, Any]]:
        """Return rows as dictionaries; select unique column aliases."""
        return self.to_arrow().to_pylist()

    def fetchall(self) -> list[tuple[Any, ...]]:
        """Return positional rows, preserving duplicate column names."""
        return list(zip(*(column.to_pylist() for column in self.to_arrow().columns)))

    def to_pandas(self, **kwargs: Any) -> Any:
        """Convert using PyArrow; requires the ``pandas`` optional dependency."""
        return self.to_arrow().to_pandas(**kwargs)

    def __len__(self) -> int:
        return 0 if self.table is None else self.table.num_rows


class Session:
    """Independent graph selection, parameters and transaction state.

    Calls on a session are serialized; independent sessions can execute on
    separate Python threads. Async Python APIs and forked handles are unsupported.
    """

    def __init__(self, database: Database) -> None:
        self._native = database._native.session()

    def run(self, gql: str) -> list[Result]:
        """Execute a GQL script, returning one result for each statement.

        Outside explicit transactions, earlier successful statements remain
        committed if a later statement fails. Use ``transaction()`` for atomicity.
        """
        return [Result._from_native(r) for r in self._native.run(gql)]

    def query(self, gql: str) -> Result:
        """Execute exactly one read-only GQL query."""
        return Result._from_native(self._native.query(gql))

    def set_parameter(self, name: str, value: Any) -> None:
        """Bind ``$name`` without interpolating GQL. Binding persists in this session.

        Supports None, bool, signed 64-bit int, float, str, bytes, lists, tuples,
        and dictionaries with string keys. Engine typing rules still apply.
        """
        self._native.set_parameter(name, value)

    def set_query_limits(self, *, max_path_hops: int = 256,
                         memory_limit_bytes: int = 256 * 1024 * 1024) -> None:
        self._native.set_query_limits(max_path_hops, memory_limit_bytes)

    @contextmanager
    def transaction(self) -> Iterator[Session]:
        """Commit on normal exit; roll back on exceptions. Cannot be nested."""
        self.run("START TRANSACTION")
        try:
            yield self
            self.run("COMMIT")
        except BaseException:
            try:
                self.run("ROLLBACK")
            except (GraphFusionError, RuntimeError):
                pass  # Preserve the original failure, including commit-unknown.
            raise

    def close(self) -> None:
        """Release this session and discard any uncommitted transaction."""
        self._native.close()

    def __enter__(self) -> Session:
        return self

    def __exit__(self, *exc: Any) -> None:
        self.close()


class Database:
    """An in-memory database, or a persistent database at a local path.

    Persistent storage requires Linux/macOS and exclusive process ownership.
    Closing the database closes its convenience session. Explicitly created
    sessions retain their own database handle and must be closed separately.
    """

    def __init__(self, path: str | PathLike[str] | None = None, *,
                 create: bool = False) -> None:
        self._native = _native.Database(None if path is None else fspath(path),
                                        create=create)
        self._default_session: Session | None = None

    def session(self) -> Session:
        return Session(self)

    def _session(self) -> Session:
        if self._default_session is None:
            self._default_session = self.session()
        return self._default_session

    def run(self, gql: str) -> list[Result]:
        """Execute through a retained convenience session."""
        return self._session().run(gql)

    def query(self, gql: str) -> Result:
        return self._session().query(gql)

    def checkpoint(self) -> None:
        """Flush persistent state; fails if a transaction is active."""
        self._native.checkpoint()

    def close(self) -> None:
        if self._default_session is not None:
            self._default_session.close()
        self._native.close()

    def __enter__(self) -> Database:
        return self

    def __exit__(self, *exc: Any) -> None:
        self.close()
