use super::*;
use catalog::{GraphShape, ObjectDefinition, ObjectKind, MAIN_SCHEMA, ROOT_DIRECTORY};
use std::collections::BTreeMap;
use std::{
    fs::{File, OpenOptions as FileOptions},
    io::{BufRead, BufReader, Read, Seek, SeekFrom, Write},
    path::PathBuf,
    process::{Child, ChildStdout, Command, Stdio},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Barrier,
    },
};

struct TestDir(PathBuf);

fn arrow_graph(ids: Vec<u64>) -> graph::GraphData {
    use arrow::{
        array::UInt64Array,
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    };
    let schema = Arc::new(Schema::new(vec![Field::new(
        graph::ID,
        DataType::UInt64,
        false,
    )]));
    let batch =
        RecordBatch::try_new(schema.clone(), vec![Arc::new(UInt64Array::from(ids))]).unwrap();
    graph::GraphData::try_new(
        vec![graph::NodeTable::try_new(vec!["N".into()], schema, vec![batch]).unwrap()],
        vec![],
    )
    .unwrap()
}

#[test]
fn arrow_graph_snapshots_conflicts_and_drop_reclamation() {
    let db = Database::new();
    let id = create_graph(&db, "g");
    let mut initial = StatementTxn::begin(&db).unwrap();
    initial
        .replace_graph_data(id, arrow_graph(vec![1]))
        .unwrap();
    initial.commit().unwrap();
    let mut old = StatementTxn::begin(&db).unwrap();
    let mut first = StatementTxn::begin(&db).unwrap();
    let mut second = StatementTxn::begin(&db).unwrap();
    first
        .replace_graph_data(id, arrow_graph(vec![2, 3]))
        .unwrap();
    second
        .replace_graph_data(id, arrow_graph(vec![4, 5, 6]))
        .unwrap();
    assert_eq!(first.graph_data(id).unwrap().node_count(), 2);
    first.commit().unwrap();
    assert!(matches!(second.commit(), Err(Error::Conflict(_))));
    assert_eq!(old.graph_data(id).unwrap().node_count(), 1);
    assert!(serde_json::to_vec(old.base.storage.as_ref()).is_err());
    let mut latest = StatementTxn::begin(&db).unwrap();
    assert_eq!(latest.graph_data(id).unwrap().node_count(), 2);
    latest.commit().unwrap();
    db.session().execute("DROP GRAPH g").unwrap();
    assert_eq!(old.graph_data(id).unwrap().node_count(), 1);
    assert!(matches!(db.checkpoint(), Err(Error::Busy)));
    old.commit().unwrap();
    db.checkpoint().unwrap();
    assert!(db
        .inner
        .state
        .lock()
        .unwrap()
        .storage
        .generations
        .is_empty());
}

#[test]
fn disjoint_arrow_imports_merge_and_dropped_graphs_reject_stale_writers() {
    let db = Database::new();
    let a = create_graph(&db, "a");
    let b = create_graph(&db, "b");
    let mut first = StatementTxn::begin(&db).unwrap();
    let mut second = StatementTxn::begin(&db).unwrap();
    first.replace_graph_data(a, arrow_graph(vec![1])).unwrap();
    second
        .replace_graph_data(b, arrow_graph(vec![2, 3]))
        .unwrap();
    first.commit().unwrap();
    second.commit().unwrap();
    let mut read = StatementTxn::begin(&db).unwrap();
    assert_eq!(read.graph_data(a).unwrap().node_count(), 1);
    assert_eq!(read.graph_data(b).unwrap().node_count(), 2);
    read.commit().unwrap();
    let mut stale = StatementTxn::begin(&db).unwrap();
    stale.replace_graph_data(a, arrow_graph(vec![9])).unwrap();
    db.session()
        .execute("DROP GRAPH a; CREATE GRAPH a ANY GRAPH")
        .unwrap();
    assert!(matches!(stale.commit(), Err(Error::Conflict(_))));
}

#[test]
fn memory_graph_import_fails_closed_for_durable_and_typed_graphs() {
    let dir = TestDir::new();
    {
        let db = dir.open();
        let mut session = db.session();
        session
            .execute("CREATE GRAPH g ANY GRAPH; SESSION SET GRAPH g")
            .unwrap();
        let seq = db.inner.state.lock().unwrap().commit_seq;
        assert!(matches!(
            session.replace_graph_data(arrow_graph(vec![1])),
            Err(Error::UnsupportedFeature(_))
        ));
        assert_eq!(db.inner.state.lock().unwrap().commit_seq, seq);
        db.checkpoint().unwrap();
    }
    let db = dir.open();
    let id = graph_id(&db, "g").unwrap();
    assert_eq!(
        StatementTxn::begin(&db)
            .unwrap()
            .graph_data(id)
            .unwrap()
            .node_count(),
        0
    );
    let db = Database::new();
    let mut session = db.session();
    session
        .execute("CREATE GRAPH TYPE t AS { NODE N }; CREATE GRAPH g TYPED t; SESSION SET GRAPH g")
        .unwrap();
    assert!(matches!(
        session.replace_graph_data(arrow_graph(vec![1])),
        Err(Error::UnsupportedFeature(_))
    ));
    session.execute("SESSION CLOSE").unwrap();
    assert!(matches!(
        session.replace_graph_data(arrow_graph(vec![1])),
        Err(Error::SessionClosed)
    ));
}
impl TestDir {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "graphfusion-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn open(&self) -> Database {
        Database::open(
            &self.0,
            OpenOptions {
                create_if_missing: true,
            },
        )
        .unwrap()
    }
}
impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn graph_id(db: &Database, name: &str) -> Option<u64> {
    db.with_catalog(|c| c.lookup(MAIN_SCHEMA, ObjectKind::Graph, name).map(|e| e.id))
        .unwrap()
}
fn create_graph(db: &Database, name: &str) -> u64 {
    let mut tx = StatementTxn::begin(db).unwrap();
    let id = tx
        .create_graph(MAIN_SCHEMA, name, GraphShape::Open)
        .unwrap();
    tx.commit().unwrap();
    id
}
fn seed(db: &Database) -> (u64, u64) {
    let mut tx = StatementTxn::begin(db).unwrap();
    let doomed = tx
        .create_graph(MAIN_SCHEMA, "doomed", GraphShape::Open)
        .unwrap();
    let keeper = tx
        .create_graph(MAIN_SCHEMA, "keeper", GraphShape::Open)
        .unwrap();
    tx.write_row(doomed, "row", Some(b"old graph data".to_vec()))
        .unwrap();
    tx.write_row(keeper, "row", Some(b"before".to_vec()))
        .unwrap();
    tx.commit().unwrap();
    (doomed, keeper)
}

