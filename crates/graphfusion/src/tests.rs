use super::*;
use catalog::{GraphShape, ObjectDefinition, ObjectId, ObjectKind, MAIN_SCHEMA, ROOT_DIRECTORY};
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
        .published
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
fn graph_import_persists_and_rejects_unvalidated_typed_graphs() {
    let dir = TestDir::new();
    {
        let db = dir.open();
        let mut session = db.session();
        session
            .execute("CREATE GRAPH g ANY GRAPH; SESSION SET GRAPH g")
            .unwrap();
        let seq = db.inner.state.lock().unwrap().published.commit_seq;
        assert_eq!(
            session.replace_graph_data(arrow_graph(vec![1])).unwrap(),
            seq + 1
        );
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
        1
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

#[test]
fn reopen_existing_database_when_an_ancestor_is_not_readable() {
    use std::os::unix::fs::PermissionsExt;
    let root = TestDir::new();
    let parent = root.0.join("locked");
    let nested = parent.join("db");
    fs::create_dir_all(&parent).unwrap();
    {
        let db = Database::open(
            &nested,
            OpenOptions {
                create_if_missing: true,
            },
        )
        .unwrap();
        db.session().execute("CREATE GRAPH g ANY GRAPH").unwrap();
    }
    let mut perms = fs::metadata(&parent).unwrap().permissions();
    perms.set_mode(0o311);
    fs::set_permissions(&parent, perms).unwrap();
    let readable = fs::read_dir(&parent).is_ok();
    let opened = if readable {
        Ok(())
    } else {
        Database::open(
            &nested,
            OpenOptions {
                create_if_missing: true,
            },
        )
        .map(|_| ())
    };
    let mut perms = fs::metadata(&parent).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&parent, perms).unwrap();
    if !readable {
        opened.unwrap();
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
            let before = db.inner.state.lock().unwrap().published.commit_seq;
            let committed = match session.execute(&statement) {
                Ok(_) => {
                    accepted += 1;
                    true
                }
                Err(Error::UnsupportedFeature(message)) => {
                    assert!(message.contains("recursion limit"), "{message}");
                    rejected += 1;
                    assert_eq!(db.inner.state.lock().unwrap().published.commit_seq, before);
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
fn batches_next_and_explicit_transactions() {
    let db = Database::new();
    let mut s = db.session();
    s.execute("START TRANSACTION; CREATE GRAPH a ANY GRAPH; ROLLBACK")
        .unwrap();
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
fn catalog_changes_preserve_pinned_snapshots_and_rollback() {
    for persistent in [false, true] {
        for branch in ["main", "dev"] {
            let dir = TestDir::new();
            let db = if persistent {
                dir.open()
            } else {
                Database::new()
            };
            let existing = create_graph(&db, "existing");
            if branch != "main" {
                db.create_branch(branch).unwrap();
            }
            let mut reader = StatementTxn::begin_on(&db, branch).unwrap();
            reader.set_read_only(true);
            let mut writer = StatementTxn::begin_on(&db, branch).unwrap();
            let created = writer
                .create_graph(MAIN_SCHEMA, "created", GraphShape::Open)
                .unwrap();
            writer.drop_object(existing).unwrap();
            assert!(writer.catalog().get(existing).is_none());
            assert_eq!(
                writer
                    .catalog()
                    .lookup(MAIN_SCHEMA, ObjectKind::Graph, "created")
                    .unwrap()
                    .id,
                created
            );
            assert!(writer.base.catalog.get(existing).is_some());
            assert!(writer.base.catalog.get(created).is_none());
            let unpublished = StatementTxn::begin_on(&db, branch).unwrap();
            assert!(unpublished.catalog().get(existing).is_some());
            assert!(unpublished.catalog().get(created).is_none());
            drop(unpublished);
            writer.commit().unwrap();

            assert!(reader.catalog().get(existing).is_some());
            assert!(reader.catalog().get(created).is_none());
            let current = StatementTxn::begin_on(&db, branch).unwrap();
            assert!(current.catalog().get(existing).is_none());
            assert!(current.catalog().get(created).is_some());
            drop(current);

            let mut aborted = StatementTxn::begin_on(&db, branch).unwrap();
            aborted.drop_object(created).unwrap();
            let abandoned = aborted
                .create_graph(MAIN_SCHEMA, "abandoned", GraphShape::Open)
                .unwrap();
            assert!(aborted.catalog().get(created).is_none());
            assert!(aborted.catalog().get(abandoned).is_some());
            assert!(aborted.base.catalog.get(created).is_some());
            assert!(aborted.base.catalog.get(abandoned).is_none());
            drop(aborted);
            let current = StatementTxn::begin_on(&db, branch).unwrap();
            assert!(current.catalog().get(created).is_some());
            assert!(current
                .catalog()
                .lookup(MAIN_SCHEMA, ObjectKind::Graph, "abandoned")
                .is_none());
            drop(current);
            if branch != "main" {
                db.with_catalog(|catalog| {
                    assert!(catalog.get(existing).is_some());
                    assert!(catalog.get(created).is_none());
                })
                .unwrap();
            }
            drop(reader);
            db.checkpoint().unwrap();
        }
    }
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
    for name in ["absent", "a/b.c\"\\名"] {
        let mut a = StatementTxn::begin(&db).unwrap();
        assert!(a.lookup(MAIN_SCHEMA, ObjectKind::Graph, name).is_none());
        assert!(a.lookup(MAIN_SCHEMA, ObjectKind::Graph, name).is_none());
        a.create_graph(MAIN_SCHEMA, "other", GraphShape::Open)
            .unwrap();
        let id = create_graph(&db, name);
        let mut b = StatementTxn::begin(&db).unwrap();
        assert_eq!(
            b.lookup(MAIN_SCHEMA, ObjectKind::Graph, name).unwrap().id,
            id
        );
        assert_eq!(
            b.lookup(MAIN_SCHEMA, ObjectKind::Graph, name).unwrap().id,
            id
        );
        b.drop_object(id).unwrap();
        assert!(b.lookup(MAIN_SCHEMA, ObjectKind::Graph, name).is_none());
        b.commit().unwrap();
        assert!(matches!(a.commit(), Err(Error::Conflict(_))));
    }
    let mut a = StatementTxn::begin(&db).unwrap();
    assert!(a.children(MAIN_SCHEMA).is_empty());
    a.create_graph(MAIN_SCHEMA, "after_scan", GraphShape::Open)
        .unwrap();
    create_graph(&db, "phantom");
    assert!(matches!(a.commit(), Err(Error::Conflict(_))));
}

#[test]
fn catalog_relationship_indexes_follow_moves_replacements_and_removals() {
    let mut catalog = catalog::CatalogSnapshot::initial();
    for (id, parent, definition) in [
        (10, ROOT_DIRECTORY, ObjectDefinition::Schema),
        (11, ROOT_DIRECTORY, ObjectDefinition::Schema),
        (
            20,
            MAIN_SCHEMA,
            ObjectDefinition::GraphType(types::GraphDefinition::default()),
        ),
        (
            21,
            MAIN_SCHEMA,
            ObjectDefinition::GraphType(types::GraphDefinition::default()),
        ),
        (
            32,
            10,
            ObjectDefinition::Graph {
                shape: GraphShape::Named(20),
                storage: 132,
            },
        ),
        (
            30,
            10,
            ObjectDefinition::Graph {
                shape: GraphShape::Named(20),
                storage: 130,
            },
        ),
        (
            31,
            11,
            ObjectDefinition::Graph {
                shape: GraphShape::Open,
                storage: 131,
            },
        ),
    ] {
        catalog.put(
            catalog::CatalogEntry {
                id,
                parent,
                name: format!("object_{id}"),
                version: 0,
                definition,
            },
            1,
        );
    }
    assert_catalog_relationships(&catalog);
    let pinned = catalog.clone();
    let mut moved = catalog.get(30).unwrap().clone();
    moved.parent = 11;
    moved.name = "moved".into();
    moved.definition = ObjectDefinition::Graph {
        shape: GraphShape::Named(21),
        storage: 130,
    };
    catalog.put(moved.clone(), 2);
    assert_catalog_relationships(&catalog);
    assert_eq!(
        catalog
            .children(11)
            .map(|entry| entry.id)
            .collect::<Vec<_>>(),
        vec![30, 31]
    );
    let restored = serde_json::from_slice(&serde_json::to_vec(&catalog).unwrap()).unwrap();
    assert_catalog_relationships(&restored);
    catalog.remove(32, 3);
    assert!(!catalog.has_dependents(20));
    assert_catalog_relationships(&catalog);
    moved.definition = ObjectDefinition::Graph {
        shape: GraphShape::Open,
        storage: 130,
    };
    catalog.put(moved, 4);
    assert!(!catalog.has_dependents(21));
    assert_catalog_relationships(&catalog);
    for id in [30, 31, 10, 11] {
        catalog.remove(id, 5);
        assert_catalog_relationships(&catalog);
    }
    assert_catalog_relationships(&pinned);
    assert!(pinned.has_dependents(20));
    assert_eq!(
        pinned
            .children(10)
            .map(|entry| entry.id)
            .collect::<Vec<_>>(),
        vec![30, 32]
    );
}

#[test]
fn catalog_relationship_indexes_restore_legacy_snapshots() {
    let input = include_str!("../tests/data/catalog-legacy.json");
    let mut catalog: catalog::CatalogSnapshot = serde_json::from_str(input).unwrap();
    assert_catalog_relationships(&catalog);
    let schema = catalog
        .lookup(ROOT_DIRECTORY, ObjectKind::Schema, "small")
        .unwrap()
        .id;
    let typ = catalog
        .lookup(MAIN_SCHEMA, ObjectKind::GraphType, "t")
        .unwrap()
        .id;
    let graph = catalog
        .lookup(MAIN_SCHEMA, ObjectKind::Graph, "typed")
        .unwrap()
        .id;
    assert_eq!(
        catalog
            .children(schema)
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        vec!["g"]
    );
    assert!(catalog.has_dependents(typ));
    catalog.remove(graph, 10);
    assert!(!catalog.has_dependents(typ));
    assert_catalog_relationships(&catalog);

    let mut corrupt: serde_json::Value = serde_json::from_str(input).unwrap();
    corrupt["objects"][MAIN_SCHEMA.to_string()]["id"] = ObjectId::MAX.into();
    let corrupt: catalog::CatalogSnapshot = serde_json::from_value(corrupt).unwrap();
    assert!(matches!(corrupt.validate(), Err(Error::Corrupt(_))));
}

fn assert_catalog_relationships(catalog: &catalog::CatalogSnapshot) {
    let targets: std::collections::BTreeSet<_> = catalog
        .entries()
        .flat_map(|entry| [entry.id, entry.parent])
        .chain([ObjectId::MAX])
        .collect();
    for target in targets {
        let expected: Vec<_> = catalog
            .entries()
            .filter(|entry| entry.parent == target)
            .map(|entry| entry.id)
            .collect();
        assert_eq!(
            catalog
                .children(target)
                .map(|entry| entry.id)
                .collect::<Vec<_>>(),
            expected,
            "parent {target}"
        );
        assert_eq!(
            catalog.has_children(target),
            !expected.is_empty(),
            "parent {target}"
        );
        assert_eq!(
            catalog.has_dependents(target),
            catalog
                .entries()
                .any(|entry| entry.definition.dependency() == Some(target)),
            "target {target}"
        );
    }
    catalog.validate().unwrap();
}

#[test]
fn catalog_restrictions_follow_transaction_local_deletions() {
    for persistent in [false, true] {
        for branch in ["main", "dev"] {
            let dir = TestDir::new();
            let db = if persistent {
                dir.open()
            } else {
                Database::new()
            };
            let mut setup = StatementTxn::begin(&db).unwrap();
            let schema = setup
                .create(ROOT_DIRECTORY, "scope", ObjectDefinition::Schema)
                .unwrap();
            let typ = setup
                .create(
                    MAIN_SCHEMA,
                    "t",
                    ObjectDefinition::GraphType(types::GraphDefinition::default()),
                )
                .unwrap();
            let first = setup
                .create_graph(schema, "first", GraphShape::Named(typ))
                .unwrap();
            let second = setup
                .create_graph(schema, "second", GraphShape::Named(typ))
                .unwrap();
            setup.commit().unwrap();
            if branch != "main" {
                db.create_branch(branch).unwrap();
            }
            let reader = StatementTxn::begin_on(&db, branch).unwrap();
            for commit in [false, true] {
                let mut writer = StatementTxn::begin_on(&db, branch).unwrap();
                assert!(matches!(
                    writer.drop_object(schema),
                    Err(Error::DependencyExists(_))
                ));
                assert!(matches!(
                    writer.drop_object(typ),
                    Err(Error::DependencyExists(_))
                ));
                writer.drop_object(first).unwrap();
                assert!(matches!(
                    writer.drop_object(schema),
                    Err(Error::DependencyExists(_))
                ));
                assert!(matches!(
                    writer.drop_object(typ),
                    Err(Error::DependencyExists(_))
                ));
                writer.drop_object(second).unwrap();
                assert!(!writer.catalog().has_children(schema));
                assert!(!writer.catalog().has_dependents(typ));
                writer.drop_object(typ).unwrap();
                writer.drop_object(schema).unwrap();
                assert_catalog_relationships(writer.catalog());
                if commit {
                    writer.commit().unwrap();
                } else {
                    drop(writer);
                }
                let current = StatementTxn::begin_on(&db, branch).unwrap();
                assert_eq!(current.catalog().get(schema).is_none(), commit);
                assert_eq!(current.catalog().has_dependents(typ), !commit);
                assert_eq!(
                    current.catalog().children(schema).count(),
                    if commit { 0 } else { 2 }
                );
                assert_catalog_relationships(current.catalog());
                assert_eq!(reader.catalog().children(schema).count(), 2);
                assert!(reader.catalog().has_dependents(typ));
            }
            if branch != "main" {
                db.with_catalog(|catalog| {
                    assert_eq!(catalog.children(schema).count(), 2);
                    assert!(catalog.has_dependents(typ));
                    assert_catalog_relationships(catalog);
                })
                .unwrap();
            }
            drop(reader);
            db.checkpoint().unwrap();
        }
    }
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
    assert_eq!(
        db.inner
            .state
            .lock()
            .unwrap()
            .published
            .storage
            .generations
            .len(),
        1
    );
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

struct ThreadWorker {
    thread: Option<std::thread::JoinHandle<String>>,
    release: Option<std::sync::mpsc::Sender<()>>,
}
impl ThreadWorker {
    fn start(dir: &TestDir, mode: &str, name: &str) -> Self {
        let (ready, waiting) = std::sync::mpsc::channel();
        let (release, proceed) = std::sync::mpsc::channel();
        let (path, mode, name) = (dir.0.clone(), mode.to_owned(), name.to_owned());
        let thread = std::thread::spawn(move || {
            let db = Database::open(path, OpenOptions::default()).unwrap();
            worker(db, &mode, &name, || {
                ready.send(()).unwrap();
                let _ = proceed.recv();
            })
        });
        waiting
            .recv_timeout(std::time::Duration::from_secs(20))
            .unwrap();
        Self {
            thread: Some(thread),
            release: Some(release),
        }
    }
    fn release(&mut self) {
        self.release.take().unwrap().send(()).unwrap();
    }
    fn finish(mut self) -> String {
        self.thread.take().unwrap().join().unwrap()
    }
}
impl Drop for ThreadWorker {
    fn drop(&mut self) {
        self.release.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
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
fn persistent_main_state_is_shared_across_statements_and_reservations() {
    let dir = TestDir::new();
    let db = dir.open();
    let reopened = dir.open();
    let recovery = db.statistics().unwrap().recovery_micros;
    let reader = StatementTxn::begin(&db).unwrap();
    for _ in 0..16 {
        let next = StatementTxn::begin(&reopened).unwrap();
        assert!(Arc::ptr_eq(&reader.base, &next.base));
    }
    let mut aborted = StatementTxn::begin(&reopened).unwrap();
    let aborted_id = aborted
        .create(ROOT_DIRECTORY, "aborted", ObjectDefinition::Schema)
        .unwrap();
    drop(aborted);
    let current = StatementTxn::begin(&db).unwrap();
    assert!(Arc::ptr_eq(&reader.base.catalog, &current.base.catalog));
    assert!(current.base.next_id > reader.base.next_id);
    assert!(current
        .catalog()
        .lookup(ROOT_DIRECTORY, ObjectKind::Schema, "aborted")
        .is_none());
    drop(current);
    let graph = create_graph(&reopened, "g");
    assert!(graph > aborted_id);
    assert!(reader
        .catalog()
        .lookup(MAIN_SCHEMA, ObjectKind::Graph, "g")
        .is_none());
    assert_eq!(graph_id(&db, "g"), Some(graph));
    drop(reader);
    db.create_branch("dev").unwrap();
    for value in [b"first", b"later"] {
        db.checkpoint().unwrap();
        let mut writer = StatementTxn::begin(&reopened).unwrap();
        writer
            .write_row(graph, "row", Some(value.to_vec()))
            .unwrap();
        writer.commit().unwrap();
    }
    assert_eq!(db.statistics().unwrap().recovery_micros, recovery);
    let mut branch = StatementTxn::begin_on(&db, "dev").unwrap();
    assert_eq!(branch.read_row(graph, "row").unwrap(), None);
    drop(branch);
    drop(reopened);
    drop(db);
    let db = dir.open();
    assert_eq!(graph_id(&db, "g"), Some(graph));
    let mut reader = StatementTxn::begin(&db).unwrap();
    assert_eq!(
        reader.read_row(graph, "row").unwrap(),
        Some(b"later".to_vec())
    );
}

#[test]
fn failed_main_snapshot_publication_recovers_before_commit_and_branch_creation() {
    let dir = TestDir::new();
    let db = dir.open();
    create_graph(&db, "g");
    let reader = StatementTxn::begin(&db).unwrap();
    let mut writer = StatementTxn::begin(&db).unwrap();
    let schema = writer
        .create(ROOT_DIRECTORY, "durable", ObjectDefinition::Schema)
        .unwrap();
    let mut stale = StatementTxn::begin(&db).unwrap();
    stale
        .create(ROOT_DIRECTORY, "durable", ObjectDefinition::Schema)
        .unwrap();
    // Make secondary snapshot publication fail after the canonical WAL commit.
    let main = dir.0.join("refs/main");
    let saved = dir.0.join("refs/main.saved");
    fs::rename(&main, &saved).unwrap();
    fs::create_dir(&main).unwrap();
    assert!(matches!(writer.commit(), Err(Error::Io(_))));
    fs::remove_dir(&main).unwrap();
    fs::rename(&saved, &main).unwrap();
    assert!(matches!(stale.commit(), Err(Error::Conflict(_))));
    assert!(reader
        .catalog()
        .lookup(ROOT_DIRECTORY, ObjectKind::Schema, "durable")
        .is_none());
    drop(reader);
    db.with_catalog(|catalog| {
        assert_eq!(
            catalog
                .lookup(ROOT_DIRECTORY, ObjectKind::Schema, "durable")
                .unwrap()
                .id,
            schema
        );
    })
    .unwrap();
    db.create_branch("dev").unwrap();
    let branch = StatementTxn::begin_on(&db, "dev").unwrap();
    assert_eq!(
        branch
            .catalog()
            .lookup(ROOT_DIRECTORY, ObjectKind::Schema, "durable")
            .unwrap()
            .id,
        schema
    );
    drop(branch);
    create_graph(&db, "after");
    db.checkpoint().unwrap();
    drop(db);
    let db = dir.open();
    assert!(graph_id(&db, "after").is_some());
    db.with_catalog(|catalog| {
        assert_eq!(
            catalog
                .lookup(ROOT_DIRECTORY, ObjectKind::Schema, "durable")
                .unwrap()
                .id,
            schema
        );
    })
    .unwrap();
}

#[test]
fn failed_checkpoint_recovers_the_selected_wal_generation() {
    for switched in [false, true] {
        let dir = TestDir::new();
        let db = dir.open();
        let (_, keeper) = seed(&db);
        let blocker = if switched {
            // Reclamation reads branch refs after MANIFEST has selected the new WAL.
            let path = dir.0.join("refs/broken");
            fs::write(&path, b"broken").unwrap();
            path
        } else {
            // Interrupt snapshot installation before MANIFEST can switch generations.
            let path = dir.0.join("data-1.snapshot");
            fs::create_dir(&path).unwrap();
            path
        };
        assert!(db.checkpoint().is_err());
        assert_eq!(dir.0.join("wal-1.log").exists(), switched);
        if switched {
            fs::remove_file(blocker).unwrap();
            let catalog = dir.0.join("catalog-1.snapshot");
            let saved = dir.0.join("catalog-1.saved");
            fs::rename(&catalog, &saved).unwrap();
            assert!(matches!(db.with_catalog(|_| ()), Err(Error::Io(_))));
            fs::rename(saved, catalog).unwrap();
        } else {
            fs::remove_dir(blocker).unwrap();
        }
        create_graph(&db, "after");
        db.checkpoint().unwrap();
        drop(db);
        let db = dir.open();
        assert!(graph_id(&db, "after").is_some());
        let mut reader = StatementTxn::begin(&db).unwrap();
        assert_eq!(
            reader.read_row(keeper, "row").unwrap(),
            Some(b"before".to_vec())
        );
    }
}

#[test]
fn multithread_writers_share_state_and_conflict() {
    let dir = TestDir::new();
    let db = dir.open();
    let mut a = ThreadWorker::start(&dir, "create", "a");
    let mut b = ThreadWorker::start(&dir, "create", "b");
    a.release();
    b.release();
    assert!(a.finish().contains("GF_COMMITTED"));
    assert!(b.finish().contains("GF_COMMITTED"));
    assert!(graph_id(&db, "a").is_some());
    assert!(graph_id(&db, "b").is_some());
    let mut a = ThreadWorker::start(&dir, "create", "same");
    let mut b = ThreadWorker::start(&dir, "create", "same");
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
fn database_ownership_includes_idle_sessions_and_path_aliases() {
    let dir = TestDir::new();
    let db = dir.open();
    let alias = TestDir::new();
    fs::remove_dir(&alias.0).unwrap();
    std::os::unix::fs::symlink(&dir.0, &alias.0).unwrap();
    let reopened = Database::open(&alias.0, OpenOptions::default()).unwrap();
    assert!(Arc::ptr_eq(&db.inner, &reopened.inner));
    let session = db.session();
    drop(db);
    drop(reopened);
    // Even an idle session owns the directory after all explicit Database handles drop.
    let rejected = child_command(&dir, "expect_in_use").output().unwrap();
    assert!(
        rejected.status.success(),
        "{}",
        String::from_utf8_lossy(&rejected.stderr)
    );
    drop(session);
    let mut owner = Worker::start(&dir, "hold", "");
    assert!(matches!(
        Database::open(&dir.0, OpenOptions::default()),
        Err(Error::DatabaseInUse(_))
    ));
    assert!(matches!(
        Database::open(&alias.0, OpenOptions::default()),
        Err(Error::DatabaseInUse(_))
    ));
    owner.release();
    owner.finish();
    // Exiting normally releases the OS lock; the lock file itself remains reusable.
    dir.open().checkpoint().unwrap();
    std::fs::remove_file(&alias.0).unwrap();
}

#[test]
fn concurrent_open_and_last_handle_drop_never_reject_this_process() {
    let dir = TestDir::new();
    drop(dir.open());
    std::thread::scope(|scope| {
        for _ in 0..8 {
            let path = &dir.0;
            scope.spawn(move || {
                for _ in 0..100 {
                    let db = Database::open(path, OpenOptions::default()).unwrap();
                    std::thread::yield_now();
                    drop(db);
                }
            });
        }
    });
}

#[test]
fn thread_reader_pins_data_and_process_kill_releases_ownership() {
    let dir = TestDir::new();
    let db = dir.open();
    let (doomed, _) = seed(&db);
    let mut reader = ThreadWorker::start(&dir, "reader", "");
    db.session().execute("DROP GRAPH doomed").unwrap();
    assert!(matches!(db.checkpoint(), Err(Error::Busy)));
    reader.release();
    assert!(reader.finish().contains("GF_OLD_OK"));
    db.checkpoint().unwrap();
    assert!(graph_id(&db, "doomed").is_none());
    drop(db);
    let mut writer = Worker::start(&dir, "create", "uncommitted");
    assert!(matches!(
        Database::open(&dir.0, OpenOptions::default()),
        Err(Error::DatabaseInUse(_))
    ));
    writer.child.kill().unwrap();
    writer.child.wait().unwrap();
    let db = dir.open();
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
        drop(db);
        let result = child_command(&dir, "drop_and_write")
            .env("GRAPHFUSION_TEST_CRASH", point)
            .output()
            .unwrap();
        let db = dir.open();
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
        drop(db);
        let result = child_command(&dir, "checkpoint")
            .env("GRAPHFUSION_TEST_CRASH", point)
            .output()
            .unwrap();
        let db = dir.open();
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
    drop(db);
    let db = dir.open();
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
    drop(db);
    assert!(matches!(
        Database::open(&dir.0, OpenOptions::default()),
        Err(Error::Corrupt(_))
    ));
}

#[test]
fn corrupted_checkpoint_never_falls_back_to_retired_files() {
    let dir = TestDir::new();
    let db = dir.open();
    seed(&db);
    db.checkpoint().unwrap();
    db.session().execute("DROP GRAPH doomed").unwrap();
    db.checkpoint().unwrap();
    drop(db);
    File::create(dir.0.join("catalog-2.snapshot"))
        .unwrap()
        .write_all(b"broken")
        .unwrap();
    assert!(matches!(
        Database::open(&dir.0, OpenOptions::default()),
        Err(Error::Corrupt(_))
    ));
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
        let other = b.open();
        drop(db);
        drop(other);
        fs::copy(b.0.join(file), a.0.join(file)).unwrap();
        assert!(
            matches!(
                Database::open(&a.0, OpenOptions::default()),
                Err(Error::Corrupt(_))
            ),
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
    drop(db);
    let result = child_command(&dir, "io_error")
        .env("GRAPHFUSION_TEST_IO", "wal_sync")
        .output()
        .unwrap();
    let db = dir.open();
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
    if mode == "expect_in_use" {
        assert!(matches!(db, Err(Error::DatabaseInUse(_))), "{db:?}");
        return;
    }
    println!(
        "{}",
        worker(
            db.unwrap(),
            &mode,
            &std::env::var("GF_NAME").unwrap_or_default(),
            child_barrier
        )
    );
}

fn worker(db: Database, mode: &str, name: &str, barrier: impl FnOnce()) -> String {
    let mut outcome = String::new();
    match mode {
        "recovery_write" => {
            create_graph(&db, "after_recovery");
        }
        "create" => {
            let mut tx = StatementTxn::begin(&db).unwrap();
            tx.create_graph(MAIN_SCHEMA, name, GraphShape::Open)
                .unwrap();
            barrier();
            match tx.commit() {
                Ok(_) => outcome.push_str("GF_COMMITTED"),
                Err(Error::Conflict(_)) => outcome.push_str("GF_CONFLICT"),
                Err(e) => panic!("{e}"),
            }
        }
        "reader" => {
            let mut tx = StatementTxn::begin(&db).unwrap();
            let graph = tx
                .lookup(MAIN_SCHEMA, ObjectKind::Graph, "doomed")
                .unwrap()
                .id;
            barrier();
            assert_eq!(
                tx.read_row(graph, "row").unwrap(),
                Some(b"old graph data".to_vec())
            );
            outcome.push_str("GF_OLD_OK");
        }
        "parquet_import" => {
            let mut tx = StatementTxn::begin(&db).unwrap();
            let graph = tx.lookup(MAIN_SCHEMA, ObjectKind::Graph, "g").unwrap().id;
            let doomed = tx
                .lookup(MAIN_SCHEMA, ObjectKind::Graph, "doomed")
                .unwrap()
                .id;
            tx.drop_object(doomed).unwrap();
            tx.replace_graph_data(graph, arrow_graph(vec![2, 3]))
                .unwrap();
            tx.commit().unwrap();
        }
        "explicit_writer" => {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            let mut s = db.session();
            s.execute("SESSION SET GRAPH g; START TRANSACTION").unwrap();
            let value = name.parse::<i64>().unwrap();
            s.set_parameter("v", Value::Integer(value)).unwrap();
            runtime.block_on(s.run("INSERT (:N {v: $v})")).unwrap();
            s.execute(&format!("CREATE GRAPH marker{value} ANY GRAPH"))
                .unwrap();
            barrier();
            match s.execute("COMMIT") {
                Ok(_) => outcome.push_str("GF_COMMITTED"),
                Err(Error::Conflict(_)) => {
                    assert!(matches!(
                        s.transaction_status(),
                        TransactionStatus::Failed { .. }
                    ));
                    s.execute("ROLLBACK").unwrap();
                    outcome.push_str("GF_CONFLICT");
                }
                Err(e) => panic!("{e}"),
            }
        }
        "explicit_reader" => {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            let mut s = db.session();
            s.execute("SESSION SET GRAPH g; START TRANSACTION READ ONLY")
                .unwrap();
            assert_eq!(
                runtime
                    .block_on(s.query("MATCH (n) RETURN ELEMENT_ID(n) AS id"))
                    .unwrap()
                    .row_count(),
                1
            );
            barrier();
            assert_eq!(
                runtime
                    .block_on(s.query("MATCH (n) RETURN ELEMENT_ID(n) AS id"))
                    .unwrap()
                    .row_count(),
                1
            );
            s.execute("COMMIT").unwrap();
            assert_eq!(
                runtime
                    .block_on(s.query("MATCH (n) RETURN ELEMENT_ID(n) AS id"))
                    .unwrap()
                    .row_count(),
                2
            );
            outcome.push_str("GF_OLD_OK");
        }
        "explicit_crash" | "explicit_io" => {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            let mut s = db.session();
            s.execute("SESSION SET GRAPH g; START TRANSACTION").unwrap();
            runtime.block_on(s.run("INSERT (:N {v: 2}); CREATE GRAPH atomic_marker ANY GRAPH; INSERT (:N {v: 3}); DROP GRAPH doomed; CREATE GRAPH second ANY GRAPH; USE GRAPH second INSERT (:N {v: 9})")).unwrap();
            if mode == "explicit_io" {
                let mut reader = db.session();
                reader.execute("START TRANSACTION READ ONLY").unwrap();
                assert!(matches!(s.execute("COMMIT"), Err(Error::CommitUnknown(_))));
                assert!(matches!(
                    s.transaction_status(),
                    TransactionStatus::Failed { .. }
                ));
                assert!(matches!(db.with_catalog(|_| ()), Err(Error::Poisoned)));
                assert!(matches!(
                    runtime.block_on(reader.query("RETURN 1 AS n")),
                    Err(Error::Poisoned)
                ));
                assert!(matches!(
                    reader.transaction_status(),
                    TransactionStatus::Failed { .. }
                ));
                reader.execute("ROLLBACK").unwrap();
                s.execute("ROLLBACK").unwrap();
                assert!(matches!(
                    s.execute("START TRANSACTION"),
                    Err(Error::Poisoned)
                ));
            } else {
                s.execute("COMMIT").unwrap();
            }
        }
        "gql_write" => {
            let mut s = db.session();
            s.execute("SESSION SET GRAPH g").unwrap();
            tokio::runtime::Runtime::new()
                .unwrap()
                .block_on(s.run("MATCH (a:N) INSERT (a)-[:E]->(b:N {v: 2}) SET b.v = 3"))
                .unwrap();
        }
        "parquet_reader" => {
            let mut tx = StatementTxn::begin(&db).unwrap();
            let graph = tx.lookup(MAIN_SCHEMA, ObjectKind::Graph, "g").unwrap().id;
            let data = tx.graph_data(graph).unwrap();
            barrier();
            let runtime = tokio::runtime::Runtime::new().unwrap();
            let batches = runtime.block_on(async {
                datafusion::prelude::SessionContext::new()
                    .read_table(data.nodes[0].0.provider.clone())
                    .unwrap()
                    .collect()
                    .await
                    .unwrap()
            });
            let ids = batches
                .iter()
                .flat_map(|batch| {
                    batch
                        .column(0)
                        .as_any()
                        .downcast_ref::<arrow::array::UInt64Array>()
                        .unwrap()
                        .values()
                        .iter()
                        .copied()
                })
                .collect::<Vec<_>>();
            assert_eq!(ids, [1]);
            outcome.push_str("GF_OLD_OK");
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
                assert!(matches!(
                    Database::open(std::env::var("GF_DIR").unwrap(), OpenOptions::default()),
                    Err(Error::Poisoned)
                ));
                drop(db);
                let recovered =
                    Database::open(std::env::var("GF_DIR").unwrap(), OpenOptions::default())
                        .unwrap();
                assert!(graph_id(&recovered, "doomed").is_none());
            } else {
                tx.commit().unwrap();
            }
        }
        "hold" => barrier(),
        "initialize" => (),
        "checkpoint" => db.checkpoint().unwrap(),
        _ => panic!("unknown worker mode"),
    }
    outcome
}

fn parquet_files(dir: &TestDir) -> Vec<PathBuf> {
    fs::read_dir(&dir.0)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|s| s == "parquet"))
        .collect()
}

#[test]
fn parquet_import_rejects_unscannable_paths_before_staging_files() {
    let root = TestDir::new();
    let dir = TestDir(root.0.join("control\n"));
    let db = dir.open();
    let mut session = db.session();
    session
        .execute("CREATE GRAPH g ANY GRAPH; SESSION SET GRAPH g")
        .unwrap();
    assert!(matches!(
        session.replace_graph_data(arrow_graph(vec![1])),
        Err(Error::UnsupportedFeature(_))
    ));
    assert!(parquet_files(&dir).is_empty());
    let id = graph_id(&db, "g").unwrap();
    assert_eq!(
        StatementTxn::begin(&db)
            .unwrap()
            .graph_data(id)
            .unwrap()
            .node_count(),
        0
    );
    db.checkpoint().unwrap();
}

#[tokio::test]
async fn parquet_crash_matrix_atomically_publishes_catalog_and_graph() {
    for (point, committed) in [
        ("parquet_write", false),
        ("parquet_sync", false),
        ("parquet_directory", false),
        ("parquet_wal_header", false),
        ("parquet_wal_payload", false),
        ("parquet_wal_commit", true),
        ("parquet_wal_sync", true),
        ("before_publish", true),
    ] {
        let dir = TestDir::new();
        let db = dir.open();
        let mut session = db.session();
        session
            .execute("CREATE GRAPH g ANY GRAPH; CREATE GRAPH doomed ANY GRAPH; SESSION SET GRAPH g")
            .unwrap();
        session.replace_graph_data(arrow_graph(vec![1])).unwrap();
        db.checkpoint().unwrap();
        drop(session);
        drop(db);
        let result = child_command(&dir, "parquet_import")
            .env("GRAPHFUSION_TEST_CRASH", point)
            .output()
            .unwrap();
        let db = dir.open();
        let mut session = db.session();
        session.execute("SESSION SET GRAPH g").unwrap();
        assert_eq!(
            result.status.code(),
            Some(86),
            "{point}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(graph_id(&db, "doomed").is_none(), committed, "{point}");
        let result = session
            .query("MATCH (n:N) RETURN ELEMENT_ID(n) AS id ORDER BY id")
            .await
            .unwrap();
        assert_eq!(result.row_count(), if committed { 2 } else { 1 }, "{point}");
        assert!(
            result.physical_plan.contains("file_type=parquet"),
            "{}",
            result.physical_plan
        );
        db.checkpoint().unwrap();
        assert_eq!(parquet_files(&dir).len(), 1);
    }
}

#[test]
fn parquet_reader_in_another_thread_pins_replaced_and_dropped_files() {
    let dir = TestDir::new();
    let db = dir.open();
    let mut session = db.session();
    session
        .execute("CREATE GRAPH g ANY GRAPH; SESSION SET GRAPH g")
        .unwrap();
    session.replace_graph_data(arrow_graph(vec![1])).unwrap();
    let mut reader = ThreadWorker::start(&dir, "parquet_reader", "");
    session.replace_graph_data(arrow_graph(vec![2, 3])).unwrap();
    session.execute("DROP GRAPH g").unwrap();
    assert_eq!(parquet_files(&dir).len(), 2);
    assert!(matches!(db.checkpoint(), Err(Error::Busy)));
    reader.release();
    assert!(reader.finish().contains("GF_OLD_OK"));
    db.checkpoint().unwrap();
    assert!(parquet_files(&dir).is_empty());
}

#[tokio::test]
async fn parquet_checkpoint_crash_matrix_and_conflict_orphan_reclamation() {
    for point in [
        "checkpoint_catalog",
        "checkpoint_data",
        "manifest_rename",
        "manifest_sync",
        "cleanup",
        "parquet_cleanup",
    ] {
        let dir = TestDir::new();
        let db = dir.open();
        let id = create_graph(&db, "g");
        let mut a = StatementTxn::begin(&db).unwrap();
        let mut b = StatementTxn::begin(&db).unwrap();
        a.replace_graph_data(id, arrow_graph(vec![1])).unwrap();
        b.replace_graph_data(id, arrow_graph(vec![2, 3])).unwrap();
        a.commit().unwrap();
        assert!(matches!(b.commit(), Err(Error::Conflict(_))));
        assert_eq!(parquet_files(&dir).len(), 2);
        drop(db);
        let result = child_command(&dir, "checkpoint")
            .env("GRAPHFUSION_TEST_CRASH", point)
            .output()
            .unwrap();
        let db = dir.open();
        assert_eq!(
            result.status.code(),
            Some(86),
            "{point}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let result = db
            .session()
            .query("USE GRAPH g MATCH (n) RETURN ELEMENT_ID(n) AS id")
            .await
            .unwrap();
        assert_eq!(result.row_count(), 1);
        db.checkpoint().unwrap();
        assert_eq!(parquet_files(&dir).len(), 1);
    }
}

#[test]
fn missing_truncated_and_substituted_parquet_files_fail_closed() {
    for change in ["missing", "truncated", "schema"] {
        let dir = TestDir::new();
        {
            let db = dir.open();
            let id = create_graph(&db, "g");
            let mut tx = StatementTxn::begin(&db).unwrap();
            tx.replace_graph_data(id, arrow_graph(vec![1])).unwrap();
            tx.commit().unwrap();
            db.checkpoint().unwrap();
        }
        let path = &parquet_files(&dir)[0];
        match change {
            "missing" => fs::remove_file(path).unwrap(),
            "truncated" => FileOptions::new()
                .write(true)
                .open(path)
                .unwrap()
                .set_len(12)
                .unwrap(),
            _ => {
                let mut file = FileOptions::new().write(true).open(path).unwrap();
                let len = file.metadata().unwrap().len();
                file.seek(SeekFrom::Start(len - 4)).unwrap();
                file.write_all(b"BAD!").unwrap();
            }
        }
        assert!(
            matches!(
                Database::open(&dir.0, OpenOptions::default()),
                Err(Error::Corrupt(_))
            ),
            "{change}"
        );
    }
}

fn child_barrier() {
    println!("GF_READY");
    std::io::stdout().flush().unwrap();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).unwrap();
    assert_eq!(line.trim(), "go");
}

#[test]
fn element_allocation_conflicts_and_graph_read_dependencies_are_validated() {
    let db = Database::new();
    let a = create_graph(&db, "a");
    let b = create_graph(&db, "b");
    let mut first = StatementTxn::begin(&db).unwrap();
    let mut second = StatementTxn::begin(&db).unwrap();
    let ids = first.allocate_element_ids(a, 2).unwrap();
    assert_eq!(ids, second.allocate_element_ids(a, 2).unwrap());
    first.replace_graph_data(a, arrow_graph(ids)).unwrap();
    first.commit().unwrap();
    second
        .replace_graph_data(a, arrow_graph(vec![0, 1]))
        .unwrap();
    assert!(matches!(second.commit(), Err(Error::Conflict(_))));
    let mut reader_writer = StatementTxn::begin(&db).unwrap();
    reader_writer.graph_data(a).unwrap();
    reader_writer
        .replace_graph_data(b, arrow_graph(vec![8]))
        .unwrap();
    let mut change = StatementTxn::begin(&db).unwrap();
    change.replace_graph_data(a, arrow_graph(vec![7])).unwrap();
    change.commit().unwrap();
    assert!(matches!(reader_writer.commit(), Err(Error::Conflict(_))));
    let mut next = StatementTxn::begin(&db).unwrap();
    assert_eq!(next.allocate_element_ids(a, 1).unwrap(), vec![8]);
}

#[tokio::test]
async fn imported_maximum_element_id_exhausts_allocator_without_wrapping() {
    let dir = TestDir::new();
    {
        let db = dir.open();
        let mut s = db.session();
        s.execute("CREATE GRAPH g ANY GRAPH; SESSION SET GRAPH g")
            .unwrap();
        s.replace_graph_data(arrow_graph(vec![u64::MAX])).unwrap();
        s.run("MATCH (n) DELETE n").await.unwrap();
        db.checkpoint().unwrap();
    }
    let db = dir.open();
    let mut s = db.session();
    s.execute("SESSION SET GRAPH g").unwrap();
    assert!(matches!(
        s.run("INSERT (:N)").await,
        Err(Error::InvalidDefinition(_))
    ));
    assert_eq!(
        s.query("MATCH (n) RETURN ELEMENT_ID(n) AS id")
            .await
            .unwrap()
            .row_count(),
        0
    );
}

#[tokio::test]
async fn gql_write_crashes_recover_graph_and_identity_counter_together() {
    for (point, committed) in [
        ("parquet_directory", false),
        ("parquet_wal_payload", false),
        ("parquet_wal_commit", true),
        ("before_publish", true),
    ] {
        let dir = TestDir::new();
        let db = dir.open();
        let mut s = db.session();
        s.execute("CREATE GRAPH g ANY GRAPH; SESSION SET GRAPH g")
            .unwrap();
        s.replace_graph_data(arrow_graph(vec![1])).unwrap();
        db.checkpoint().unwrap();
        drop(s);
        drop(db);
        let child = child_command(&dir, "gql_write")
            .env("GRAPHFUSION_TEST_CRASH", point)
            .output()
            .unwrap();
        let db = dir.open();
        let mut s = db.session();
        s.execute("SESSION SET GRAPH g").unwrap();
        assert_eq!(
            child.status.code(),
            Some(86),
            "{point}: {}",
            String::from_utf8_lossy(&child.stderr)
        );
        assert_eq!(
            s.query("MATCH (n) RETURN ELEMENT_ID(n) AS id")
                .await
                .unwrap()
                .row_count(),
            if committed { 2 } else { 1 }
        );
        let result = s
            .query("MATCH ()-[e:E]->(b) RETURN b.v AS v")
            .await
            .unwrap();
        assert_eq!(result.row_count(), usize::from(committed));
        if committed {
            assert_eq!(
                result.batches[0]
                    .column(0)
                    .as_any()
                    .downcast_ref::<arrow::array::Int64Array>()
                    .unwrap()
                    .value(0),
                3
            );
        }
        let outputs = s
            .run("INSERT (n:N) RETURN ELEMENT_ID(n) AS id")
            .await
            .unwrap();
        let StatementOutput::Query(result) = &outputs[0] else {
            panic!("query output")
        };
        let id = result.batches[0]
            .column(0)
            .as_any()
            .downcast_ref::<arrow::array::StringArray>()
            .unwrap()
            .value(0);
        assert!(
            id.ends_with(if committed { ":4" } else { ":2" }),
            "{point}: {id}"
        );
        db.checkpoint().unwrap();
    }
}

#[tokio::test]
async fn cancelling_pending_datafusion_scan_aborts_explicit_transaction() {
    use datafusion::{
        catalog::{Session as DfSession, TableProvider},
        logical_expr::{Expr, TableType},
        physical_plan::ExecutionPlan,
    };
    #[derive(Debug)]
    struct PausedScan {
        schema: arrow::datatypes::SchemaRef,
        entered: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    }
    #[async_trait::async_trait]
    impl TableProvider for PausedScan {
        fn schema(&self) -> arrow::datatypes::SchemaRef {
            self.schema.clone()
        }
        fn table_type(&self) -> TableType {
            TableType::Base
        }
        async fn scan(
            &self,
            _state: &dyn DfSession,
            _projection: Option<&Vec<usize>>,
            _filters: &[Expr],
            _limit: Option<usize>,
        ) -> datafusion::common::Result<Arc<dyn ExecutionPlan>> {
            self.entered
                .lock()
                .unwrap()
                .take()
                .unwrap()
                .send(())
                .unwrap();
            std::future::pending().await
        }
    }
    let db = Database::new();
    let mut s = db.session();
    s.execute("CREATE GRAPH paused ANY GRAPH; CREATE GRAPH g ANY GRAPH; SESSION SET GRAPH paused")
        .unwrap();
    let (signal, entered) = tokio::sync::oneshot::channel();
    let mut graph = arrow_graph(vec![1]);
    graph.nodes[0].0.provider = Arc::new(PausedScan {
        schema: graph.nodes[0].0.schema.clone(),
        entered: Mutex::new(Some(signal)),
    });
    s.replace_graph_data(graph).unwrap();
    s.run("SESSION SET GRAPH g; START TRANSACTION; INSERT (:N {v: 2}); CREATE GRAPH cancelled ANY GRAPH").await.unwrap();
    let mut query = Box::pin(s.query("USE GRAPH paused MATCH (n) RETURN ELEMENT_ID(n) AS id"));
    tokio::select! {
        _ = entered => (),
        result = &mut query => panic!("paused scan returned: {result:?}"),
    }
    drop(query);
    assert!(matches!(
        s.transaction_status(),
        TransactionStatus::Failed { .. }
    ));
    assert!(graph_id(&db, "cancelled").is_none());
    db.checkpoint().unwrap();
    assert!(matches!(s.execute("COMMIT"), Err(Error::TransactionFailed)));
    s.execute("ROLLBACK").unwrap();
    assert_eq!(
        s.query("MATCH (n) RETURN ELEMENT_ID(n) AS id")
            .await
            .unwrap()
            .row_count(),
        0
    );
}

#[tokio::test]
async fn explicit_transactions_isolate_and_conflict_across_threads() {
    let dir = TestDir::new();
    let db = dir.open();
    let mut s = db.session();
    s.run("CREATE GRAPH g ANY GRAPH; SESSION SET GRAPH g; INSERT (:N {v: 1})")
        .await
        .unwrap();
    let mut a = ThreadWorker::start(&dir, "explicit_writer", "2");
    let mut b = ThreadWorker::start(&dir, "explicit_writer", "3");
    assert_eq!(
        s.query("MATCH (n) RETURN n.v AS v")
            .await
            .unwrap()
            .row_count(),
        1
    );
    assert!(graph_id(&db, "marker2").is_none());
    assert!(graph_id(&db, "marker3").is_none());
    assert!(matches!(db.checkpoint(), Err(Error::Busy)));
    a.release();
    assert!(a.finish().contains("GF_COMMITTED"));
    b.release();
    assert!(b.finish().contains("GF_CONFLICT"));
    assert!(graph_id(&db, "marker2").is_some());
    assert!(graph_id(&db, "marker3").is_none());
    assert_eq!(
        s.query("MATCH (n) RETURN n.v AS v")
            .await
            .unwrap()
            .row_count(),
        2
    );
    db.checkpoint().unwrap();
    let mut abandoned = db.session();
    abandoned.run("SESSION SET GRAPH g; START TRANSACTION; INSERT (:N {v: 4}); CREATE GRAPH marker4 ANY GRAPH").await.unwrap();
    drop(abandoned);
    assert!(graph_id(&db, "marker4").is_none());
    assert_eq!(
        s.query("MATCH (n) RETURN n.v AS v")
            .await
            .unwrap()
            .row_count(),
        2
    );
    db.checkpoint().unwrap();
}

#[test]
fn explicit_reader_pins_old_parquet_files_across_thread_replacement() {
    let dir = TestDir::new();
    let db = dir.open();
    let mut s = db.session();
    s.execute("CREATE GRAPH g ANY GRAPH; SESSION SET GRAPH g")
        .unwrap();
    s.replace_graph_data(arrow_graph(vec![1])).unwrap();
    db.checkpoint().unwrap();
    let mut reader = ThreadWorker::start(&dir, "explicit_reader", "");
    s.replace_graph_data(arrow_graph(vec![2, 3])).unwrap();
    assert!(matches!(db.checkpoint(), Err(Error::Busy)));
    reader.release();
    assert!(reader.finish().contains("GF_OLD_OK"));
    db.checkpoint().unwrap();
}

#[tokio::test]
async fn explicit_transaction_crashes_never_split_statements_or_graphs() {
    for (point, committed) in [
        ("parquet_directory", false),
        ("parquet_wal_header", false),
        ("parquet_wal_payload", false),
        ("parquet_wal_commit", true),
        ("parquet_wal_sync", true),
        ("before_publish", true),
        ("io", true),
    ] {
        let dir = TestDir::new();
        let db = dir.open();
        let mut s = db.session();
        s.execute("CREATE GRAPH g ANY GRAPH; CREATE GRAPH doomed ANY GRAPH; SESSION SET GRAPH g")
            .unwrap();
        s.replace_graph_data(arrow_graph(vec![1])).unwrap();
        db.checkpoint().unwrap();
        drop(s);
        drop(db);
        let mut command = child_command(
            &dir,
            if point == "io" {
                "explicit_io"
            } else {
                "explicit_crash"
            },
        );
        if point == "io" {
            command.env("GRAPHFUSION_TEST_IO", "parquet_wal_sync");
        } else {
            command.env("GRAPHFUSION_TEST_CRASH", point);
        }
        let child = command.output().unwrap();
        let db = dir.open();
        let mut s = db.session();
        s.execute("SESSION SET GRAPH g").unwrap();
        assert_eq!(
            child.status.code(),
            Some(if point == "io" { 0 } else { 86 }),
            "{point}: {}",
            String::from_utf8_lossy(&child.stderr)
        );
        assert_eq!(
            graph_id(&db, "atomic_marker").is_some(),
            committed,
            "{point}"
        );
        assert_eq!(graph_id(&db, "doomed").is_none(), committed, "{point}");
        assert_eq!(graph_id(&db, "second").is_some(), committed, "{point}");
        assert_eq!(
            s.query("MATCH (n) RETURN ELEMENT_ID(n) AS id")
                .await
                .unwrap()
                .row_count(),
            if committed { 3 } else { 1 },
            "{point}"
        );
        if committed {
            assert_eq!(
                s.query("USE GRAPH second MATCH (n) RETURN n.v AS v")
                    .await
                    .unwrap()
                    .row_count(),
                1
            );
        }
        let outputs = s
            .run("INSERT (n:N) RETURN ELEMENT_ID(n) AS id")
            .await
            .unwrap();
        let StatementOutput::Query(result) = &outputs[0] else {
            panic!()
        };
        let id = result.batches[0]
            .column(0)
            .as_any()
            .downcast_ref::<arrow::array::StringArray>()
            .unwrap()
            .value(0);
        assert!(
            id.ends_with(if committed { ":4" } else { ":2" }),
            "{point}: {id}"
        );
        db.checkpoint().unwrap();
    }
}

#[tokio::test]
async fn branches_diverge_and_survive_checkpoint() {
    let dir = TestDir::new();
    let db = dir.open();
    let mut main = db.session();
    main.run("CREATE GRAPH g ANY GRAPH; SESSION SET GRAPH g; INSERT (:N {v: 1})")
        .await
        .unwrap();
    let fork = db.create_branch("dev").unwrap();
    assert!(fork >= 1);
    assert!(dir.0.join("refs").join("dev").is_file());
    assert!(dir.0.join("commits").join(format!("{fork}.snap")).is_file());
    main.run("INSERT (:N {v: 2})").await.unwrap();
    let mut dev = db.session();
    dev.set_branch("dev").unwrap();
    dev.execute("SESSION SET GRAPH g").unwrap();
    assert_eq!(
        property_values(&dev.query("MATCH (n) RETURN n.v AS v").await.unwrap()),
        vec![1]
    );
    dev.run("INSERT (:N {v: 3})").await.unwrap();
    assert_eq!(
        property_values(&dev.query("MATCH (n) RETURN n.v AS v").await.unwrap()),
        vec![1, 3]
    );
    assert_eq!(
        property_values(&main.query("MATCH (n) RETURN n.v AS v").await.unwrap()),
        vec![1, 2]
    );
    let mut stalled = db.session();
    stalled.set_branch("dev").unwrap();
    stalled.execute("SESSION SET GRAPH g").unwrap();
    stalled.execute("START TRANSACTION").unwrap();
    stalled.run("INSERT (:N {v: 9})").await.unwrap();
    dev.run("INSERT (:N {v: 4})").await.unwrap();
    assert!(matches!(stalled.execute("COMMIT"), Err(Error::Conflict(_))));
    db.checkpoint().unwrap();
    assert_eq!(
        property_values(&dev.query("MATCH (n) RETURN n.v AS v").await.unwrap()),
        vec![1, 3, 4]
    );
    drop(main);
    drop(dev);
    drop(stalled);
    drop(db);
    let db = dir.open();
    let mut dev = db.session();
    dev.set_branch("dev").unwrap();
    dev.execute("SESSION SET GRAPH g").unwrap();
    assert_eq!(
        property_values(&dev.query("MATCH (n) RETURN n.v AS v").await.unwrap()),
        vec![1, 3, 4]
    );
    let mut main = db.session();
    main.execute("SESSION SET GRAPH g").unwrap();
    assert_eq!(
        property_values(&main.query("MATCH (n) RETURN n.v AS v").await.unwrap()),
        vec![1, 2]
    );
    let names: Vec<_> = db
        .branches()
        .unwrap()
        .into_iter()
        .map(|branch| branch.name)
        .collect();
    assert_eq!(names, vec!["dev".to_owned(), "main".to_owned()]);
}

fn property_values(result: &QueryResult) -> Vec<i64> {
    use arrow::array::Int64Array;
    let mut values = Vec::new();
    for batch in &result.batches {
        let column = batch
            .column(0)
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap();
        for row in 0..column.len() {
            values.push(column.value(row));
        }
    }
    values.sort_unstable();
    values
}
