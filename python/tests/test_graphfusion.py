from concurrent.futures import ThreadPoolExecutor

import pyarrow as pa
import pytest

from graphfusion import Database, GraphFusionError


SOCIAL = """
CREATE GRAPH social ANY GRAPH;
SESSION SET GRAPH social;
INSERT (:Person {name: 'Alice', age: 30})-[:Knows]->
       (:Person {name: 'Bob', age: 40})-[:Knows]->
       (:Person {name: 'Cara', age: 25});
"""


def test_graph_crud_and_arrow():
    with Database() as db:
        outputs = db.run(SOCIAL)
        assert [r.kind for r in outputs] == ["command", "command", "query"]
        result = db.query("MATCH (:Person {name: 'Alice'})-[:Knows]->()-[:Knows]->(p) RETURN p.name AS name")
        assert result.to_pylist() == [{"name": "Cara"}]
        assert isinstance(result.to_arrow(), pa.Table)
        assert result.columns == ["name"]
        assert len(result) == 1
        changed = db.run("MATCH (p:Person {name: 'Alice'}) SET p.age = 31 RETURN p.age AS age")[-1]
        assert changed.fetchall() == [(31,)]
        assert changed.affected == 1
        db.run("MATCH (p:Person {name: 'Cara'}) DETACH DELETE p")
        assert db.query("MATCH (p:Person) RETURN count(p) AS n").fetchall() == [(2,)]
        empty = db.query("MATCH (p:Missing) RETURN p.name AS name")
        assert empty.columns == ["name"]
        assert empty.fetchall() == []


def test_parameters_preserve_python_types():
    with Database() as db, db.session() as s:
        values = {"big": 2**60 + 1, "flag": True, "text": "O'Reilly 中文", "real": 1.25,
                  "items": [1, 2, 3], "binary": b"abc", "missing": None}
        for name, value in values.items():
            s.set_parameter(name, value)
        result = s.query("RETURN $big AS big, $flag AS flag, $text AS text, $real AS real, $items AS items, $binary AS binary, $missing AS missing")
        assert result.to_pylist() == [values]
        assert s.query("RETURN null AS missing").to_pylist() == [{"missing": None}]
        s.set_parameter("record", {"name": "Alice"})
        with pytest.raises(GraphFusionError, match="query parameter value type"):
            s.query("RETURN $record AS record")
        with pytest.raises(OverflowError):
            s.set_parameter("overflow", 2**80)
        with pytest.raises(TypeError):
            s.set_parameter("object", object())
        with pytest.raises(TypeError):
            s.set_parameter("bad_keys", {1: "bad"})


def test_transactions_and_session_isolation():
    with Database() as db:
        db.run("CREATE GRAPH g ANY GRAPH; SESSION SET GRAPH g")
        with db.session() as s:
            s.run("SESSION SET GRAPH g")
            with s.transaction():
                result = s.run("INSERT (:Person {name: 'Alice'})")[-1]
                assert result.transaction_pending
            with pytest.raises(ValueError, match="abort"):
                with s.transaction():
                    s.run("INSERT (:Person {name: 'Bob'})")
                    raise ValueError("abort")
            assert s.query("MATCH (p:Person) RETURN p.name AS name").fetchall() == [("Alice",)]
            with pytest.raises(GraphFusionError):
                with s.transaction():
                    s.run("INSERT (:Person {name: 'Cara'})")
                    s.query("this is not valid GQL")
            assert s.query("MATCH (p:Person) RETURN count(p) AS n").fetchall() == [(1,)]
        with db.session() as other:
            with pytest.raises(GraphFusionError):
                other.query("MATCH (p:Person) RETURN p.name")


def test_persistent_reopen_and_lifetimes(tmp_path):
    path = tmp_path / "database"
    with Database(path, create=True) as db:
        db.run(SOCIAL)
        db.checkpoint()
    with Database(path) as db:
        db.run("SESSION SET GRAPH social")
        assert db.query("MATCH (p:Person) RETURN count(p) AS n").fetchall() == [(3,)]
        s = db.session()
    # Explicit sessions retain a handle after the parent database is closed.
    assert s.query("RETURN 42 AS answer").fetchall() == [(42,)]
    s.close()
    s.close()
    with pytest.raises(RuntimeError, match="closed"):
        s.query("RETURN 1")
    with pytest.raises(RuntimeError, match="closed"):
        db.session()
    with pytest.raises(GraphFusionError):
        Database(tmp_path / "missing")


def test_parallel_sessions_and_shared_session():
    with Database() as db:
        sessions = [db.session() for _ in range(4)]
        try:
            with ThreadPoolExecutor(max_workers=4) as pool:
                results = list(pool.map(lambda s: s.query("RETURN 42 AS n").fetchall(), sessions))
                assert results == [[(42,)]] * 4
                assert list(pool.map(lambda _: sessions[0].query("RETURN 7 AS n").fetchall(), range(8))) == [[(7,)]] * 8
        finally:
            for s in sessions:
                s.close()


def test_read_only_query_errors_and_limits():
    with Database() as db, db.session() as s:
        with pytest.raises(GraphFusionError):
            s.query("CREATE GRAPH g ANY GRAPH")
        with pytest.raises(GraphFusionError):
            s.set_query_limits(memory_limit_bytes=0)
        s.set_query_limits(max_path_hops=16, memory_limit_bytes=64 * 1024 * 1024)
        command = s.run("CREATE GRAPH g ANY GRAPH")[0]
        assert command.table is None
        with pytest.raises(ValueError):
            command.to_arrow()


def test_close_rolls_back_and_checkpoint_reports_busy():
    with Database() as db:
        db.run("CREATE GRAPH g ANY GRAPH; SESSION SET GRAPH g")
        s = db.session()
        s.run("SESSION SET GRAPH g; START TRANSACTION; INSERT (:Person {name: 'uncommitted'})")
        with pytest.raises(GraphFusionError, match="active"):
            db.checkpoint()
        s.close()
        db.checkpoint()
        assert db.query("MATCH (p:Person) RETURN count(p) AS n").fetchall() == [(0,)]


def test_pandas_conversion():
    pytest.importorskip("pandas")
    with Database() as db:
        assert db.query("RETURN 42 AS answer").to_pandas().to_dict("records") == [{"answer": 42}]


def test_parameterized_graph_writes():
    with Database() as db, db.session() as s:
        s.run("CREATE GRAPH g ANY GRAPH; SESSION SET GRAPH g")
        name = "O'Reilly 中文; MATCH (n) DELETE n"
        s.set_parameter("name", name)
        with s.transaction():
            s.run("INSERT (:Person {name: $name})")
        assert s.query("MATCH (p:Person) WHERE p.name = $name RETURN p.name AS name").fetchall() == [(name,)]