#[test]
fn catalog_session_and_reopen() {
    let dir = TestDir::new();
    {
        let db = dir.open();
        db.create_directory(&["app"]).unwrap();
        let mut a = db.session();
        a.execute("CREATE SCHEMA /app/social; SESSION SET SCHEMA /app/social; CREATE GRAPH g ANY GRAPH; SESSION SET GRAPH g; SESSION SET TIME ZONE '+08:00'; SESSION SET VALUE $limit INTEGER = 10").unwrap();
        assert_eq!(a.state().time_zone, "+08:00");
        assert_eq!(a.state().parameters["limit"].value, Value::Integer(10));
        let b = db.session();
        assert_eq!(b.state().current_schema, MAIN_SCHEMA);
        assert!(b.state().parameters.is_empty());
        let saved = a.state().clone();
        assert!(a.execute("SESSION SET TIME ZONE 'invalid'").is_err());
        assert_eq!(*a.state(), saved);
        a.execute("SESSION RESET ALL PARAMETERS; SESSION RESET ALL CHARACTERISTICS")
            .unwrap();
        assert!(a.state().parameters.is_empty());
        assert_eq!(a.state().time_zone, "UTC");
        assert_eq!(a.state().current_graph, None);
        a.execute("SESSION CLOSE").unwrap();
        assert!(matches!(
            a.execute("CREATE GRAPH x ANY GRAPH"),
            Err(Error::SessionClosed)
        ));
        db.checkpoint().unwrap();
    }
    let db = dir.open();
    let mut s = db.session();
    s.execute("SESSION SET SCHEMA /app/social; SESSION SET GRAPH g")
        .unwrap();
    assert!(s.state().current_graph.is_some());
    assert!(s.state().parameters.is_empty());
}

#[test]
fn graph_types_and_restrict_survive_recovery() {
    let dir = TestDir::new();
    {
        let db = dir.open();
        let mut s = db.session();
        s.execute("CREATE GRAPH TYPE social AS { NODE Person {name STRING NOT NULL, age UINT16, tags LIST<STRING>}, DIRECTED EDGE Knows FROM Person TO Person {since INTEGER} }; CREATE GRAPH g TYPED social").unwrap();
        assert!(matches!(
            s.execute("DROP GRAPH TYPE social"),
            Err(Error::DependencyExists(_))
        ));
        assert!(matches!(
            s.execute("CREATE OR REPLACE GRAPH TYPE social AS { NODE X }"),
            Err(Error::DependencyExists(_))
        ));
        s.execute("CREATE GRAPH TYPE copied AS COPY OF social; CREATE GRAPH duplicate LIKE g")
            .unwrap();
    }
    let db = dir.open();
    db.with_catalog(|catalog| {
        let entry = catalog
            .lookup(MAIN_SCHEMA, ObjectKind::GraphType, "social")
            .unwrap();
        let ObjectDefinition::GraphType(definition) = &entry.definition else {
            panic!()
        };
        assert!(!definition.nodes["Person"].properties["name"].nullable);
        assert_eq!(
            definition.nodes["Person"].properties["age"].parameters["precision"],
            16
        );
        assert_eq!(definition.edges["Knows"].source.as_deref(), Some("Person"));
    })
    .unwrap();
    db.session()
        .execute("DROP GRAPH g; DROP GRAPH TYPE social")
        .unwrap();
    assert!(graph_id(&db, "duplicate").is_some());
}

#[test]
fn nested_definitions_are_rejected_before_commit_or_remain_recoverable() {
    // Exercise both sides of the JSON recursion limit, including checkpoint envelopes.
    let mut accepted = 0;
    let mut rejected = 0;
    for depth in 54..=60 {
        for prefix in ["CREATE GRAPH g", "CREATE GRAPH TYPE t AS"] {
            let dir = TestDir::new();
            let db = dir.open();
            let mut session = db.session();
            let value_type = format!("{}INTEGER{}", "LIST<".repeat(depth), ">".repeat(depth));
            let statement = format!("{prefix} {{ NODE N {{ value {value_type} }} }}");
            let before = db.inner.state.lock().unwrap().commit_seq;
            let committed = match session.execute(&statement) {
                Ok(_) => {
                    accepted += 1;
                    true
                }
                Err(Error::UnsupportedFeature(message)) => {
                    assert!(message.contains("recursion limit"), "{message}");
                    rejected += 1;
                    assert_eq!(db.inner.state.lock().unwrap().commit_seq, before);
                    false
                }
                Err(error) => panic!("depth {depth}: {error}"),
            };
            if depth == 60 {
                assert!(!committed, "unsupported nesting was acknowledged");
            }
            session
                .execute("CREATE GRAPH after_write ANY GRAPH")
                .unwrap();
            let count = db
                .with_catalog(|c| c.children(MAIN_SCHEMA).count())
                .unwrap();
            assert_eq!(count, 1 + usize::from(committed));
            drop(session);
            drop(db);
            let recovered = dir.open();
            assert_eq!(
                recovered
                    .with_catalog(|c| c.children(MAIN_SCHEMA).count())
                    .unwrap(),
                count
            );
            recovered.checkpoint().unwrap();
            drop(recovered);
            let recovered = dir.open();
            assert_eq!(
                recovered
                    .with_catalog(|c| c.children(MAIN_SCHEMA).count())
                    .unwrap(),
                count
            );
            assert!(graph_id(&recovered, "after_write").is_some());
        }
    }
    assert!(accepted > 0);
    assert!(rejected > 0);
}

#[test]
fn batches_next_and_transaction_preflight() {
    let db = Database::new();
    let mut s = db.session();
    assert!(matches!(
        s.execute("CREATE GRAPH a ANY GRAPH; START TRANSACTION; COMMIT"),
        Err(Error::UnsupportedFeature(_))
    ));
    assert!(graph_id(&db, "a").is_none());
    assert!(matches!(
        s.execute("CREATE GRAPH a ANY GRAPH; CREATE GRAPH a ANY GRAPH"),
        Err(Error::AlreadyExists(_))
    ));
    assert!(graph_id(&db, "a").is_some());
    assert!(s
        .execute("CREATE GRAPH b ANY GRAPH NEXT CREATE GRAPH b ANY GRAPH")
        .is_err());
    assert!(graph_id(&db, "b").is_none());
    let result = s
        .execute("CREATE GRAPH c ANY GRAPH NEXT CREATE GRAPH d ANY GRAPH")
        .unwrap();
    assert_eq!(result.statements.len(), 1);
    assert_eq!(result.statements[0].affected_objects, 2);
    assert!(s
        .execute("CREATE GRAPH e ANY GRAPH CREATE GRAPH e ANY GRAPH")
        .is_err());
    assert!(graph_id(&db, "e").is_none());
}

#[test]
fn old_session_reference_never_rebinds() {
    let db = Database::new();
    let mut a = db.session();
    let mut b = db.session();
    a.execute("CREATE GRAPH g ANY GRAPH; SESSION SET GRAPH g; SESSION SET GRAPH $saved = g")
        .unwrap();
    let old = a.state().current_graph.unwrap();
    b.execute("DROP GRAPH g; CREATE GRAPH g ANY GRAPH").unwrap();
    assert_ne!(old, graph_id(&db, "g").unwrap());
    assert!(matches!(
        a.execute("CREATE GRAPH copy LIKE CURRENT_GRAPH"),
        Err(Error::InvalidReference(_))
    ));
    assert!(matches!(
        a.execute("SESSION SET GRAPH $saved"),
        Err(Error::InvalidReference(_))
    ));
    a.execute("SESSION SET GRAPH g").unwrap();
    assert_ne!(old, a.state().current_graph.unwrap());
}

#[test]
fn values_are_typed_and_set_is_atomic() {
    let db = Database::new();
    let mut s = db.session();
    s.execute("SESSION SET VALUE $count INTEGER NOT NULL = 12; SESSION SET VALUE $list LIST<STRING> = ['a', 'b']").unwrap();
    let before = s.state().clone();
    assert!(s
        .execute("SESSION SET VALUE $count INTEGER NOT NULL = NULL")
        .is_err());
    assert_eq!(*s.state(), before);
    s.execute("SESSION SET VALUE IF NOT EXISTS $count INTEGER = 'ignored'")
        .unwrap();
    assert_eq!(s.state().parameters["count"].value, Value::Integer(12));
    let g = create_graph(&db, "refgraph");
    s.set_parameter("input", Value::Graph(g)).unwrap();
    s.execute("SESSION SET GRAPH $input").unwrap();
    s.set_parameter(
        "rows",
        Value::BindingTable(vec![BTreeMap::from([("id".into(), Value::Integer(3))])]),
    )
    .unwrap();
    s.execute("SESSION SET BINDING TABLE $copy BINDING TABLE {id INTEGER} = VARIABLE $rows")
        .unwrap();
}

#[test]
fn dynamic_unions_preserve_component_nullability() {
    let db = Database::new();
    let mut session = db.session();
    for initializer in ["12", "'text'"] {
        session
            .execute(&format!(
                "SESSION SET VALUE $x INTEGER NOT NULL | STRING NOT NULL = {initializer}"
            ))
            .unwrap();
        assert!(
            !session.state().parameters["x"]
                .declared_type
                .as_ref()
                .unwrap()
                .nullable
        );
    }
    let before = session.state().clone();
    for initializer in ["NULL", "UNKNOWN"] {
        assert!(matches!(
            session.execute(&format!(
                "SESSION SET VALUE $x INTEGER NOT NULL | STRING NOT NULL = {initializer}"
            )),
            Err(Error::InvalidDefinition(_))
        ));
        assert_eq!(*session.state(), before);
    }
    session
        .execute("SESSION SET VALUE $nullable INTEGER | STRING = NULL")
        .unwrap();
    assert_eq!(session.state().parameters["nullable"].value, Value::Null);
    assert!(
        session.state().parameters["nullable"]
            .declared_type
            .as_ref()
            .unwrap()
            .nullable
    );
    session
        .execute("SESSION SET VALUE $items LIST<INTEGER NOT NULL | STRING NOT NULL> = [1, 'two']")
        .unwrap();
    assert!(matches!(
        session
            .execute("SESSION SET VALUE $items LIST<INTEGER NOT NULL | STRING NOT NULL> = [NULL]"),
        Err(Error::InvalidDefinition(_))
    ));
}

#[test]
fn unary_not_preserves_unknown_in_session_initializers() {
    let db = Database::new();
    let mut session = db.session();
    for (expression, expected) in [
        ("NOT TRUE", Value::Boolean(false)),
        ("NOT FALSE", Value::Boolean(true)),
        ("NOT UNKNOWN", Value::Null),
        ("NOT NULL", Value::Null),
        ("NOT NOT UNKNOWN", Value::Null),
    ] {
        session
            .execute(&format!("SESSION SET VALUE $x BOOLEAN = {expression}"))
            .unwrap();
        assert_eq!(session.state().parameters["x"].value, expected);
    }
    let before = session.state().clone();
    for statement in [
        "SESSION SET VALUE $x BOOLEAN NOT NULL = NOT UNKNOWN",
        "SESSION SET VALUE $x BOOLEAN = NOT 1",
    ] {
        assert!(matches!(
            session.execute(statement),
            Err(Error::InvalidDefinition(_))
        ));
        assert_eq!(*session.state(), before);
    }
}

#[test]
fn paths_preserve_delimited_components_and_parent_resolution() {
    let db = Database::new();
    db.create_directory(&["tenant"]).unwrap();
    let mut s = db.session();
    s.execute("CREATE SCHEMA /tenant/one; CREATE SCHEMA /tenant/two; SESSION SET SCHEMA /tenant/one; CREATE GRAPH `a/b` ANY GRAPH; CREATE GRAPH ../two/other ANY GRAPH").unwrap();
    s.execute("SESSION SET GRAPH ./`a/b`; SESSION SET GRAPH ../two/other")
        .unwrap();
    assert!(s.execute("CREATE GRAPH /missing/g ANY GRAPH").is_err());
    s.execute("CREATE GRAPH lower ANY GRAPH; CREATE GRAPH LOWER ANY GRAPH")
        .unwrap();
    assert!(s.execute("DROP SCHEMA /tenant/one").is_err());
}

#[test]
fn independent_writers_merge_and_same_name_conflicts() {
    let db = Database::new();
    let mut a = StatementTxn::begin(&db).unwrap();
    let mut b = StatementTxn::begin(&db).unwrap();
    a.create_graph(MAIN_SCHEMA, "a", GraphShape::Open).unwrap();
    b.create_graph(MAIN_SCHEMA, "b", GraphShape::Open).unwrap();
    a.commit().unwrap();
    b.commit().unwrap();
    assert!(graph_id(&db, "a").is_some());
    assert!(graph_id(&db, "b").is_some());
    let mut a = StatementTxn::begin(&db).unwrap();
    let mut b = StatementTxn::begin(&db).unwrap();
    a.create_graph(MAIN_SCHEMA, "same", GraphShape::Open)
        .unwrap();
    b.create_graph(MAIN_SCHEMA, "same", GraphShape::Open)
        .unwrap();
    a.commit().unwrap();
    assert!(matches!(b.commit(), Err(Error::Conflict(_))));
}

#[test]
fn negative_name_and_collection_reads_detect_phantoms() {
    let db = Database::new();
    let mut a = StatementTxn::begin(&db).unwrap();
    assert!(a.lookup(MAIN_SCHEMA, ObjectKind::Graph, "absent").is_none());
    a.create_graph(MAIN_SCHEMA, "other", GraphShape::Open)
        .unwrap();
    let id = create_graph(&db, "absent");
    let mut b = StatementTxn::begin(&db).unwrap();
    b.drop_object(id).unwrap();
    b.commit().unwrap();
    assert!(matches!(a.commit(), Err(Error::Conflict(_))));
    let mut a = StatementTxn::begin(&db).unwrap();
    assert!(a.children(MAIN_SCHEMA).is_empty());
    a.create_graph(MAIN_SCHEMA, "after_scan", GraphShape::Open)
        .unwrap();
    create_graph(&db, "phantom");
    assert!(matches!(a.commit(), Err(Error::Conflict(_))));
}

#[test]
fn schema_and_type_dependency_races() {
    let db = Database::new();
    let mut s = db.session();
    s.execute("CREATE SCHEMA empty; CREATE GRAPH TYPE t AS { NODE N }")
        .unwrap();
    let (schema, typ) = db
        .with_catalog(|c| {
            (
                c.lookup(ROOT_DIRECTORY, ObjectKind::Schema, "empty")
                    .unwrap()
                    .id,
                c.lookup(MAIN_SCHEMA, ObjectKind::GraphType, "t")
                    .unwrap()
                    .id,
            )
        })
        .unwrap();
    let mut dropper = StatementTxn::begin(&db).unwrap();
    dropper.drop_object(schema).unwrap();
    let mut creator = StatementTxn::begin(&db).unwrap();
    creator.create_graph(schema, "g", GraphShape::Open).unwrap();
    creator.commit().unwrap();
    assert!(matches!(dropper.commit(), Err(Error::Conflict(_))));
    let mut dropper = StatementTxn::begin(&db).unwrap();
    dropper.drop_object(typ).unwrap();
    let mut creator = StatementTxn::begin(&db).unwrap();
    creator
        .create_graph(MAIN_SCHEMA, "typed", GraphShape::Named(typ))
        .unwrap();
    creator.commit().unwrap();
    assert!(matches!(dropper.commit(), Err(Error::Conflict(_))));
}

#[test]
fn drop_and_data_commit_share_snapshot_and_retirement() {
    let dir = TestDir::new();
    let db = dir.open();
    let (doomed, keeper) = seed(&db);
    db.checkpoint().unwrap();
    let mut old_reader = StatementTxn::begin(&db).unwrap();
    let mut old_writer = StatementTxn::begin(&db).unwrap();
    old_writer
        .write_row(doomed, "row", Some(b"late".to_vec()))
        .unwrap();
    let mut dropper = StatementTxn::begin(&db).unwrap();
    dropper.drop_object(doomed).unwrap();
    dropper
        .write_row(keeper, "row", Some(b"after".to_vec()))
        .unwrap();
    dropper.commit().unwrap();
    assert_eq!(
        old_reader.read_row(doomed, "row").unwrap(),
        Some(b"old graph data".to_vec())
    );
    assert_eq!(
        old_reader.read_row(keeper, "row").unwrap(),
        Some(b"before".to_vec())
    );
    assert!(matches!(old_writer.commit(), Err(Error::Conflict(_))));
    assert!(matches!(db.checkpoint(), Err(Error::Busy)));
    assert!(graph_id(&db, "doomed").is_none());
    drop(old_reader);
    db.checkpoint().unwrap();
    drop(db);
    let db = dir.open();
    let mut tx = StatementTxn::begin(&db).unwrap();
    assert_eq!(tx.read_row(keeper, "row").unwrap(), Some(b"after".to_vec()));
    assert!(tx
        .base
        .storage
        .generations
        .values()
        .all(|g| g.retired_at.is_none()));
}

#[test]
fn write_before_drop_retires_latest_data_and_row_conflicts() {
    let db = Database::new();
    let (doomed, _) = seed(&db);
    let mut dropper = StatementTxn::begin(&db).unwrap();
    dropper.drop_object(doomed).unwrap();
    let mut a = StatementTxn::begin(&db).unwrap();
    let mut b = StatementTxn::begin(&db).unwrap();
    a.write_row(doomed, "new", Some(vec![1])).unwrap();
    b.write_row(doomed, "new", Some(vec![2])).unwrap();
    a.commit().unwrap();
    assert!(matches!(b.commit(), Err(Error::Conflict(_))));
    dropper.commit().unwrap();
    db.checkpoint().unwrap();
    assert_eq!(db.inner.state.lock().unwrap().storage.generations.len(), 1);
    assert!(graph_id(&db, "doomed").is_none());
}

#[test]
fn aborted_ids_are_not_reused_after_restart() {
    let dir = TestDir::new();
    let id = {
        let db = dir.open();
        let mut tx = StatementTxn::begin(&db).unwrap();
        tx.create_graph(MAIN_SCHEMA, "aborted", GraphShape::Open)
            .unwrap()
    };
    let db = dir.open();
    assert!(graph_id(&db, "aborted").is_none());
    assert!(create_graph(&db, "later") > id);
}

#[test]
fn multiple_threads_commit_disjoint_changes() {
    let db = Database::new();
    let barrier = Arc::new(Barrier::new(8));
    std::thread::scope(|scope| {
        for i in 0..8 {
            let db = db.clone();
            let barrier = barrier.clone();
            scope.spawn(move || {
                let mut tx = StatementTxn::begin(&db).unwrap();
                tx.create_graph(MAIN_SCHEMA, &format!("g{i}"), GraphShape::Open)
                    .unwrap();
                barrier.wait();
                tx.commit().unwrap();
            });
        }
    });
    assert_eq!(
        db.with_catalog(|c| c.children(MAIN_SCHEMA).count())
            .unwrap(),
        8
    );
}

struct Worker {
    child: Child,
    output: BufReader<ChildStdout>,
}
impl Worker {
    fn start(dir: &TestDir, mode: &str, name: &str) -> Self {
        let mut child = child_command(dir, mode)
            .env("GF_NAME", name)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut output = BufReader::new(child.stdout.take().unwrap());
        loop {
            let mut line = String::new();
            assert_ne!(
                output.read_line(&mut line).unwrap(),
                0,
                "worker exited before barrier"
            );
            if line.contains("GF_READY") {
                break;
            }
        }
        Self { child, output }
    }
    fn release(&mut self) {
        writeln!(self.child.stdin.as_mut().unwrap(), "go").unwrap();
    }
    fn finish(mut self) -> String {
        let mut out = String::new();
        self.output.read_to_string(&mut out).unwrap();
        assert!(self.child.wait().unwrap().success(), "{out}");
        out
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn child_command(dir: &TestDir, mode: &str) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "tests::child_worker", "--nocapture"])
        .env("GF_DIR", &dir.0)
        .env("GF_MODE", mode);
    command
}

#[test]
fn multiprocess_writers_and_refresh() {
    let dir = TestDir::new();
    let db = dir.open();
    let mut a = Worker::start(&dir, "create", "a");
    let mut b = Worker::start(&dir, "create", "b");
    a.release();
    b.release();
    assert!(a.finish().contains("GF_COMMITTED"));
    assert!(b.finish().contains("GF_COMMITTED"));
    assert!(graph_id(&db, "a").is_some());
    assert!(graph_id(&db, "b").is_some());
    let mut a = Worker::start(&dir, "create", "same");
    let mut b = Worker::start(&dir, "create", "same");
    a.release();
    b.release();
    let outcomes = [a.finish(), b.finish()];
    assert_eq!(
        outcomes
            .iter()
            .filter(|s| s.contains("GF_COMMITTED"))
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|s| s.contains("GF_CONFLICT"))
            .count(),
        1
    );
    db.checkpoint().unwrap();
}

#[test]
fn cross_process_reader_pins_data_and_kill_releases_locks() {
    let dir = TestDir::new();
    let db = dir.open();
    let (doomed, _) = seed(&db);
    let mut reader = Worker::start(&dir, "reader", "");
    db.session().execute("DROP GRAPH doomed").unwrap();
    assert!(matches!(db.checkpoint(), Err(Error::Busy)));
    reader.release();
    assert!(reader.finish().contains("GF_OLD_OK"));
    db.checkpoint().unwrap();
    assert!(graph_id(&db, "doomed").is_none());
    let mut writer = Worker::start(&dir, "create", "uncommitted");
    assert!(matches!(db.checkpoint(), Err(Error::Busy)));
    writer.child.kill().unwrap();
    writer.child.wait().unwrap();
    db.checkpoint().unwrap();
    assert!(graph_id(&db, "uncommitted").is_none());
    assert_ne!(create_graph(&db, "doomed"), doomed);
}

#[test]
fn commit_crash_matrix_never_splits_catalog_and_data() {
    for (point, committed) in [
        ("wal_header", false),
        ("wal_payload", false),
        ("wal_commit", true),
        ("wal_sync", true),
        ("before_publish", true),
    ] {
        let dir = TestDir::new();
        let db = dir.open();
        let (_, keeper) = seed(&db);
        db.checkpoint().unwrap();
        let result = child_command(&dir, "drop_and_write")
            .env("GRAPHFUSION_TEST_CRASH", point)
            .output()
            .unwrap();
        assert_eq!(
            result.status.code(),
            Some(86),
            "{point}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(graph_id(&db, "doomed").is_none(), committed, "{point}");
        let mut tx = StatementTxn::begin(&db).unwrap();
        assert_eq!(
            tx.read_row(keeper, "row").unwrap(),
            Some(if committed {
                b"after".to_vec()
            } else {
                b"before".to_vec()
            }),
            "{point}"
        );
        drop(tx);
        create_graph(&db, "after_recovery");
        db.checkpoint().unwrap();
    }
}

#[test]
fn checkpoint_crash_matrix_retains_joint_state() {
    for point in [
        "checkpoint_catalog",
        "checkpoint_data",
        "manifest_rename",
        "manifest_sync",
        "cleanup",
    ] {
        let dir = TestDir::new();
        let db = dir.open();
        let (doomed, keeper) = seed(&db);
        db.checkpoint().unwrap();
        let mut tx = StatementTxn::begin(&db).unwrap();
        tx.drop_object(doomed).unwrap();
        tx.write_row(keeper, "row", Some(b"after".to_vec()))
            .unwrap();
        tx.commit().unwrap();
        let result = child_command(&dir, "checkpoint")
            .env("GRAPHFUSION_TEST_CRASH", point)
            .output()
            .unwrap();
        assert_eq!(
            result.status.code(),
            Some(86),
            "{point}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(graph_id(&db, "doomed").is_none());
        let mut tx = StatementTxn::begin(&db).unwrap();
        assert_eq!(tx.read_row(keeper, "row").unwrap(), Some(b"after".to_vec()));
        drop(tx);
        db.checkpoint().unwrap();
        db.checkpoint().unwrap();
    }
}

#[test]
fn recovery_requires_a_durable_manifest_before_accepting_writes() {
    let dir = TestDir::new();
    let db = dir.open();
    create_graph(&db, "before");
    drop(db);
    let crashed = child_command(&dir, "checkpoint")
        .env("GRAPHFUSION_TEST_CRASH", "manifest_rename")
        .output()
        .unwrap();
    assert_eq!(crashed.status.code(), Some(86));
    let wal = dir.0.join("wal-1.log");
    let before = fs::read(&wal).unwrap();
    let failed = child_command(&dir, "recovery_sync_error")
        .env("GRAPHFUSION_TEST_IO", "recovery_directory_sync")
        .output()
        .unwrap();
    assert!(
        failed.status.success(),
        "{}",
        String::from_utf8_lossy(&failed.stderr)
    );
    assert_eq!(fs::read(&wal).unwrap(), before);
    let recovered = child_command(&dir, "recovery_write").output().unwrap();
    assert!(
        recovered.status.success(),
        "{}",
        String::from_utf8_lossy(&recovered.stderr)
    );
    let db = dir.open();
    assert!(graph_id(&db, "before").is_some());
    assert!(graph_id(&db, "after_recovery").is_some());
    db.checkpoint().unwrap();
    drop(db);
    assert!(graph_id(&dir.open(), "after_recovery").is_some());
}

#[test]
fn corrupted_complete_records_fail_closed_and_torn_tail_is_removed() {
    let dir = TestDir::new();
    let db = dir.open();
    create_graph(&db, "before");
    let log = dir.0.join("wal-0.log");
    let valid_length = fs::metadata(&log).unwrap().len();
    FileOptions::new()
        .append(true)
        .open(&log)
        .unwrap()
        .write_all(b"GFLOG")
        .unwrap();
    assert!(graph_id(&db, "before").is_some());
    assert_eq!(fs::metadata(&log).unwrap().len(), valid_length);
    let mut file = FileOptions::new()
        .write(true)
        .read(true)
        .open(&log)
        .unwrap();
    file.seek(SeekFrom::Start(24)).unwrap();
    let mut byte = [0];
    file.read_exact(&mut byte).unwrap();
    byte[0] ^= 1;
    file.seek(SeekFrom::Start(24)).unwrap();
    file.write_all(&byte).unwrap();
    file.sync_all().unwrap();
    assert!(matches!(db.with_catalog(|_| ()), Err(Error::Corrupt(_))));
}

#[test]
fn corrupted_checkpoint_never_falls_back_to_retired_files() {
    let dir = TestDir::new();
    let db = dir.open();
    seed(&db);
    db.checkpoint().unwrap();
    db.session().execute("DROP GRAPH doomed").unwrap();
    db.checkpoint().unwrap();
    File::create(dir.0.join("catalog-2.snapshot"))
        .unwrap()
        .write_all(b"broken")
        .unwrap();
    assert!(matches!(db.with_catalog(|_| ()), Err(Error::Corrupt(_))));
}

#[test]
fn initialization_crashes_resume_but_missing_manifest_does_not_reset_data() {
    for point in [
        "init_marker",
        "checkpoint_catalog",
        "checkpoint_data",
        "manifest_rename",
        "manifest_sync",
    ] {
        let dir = TestDir::new();
        let status = child_command(&dir, "initialize")
            .env("GRAPHFUSION_TEST_CRASH", point)
            .output()
            .unwrap();
        assert_eq!(
            status.status.code(),
            Some(86),
            "{point}: {}",
            String::from_utf8_lossy(&status.stderr)
        );
        let db = dir.open();
        create_graph(&db, "preserved");
        drop(db);
        fs::remove_file(dir.0.join("MANIFEST")).unwrap();
        assert!(matches!(
            Database::open(
                &dir.0,
                OpenOptions {
                    create_if_missing: true
                }
            ),
            Err(Error::Corrupt(_))
        ));
    }
}

#[test]
fn swapping_database_files_is_detected() {
    for file in ["catalog-0.snapshot", "data-0.snapshot", "wal-0.log"] {
        let a = TestDir::new();
        let b = TestDir::new();
        let db = a.open();
        let _other = b.open();
        fs::copy(b.0.join(file), a.0.join(file)).unwrap();
        assert!(
            matches!(db.with_catalog(|_| ()), Err(Error::Corrupt(_))),
            "{file}"
        );
    }
}

#[test]
fn row_tombstones_do_not_resurrect_after_checkpoint() {
    let dir = TestDir::new();
    let db = dir.open();
    let (_, keeper) = seed(&db);
    db.checkpoint().unwrap();
    let mut old = StatementTxn::begin(&db).unwrap();
    let mut deleting = StatementTxn::begin(&db).unwrap();
    deleting.write_row(keeper, "row", None).unwrap();
    deleting.commit().unwrap();
    assert_eq!(
        old.read_row(keeper, "row").unwrap(),
        Some(b"before".to_vec())
    );
    drop(old);
    db.checkpoint().unwrap();
    drop(db);
    let db = dir.open();
    let mut tx = StatementTxn::begin(&db).unwrap();
    assert_eq!(tx.read_row(keeper, "row").unwrap(), None);
}

#[test]
fn schema_removal_wins_against_uncommitted_child() {
    let db = Database::new();
    db.session().execute("CREATE SCHEMA empty").unwrap();
    let schema = db
        .with_catalog(|c| {
            c.lookup(ROOT_DIRECTORY, ObjectKind::Schema, "empty")
                .unwrap()
                .id
        })
        .unwrap();
    let mut creating = StatementTxn::begin(&db).unwrap();
    creating
        .create_graph(schema, "late", GraphShape::Open)
        .unwrap();
    db.session().execute("DROP SCHEMA empty").unwrap();
    assert!(matches!(creating.commit(), Err(Error::Conflict(_))));
}

#[test]
fn graph_replacement_preserves_old_data_and_allocates_new_storage() {
    let db = Database::new();
    let (old_id, _) = seed(&db);
    let mut reader = StatementTxn::begin(&db).unwrap();
    db.session()
        .execute("CREATE OR REPLACE GRAPH doomed ANY GRAPH")
        .unwrap();
    let new_id = graph_id(&db, "doomed").unwrap();
    assert_ne!(old_id, new_id);
    assert_eq!(
        reader.read_row(old_id, "row").unwrap(),
        Some(b"old graph data".to_vec())
    );
    let mut now = StatementTxn::begin(&db).unwrap();
    assert_eq!(now.read_row(new_id, "row").unwrap(), None);
    drop(reader);
    drop(now);
    db.checkpoint().unwrap();
}

#[test]
fn integer_bounds_and_closed_reference_constraints_are_enforced() {
    let db = Database::new();
    let mut s = db.session();
    s.execute("SESSION SET VALUE $tiny INT8 = 127").unwrap();
    assert!(s.execute("SESSION SET VALUE $tiny INT8 = 128").is_err());
    assert!(s.execute("SESSION SET VALUE $u UINT8 = -1").is_err());
    assert!(s.execute("SESSION SET VALUE $u UINT8 = 256").is_err());
    assert_eq!(s.state().parameters["tiny"].value, Value::Integer(127));
    s.execute("CREATE GRAPH g { NODE Person }; SESSION SET GRAPH $g GRAPH { NODE Person } = g")
        .unwrap();
    assert!(s
        .execute("SESSION SET GRAPH $g GRAPH { NODE Company } = g")
        .is_err());
    s.execute("CREATE GRAPH TYPE refs AS { NODE Holder {e (:Person)-[:KNOWS {since INTEGER}]->(:Person)} }").unwrap();
}

#[test]
fn approximate_numeric_scale_survives_recovery_and_reference_checks() {
    let dir = TestDir::new();
    {
        let db = dir.open();
        db.session().execute("CREATE GRAPH TYPE floating AS { NODE N {v FLOAT(24,2)} }; CREATE GRAPH named TYPED floating; CREATE GRAPH inline { NODE N {v FLOAT(24,2)} }").unwrap();
    }
    // First recover from the WAL, then recover from a checkpoint.
    for _ in 0..2 {
        let db = dir.open();
        db.with_catalog(|catalog| {
            let entry = catalog
                .lookup(MAIN_SCHEMA, ObjectKind::GraphType, "floating")
                .unwrap();
            let ObjectDefinition::GraphType(definition) = &entry.definition else {
                panic!()
            };
            let value_type = &definition.nodes["N"].properties["v"];
            assert_eq!(value_type.parameters.get("precision"), Some(&24));
            assert_eq!(value_type.parameters.get("scale"), Some(&2));
        })
        .unwrap();
        let mut session = db.session();
        for graph in ["named", "inline"] {
            session
                .execute(&format!(
                    "SESSION SET GRAPH $g GRAPH {{ NODE N {{v FLOAT(24,2)}} }} = {graph}"
                ))
                .unwrap();
            let before = session.state().clone();
            assert!(matches!(
                session.execute(&format!(
                    "SESSION SET GRAPH $g GRAPH {{ NODE N {{v FLOAT(24,4)}} }} = {graph}"
                )),
                Err(Error::InvalidDefinition(_))
            ));
            assert_eq!(*session.state(), before);
        }
        assert!(matches!(
            session.execute("SESSION SET VALUE $bad FLOAT(24,25) = 1.0"),
            Err(Error::InvalidDefinition(_))
        ));
        db.checkpoint().unwrap();
    }
}

#[test]
fn indeterminate_io_commit_requires_reopen_and_recovers_joint_state() {
    let dir = TestDir::new();
    let db = dir.open();
    let (_, keeper) = seed(&db);
    let result = child_command(&dir, "io_error")
        .env("GRAPHFUSION_TEST_IO", "wal_sync")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(graph_id(&db, "doomed").is_none());
    let mut tx = StatementTxn::begin(&db).unwrap();
    assert_eq!(tx.read_row(keeper, "row").unwrap(), Some(b"after".to_vec()));
}

#[test]
fn child_worker() {
    let Ok(mode) = std::env::var("GF_MODE") else {
        return;
    };
    let db = Database::open(
        std::env::var("GF_DIR").unwrap(),
        OpenOptions {
            create_if_missing: mode == "initialize",
        },
    );
    if mode == "recovery_sync_error" {
        assert!(matches!(db, Err(Error::Io(_))), "{db:?}");
        return;
    }
    let db = db.unwrap();
    match mode.as_str() {
        "recovery_write" => {
            create_graph(&db, "after_recovery");
        }
        "create" => {
            let mut tx = StatementTxn::begin(&db).unwrap();
            tx.create_graph(
                MAIN_SCHEMA,
                &std::env::var("GF_NAME").unwrap(),
                GraphShape::Open,
            )
            .unwrap();
            child_barrier();
            match tx.commit() {
                Ok(_) => println!("GF_COMMITTED"),
                Err(Error::Conflict(_)) => println!("GF_CONFLICT"),
                Err(e) => panic!("{e}"),
            }
        }
        "reader" => {
            let mut tx = StatementTxn::begin(&db).unwrap();
            let graph = tx
                .lookup(MAIN_SCHEMA, ObjectKind::Graph, "doomed")
                .unwrap()
                .id;
            child_barrier();
            assert_eq!(
                tx.read_row(graph, "row").unwrap(),
                Some(b"old graph data".to_vec())
            );
            println!("GF_OLD_OK");
        }
        "drop_and_write" | "io_error" => {
            let mut tx = StatementTxn::begin(&db).unwrap();
            let doomed = tx
                .lookup(MAIN_SCHEMA, ObjectKind::Graph, "doomed")
                .unwrap()
                .id;
            let keeper = tx
                .lookup(MAIN_SCHEMA, ObjectKind::Graph, "keeper")
                .unwrap()
                .id;
            tx.drop_object(doomed).unwrap();
            tx.write_row(keeper, "row", Some(b"after".to_vec()))
                .unwrap();
            if mode == "io_error" {
                assert!(matches!(tx.commit(), Err(Error::CommitUnknown(_))));
                assert!(matches!(db.with_catalog(|_| ()), Err(Error::Poisoned)));
                let recovered =
                    Database::open(std::env::var("GF_DIR").unwrap(), OpenOptions::default())
                        .unwrap();
                assert!(graph_id(&recovered, "doomed").is_none());
            } else {
                tx.commit().unwrap();
            }
        }
        "initialize" => (),
        "checkpoint" => db.checkpoint().unwrap(),
        _ => panic!("unknown worker mode"),
    }
}
fn child_barrier() {
    println!("GF_READY");
    std::io::stdout().flush().unwrap();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).unwrap();
    assert_eq!(line.trim(), "go");
}
