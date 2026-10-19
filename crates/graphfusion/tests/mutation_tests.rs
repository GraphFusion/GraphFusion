use datafusion::common::ScalarValue;
use graphfusion::{Database, Error, OpenOptions, QueryResult, Session, StatementOutput};
use std::path::PathBuf;

struct Durable(PathBuf);
impl Durable {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        Self(std::env::temp_dir().join(format!(
                "graphfusion-writes-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            )))
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
impl Drop for Durable {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn session(db: &Database) -> Session {
    let mut s = db.session();
    s.execute("CREATE GRAPH g ANY GRAPH; SESSION SET GRAPH g")
        .unwrap();
    s
}
async fn run(s: &mut Session, query: &str) -> QueryResult {
    let output = s.run(query).await.unwrap().pop().unwrap();
    match output {
        StatementOutput::Query(result) => result,
        _ => panic!("query output expected"),
    }
}
fn rows(result: &QueryResult) -> Vec<Vec<String>> {
    result
        .batches
        .iter()
        .flat_map(|batch| {
            (0..batch.num_rows()).map(move |i| {
                batch
                    .columns()
                    .iter()
                    .map(|c| ScalarValue::try_from_array(c, i).unwrap().to_string())
                    .collect()
            })
        })
        .collect()
}

#[tokio::test]
async fn insert_updates_labels_properties_and_returns_use_datafusion() {
    for durable in [false, true] {
        let dir = Durable::new();
        let db = if durable { dir.open() } else { Database::new() };
        let mut s = session(&db);
        let result = run(&mut s, "INSERT (a:Person&Admin {name: 'Alice', age: 30})-[e:Knows {since: 2020}]->(b:Person {name: 'Bob', age: 40}) RETURN a.name AS a, b.name AS b, e.since AS since").await;
        assert_eq!(rows(&result), vec![vec!["Alice", "Bob", "2020"]]);
        assert_eq!(result.affected_elements, 3);
        assert!(result.physical_plan.contains("HashJoinExec"));
        let result = run(&mut s, "MATCH (n:Person) SET n.age = n.age + 1 RETURN n.name AS name, n.age AS age ORDER BY name").await;
        assert_eq!(rows(&result), vec![vec!["Alice", "31"], vec!["Bob", "41"]]);
        assert_eq!(result.affected_elements, 2);
        let result = run(&mut s, "MATCH (n {name: 'Alice'}) REMOVE n.age SET n IS Retired REMOVE n IS Admin RETURN PROPERTY_EXISTS(n, age) AS has_age, n IS LABELED Retired AS retired, n IS LABELED Admin AS admin").await;
        assert_eq!(rows(&result), vec![vec!["false", "true", "false"]]);
        let result = run(&mut s, "MATCH (n {name: 'Bob'}) SET n = {name: 'Robert', active: TRUE} RETURN n.name AS name, n.age AS age, n.active AS active").await;
        assert_eq!(rows(&result), vec![vec!["Robert", "NULL", "true"]]);
        run(&mut s, "MATCH ()-[e:Knows]->() SET e.since = e.since + 1").await;
        assert_eq!(
            rows(
                &s.query("MATCH ()-[e]->() RETURN e.since AS since")
                    .await
                    .unwrap()
            ),
            vec![vec!["2021"]]
        );
    }
}

#[tokio::test]
async fn insert_rejects_nonconcrete_labels_without_publishing_writes() {
    for durable in [false, true] {
        let dir = Durable::new();
        let db = if durable { dir.open() } else { Database::new() };
        let mut s = session(&db);
        let result = run(
            &mut s,
            "INSERT (a:(A&B))-[e:(R&S)]->(:C) RETURN a IS LABELED A AS a, a IS LABELED B AS b, e IS LABELED R AS r, e IS LABELED S AS s",
        )
        .await;
        assert_eq!(rows(&result), vec![vec!["true", "true", "true", "true"]]);
        let query = "MATCH (n) RETURN ELEMENT_ID(n) AS id ORDER BY id";
        let before = s.query(query).await.unwrap();
        for invalid in [
            "INSERT (:Created), (:!Forbidden)",
            "INSERT (:A|B)",
            "INSERT (:%)",
            "INSERT (:Created)-[:!Forbidden]->(:Created)",
            "INSERT (:Created)-[:A|B]->(:Created)",
            "INSERT (:Created)-[:%]->(:Created)",
            "MATCH (n:A) INSERT (n:%)",
        ] {
            assert!(
                matches!(s.run(invalid).await, Err(Error::UnsupportedFeature(_))),
                "{invalid}"
            );
            let after = s.query(query).await.unwrap();
            assert_eq!(after.commit_seq, before.commit_seq, "{invalid}");
            assert_eq!(rows(&after), rows(&before), "{invalid}");
        }
    }
}

#[tokio::test]
async fn insert_bare_label_set_keeps_case_distinct_labels() {
    let db = Database::new();
    let mut s = session(&db);
    run(&mut s, "INSERT (A & a {name: 'n'})-[A & a]->()").await;
    let result = run(
        &mut s,
        "MATCH (n:A) \
         RETURN n.name AS name, n IS LABELED A AS upper, n IS LABELED a AS lower",
    )
    .await;
    assert_eq!(rows(&result), vec![vec!["n", "true", "true"]]);
    assert_eq!(
        rows(&run(&mut s, "MATCH (n:A & a) RETURN n.name AS name").await),
        vec![vec!["n"]]
    );
    assert_eq!(
        rows(&run(&mut s, "MATCH (n:a) RETURN n.name AS name").await),
        vec![vec!["n"]]
    );
    assert_eq!(
        rows(
            &run(
                &mut s,
                "MATCH ()-[e]->() RETURN e IS LABELED A AS upper, e IS LABELED a AS lower",
            )
            .await
        ),
        vec![vec!["true", "true"]]
    );
    assert_eq!(
        rows(
            &run(
                &mut s,
                "MATCH ()-[e:A & a]->() RETURN e IS LABELED a AS lower"
            )
            .await
        ),
        vec![vec!["true"]]
    );
}

#[tokio::test]
async fn matched_insert_preserves_bags_and_shared_nodes_and_edge_directions() {
    let db = Database::new();
    let mut s = session(&db);
    run(&mut s, "INSERT (a:N {name: 'A'})-[e:E {v: 1}]->(b:N {name: 'B'}), (a)-[:E {v: 2}]->(b), (a)-[:U]-(a)").await;
    let result = run(&mut s, "MATCH (a)-[e:E]->(b) INSERT (a)<-[:New]-(c:C {value: e.v}) RETURN c.value AS value ORDER BY value").await;
    assert_eq!(rows(&result), vec![vec!["1"], vec!["2"]]);
    assert_eq!(result.affected_elements, 4);
    assert_eq!(
        s.query("MATCH (a)-[:U]-(b) RETURN a.name AS name")
            .await
            .unwrap()
            .row_count(),
        1
    );
    assert_eq!(
        s.query("MATCH (c:C)-[:New]->(a) RETURN c.value AS value")
            .await
            .unwrap()
            .row_count(),
        2
    );
    let result = run(
        &mut s,
        "MATCH (a)-[e:E]->(b) SET a.seen = TRUE RETURN a.seen AS seen",
    )
    .await;
    assert_eq!(result.row_count(), 2);
    assert_eq!(result.affected_elements, 1);
    assert!(matches!(
        s.run("MATCH (a)-[e:E]->(b) SET a.seen = e.v").await,
        Err(Error::UnsupportedFeature(_))
    ));
    assert_eq!(
        rows(
            &s.query("MATCH (a {name: 'A'}) RETURN a.seen AS seen")
                .await
                .unwrap()
        ),
        vec![vec!["true"]]
    );
}

#[tokio::test]
async fn writes_refresh_aliases_and_observe_previous_clauses() {
    let db = Database::new();
    let mut s = session(&db);
    run(&mut s, "INSERT (:N {v: 1}), (:N {v: 2})").await;
    let result = run(
        &mut s,
        "MATCH (n), (other) WHERE SAME(n, other) SET n.v = n.v + 10 RETURN other.v AS v ORDER BY v",
    )
    .await;
    assert_eq!(rows(&result), vec![vec!["11"], vec!["12"]]);
    let result = run(
        &mut s,
        "INSERT (n:N {v: 3}) SET n.v = n.v + 1 MATCH (m:N {v: 4}) RETURN SAME(n, m) AS same",
    )
    .await;
    assert_eq!(rows(&result), vec![vec!["true"]]);
    let result = run(
        &mut s,
        "MATCH (n {v: 4}) SET n.v = NULL RETURN PROPERTY_EXISTS(n, v) AS exists",
    )
    .await;
    assert_eq!(rows(&result), vec![vec!["false"]]);
}

#[tokio::test]
async fn null_insert_expression_does_not_pin_property_type() {
    let db = Database::new();
    let mut s = session(&db);
    run(
        &mut s,
        "INSERT (:P {v: 1 + NULL, name: 'A'})-[:E {w: 1.5 * NULL}]->(:Q {flag: NULL AND TRUE})",
    )
    .await;
    let result = run(
        &mut s,
        "MATCH (n:P)-[e:E]->(m:Q) RETURN PROPERTY_EXISTS(n, v) AS v, n.name AS name, PROPERTY_EXISTS(e, w) AS w, PROPERTY_EXISTS(m, flag) AS flag",
    )
    .await;
    assert_eq!(rows(&result), vec![vec!["false", "A", "false", "false"]]);
    let result = run(&mut s, "MATCH (n:P) SET n.v = 'hi' RETURN n.v AS v").await;
    assert_eq!(rows(&result), vec![vec!["hi"]]);
    let result = run(&mut s, "MATCH ()-[e:E]->() SET e.w = TRUE RETURN e.w AS w").await;
    assert_eq!(rows(&result), vec![vec!["true"]]);
    let result = run(&mut s, "MATCH (m:Q) SET m.flag = 7 RETURN m.flag AS flag").await;
    assert_eq!(rows(&result), vec![vec!["7"]]);

    run(&mut s, "INSERT (:R {text: 'ab' || NULL})").await;
    let result = run(
        &mut s,
        "MATCH (n:R) SET n.text = 4 RETURN n.text AS text, PROPERTY_EXISTS(n, text) AS before",
    )
    .await;
    assert_eq!(rows(&result), vec![vec!["4", "true"]]);

    // A non-null value in the same insert still stores that type.
    run(&mut s, "FOR n IN [1, NULL] INSERT (:Keep {v: n})").await;
    let blocked = s.run("MATCH (n:Keep) SET n.v = 'x'").await.unwrap_err();
    assert!(
        blocked
            .to_string()
            .contains("mixed types for graph property v"),
        "{blocked}"
    );
    let result = run(&mut s, "MATCH (n:Keep) RETURN n.v AS v ORDER BY v").await;
    assert_eq!(rows(&result), vec![vec!["1"], vec!["NULL"]]);
}

#[tokio::test]
async fn null_expression_removes_property_instead_of_pinning_its_type() {
    let db = Database::new();
    let mut s = session(&db);
    run(&mut s, "INSERT (:A {k: 1, v: 1})").await;
    let result = run(
        &mut s,
        "MATCH (n) SET n.v = 1 + NULL RETURN PROPERTY_EXISTS(n, v) AS exists, n.v IS NULL AS missing",
    )
    .await;
    assert_eq!(rows(&result), vec![vec!["false", "true"]]);
    let result = run(&mut s, "MATCH (n) SET n.v = 'hi' RETURN n.v AS v").await;
    assert_eq!(rows(&result), vec![vec!["hi"]]);

    run(&mut s, "MATCH (n) REMOVE n.v").await;
    run(&mut s, "MATCH (n) SET n.w = 1.5 * NULL").await;
    let result = run(&mut s, "MATCH (n) RETURN PROPERTY_EXISTS(n, w) AS exists").await;
    assert_eq!(rows(&result), vec![vec!["false"]]);
    let result = run(&mut s, "MATCH (n) SET n.w = TRUE RETURN n.w AS w").await;
    assert_eq!(rows(&result), vec![vec!["true"]]);

    run(&mut s, "MATCH (n) SET n.flag = NULL AND TRUE").await;
    let result = run(
        &mut s,
        "MATCH (n) SET n.flag = 7 RETURN n.flag AS flag, PROPERTY_EXISTS(n, w) AS kept",
    )
    .await;
    assert_eq!(rows(&result), vec![vec!["7", "true"]]);

    run(&mut s, "INSERT (:B {k: 2, v: 2})").await;
    run(&mut s, "MATCH (n:B) SET n = {k: n.k, v: n.v + NULL}").await;
    let result = run(
        &mut s,
        "MATCH (n:B) RETURN n.k AS k, PROPERTY_EXISTS(n, v) AS exists",
    )
    .await;
    assert_eq!(rows(&result), vec![vec!["2", "false"]]);
    let result = run(&mut s, "MATCH (n:B) SET n.v = 'x' RETURN n.v AS v").await;
    assert_eq!(rows(&result), vec![vec!["x"]]);

    // A non-null value in the same assignment still stores that type.
    run(&mut s, "INSERT (:C {k: 3, src: 5}), (:C {k: 4})").await;
    run(&mut s, "MATCH (n:C) SET n.v = n.src").await;
    let result = run(
        &mut s,
        "MATCH (n:C) RETURN n.k AS k, n.v AS v, PROPERTY_EXISTS(n, v) AS exists ORDER BY k",
    )
    .await;
    assert_eq!(
        rows(&result),
        vec![vec!["3", "5", "true"], vec!["4", "NULL", "false"]]
    );
}

#[tokio::test]
async fn delete_restrict_detach_and_explicit_edges_enforce_endpoints() {
    let db = Database::new();
    let mut s = session(&db);
    run(
        &mut s,
        "INSERT (a:N {name: 'A'})-[:E]->(b:N {name: 'B'}), (b)-[:U]-(b)",
    )
    .await;
    let blocked = s
        .run("MATCH (a {name: 'A'}) NODETACH DELETE a")
        .await
        .unwrap_err();
    assert!(blocked.to_string().contains("DETACH DELETE"), "{blocked}");
    assert_eq!(
        s.query("MATCH (n) RETURN n.name AS name")
            .await
            .unwrap()
            .row_count(),
        2
    );
    let result = run(
        &mut s,
        "MATCH (a {name: 'A'})-[e:E]->(b) LET saved = a.name DELETE a, e RETURN saved",
    )
    .await;
    assert_eq!(rows(&result), vec![vec!["A"]]);
    assert_eq!(result.affected_elements, 2);
    let result = run(&mut s, "MATCH (b {name: 'B'}) DETACH DELETE b").await;
    assert_eq!(result.affected_elements, 2);
    assert_eq!(
        s.query("MATCH (n) RETURN ELEMENT_ID(n) AS id")
            .await
            .unwrap()
            .row_count(),
        0
    );
}

#[tokio::test]
async fn failure_in_later_clause_or_result_rolls_back_all_graph_changes() {
    let dir = Durable::new();
    let db = dir.open();
    let mut s = session(&db);
    run(&mut s, "INSERT (:N {v: 1})").await;
    let before = s
        .query("MATCH (n) RETURN n.v AS v")
        .await
        .unwrap()
        .commit_seq;
    for bad in [
        "INSERT (n:N {v: 2}) SET n.v = missing RETURN n.v AS v",
        "MATCH (n) SET n.v = 3 RETURN 1 / 0 AS bad",
        "MATCH (n) SET n.v = 3 LET bad = 1 / 0 FINISH",
        "INSERT (n:N {v: 2}) MATCH (m {v: 1}) SET m.v = 'incompatible'",
        "MATCH (n) SET n.v = 'incompatible'",
        "INSERT (:N {v: 'also incompatible'})",
        "MATCH (n) DELETE n RETURN n.v AS v",
        "INSERT (:N {__gf_bad: 1})",
    ] {
        assert!(s.run(bad).await.is_err(), "{bad}");
    }
    assert!(s
        .query("MATCH (n) SET n.v = 8 RETURN n.v AS v")
        .await
        .is_err());
    let result = s.query("MATCH (n) RETURN n.v AS v").await.unwrap();
    assert_eq!(result.commit_seq, before);
    assert_eq!(rows(&result), vec![vec!["1"]]);
    assert_eq!(
        std::fs::read_dir(&dir.0)
            .unwrap()
            .filter(|p| p
                .as_ref()
                .unwrap()
                .path()
                .extension()
                .is_some_and(|e| e == "parquet"))
            .count(),
        0
    );
}

#[tokio::test]
async fn element_ids_are_not_reused_after_delete_checkpoint_and_restart() {
    let dir = Durable::new();
    let first;
    {
        let db = dir.open();
        let mut s = session(&db);
        first = rows(&run(&mut s, "INSERT (n:N) RETURN ELEMENT_ID(n) AS id").await)[0][0].clone();
        run(&mut s, "MATCH (n) DELETE n").await;
        db.checkpoint().unwrap();
    }
    {
        let db = dir.open();
        let mut s = db.session();
        s.execute("SESSION SET GRAPH g").unwrap();
        let second =
            rows(&run(&mut s, "INSERT (n:N) RETURN ELEMENT_ID(n) AS id").await)[0][0].clone();
        assert_ne!(first, second);
        run(&mut s, "INSERT (:N {v: 2})").await;
        assert_eq!(
            s.query("MATCH (n) RETURN ELEMENT_ID(n) AS id")
                .await
                .unwrap()
                .row_count(),
            2
        );
    }
}

#[tokio::test]
async fn empty_matches_do_not_insert_or_update_elements() {
    let db = Database::new();
    let mut s = session(&db);
    let result = run(
        &mut s,
        "MATCH (a:Missing) INSERT (b:N {v: 1}) RETURN b.v AS v",
    )
    .await;
    assert_eq!(result.row_count(), 0);
    assert_eq!(result.affected_elements, 0);
    assert_eq!(
        result.schema.field(0).data_type(),
        &graphfusion::arrow::datatypes::DataType::Int64
    );
    run(&mut s, "MATCH (n) SET n.v = 42").await;
    assert_eq!(
        s.query("MATCH (n) RETURN ELEMENT_ID(n) AS id")
            .await
            .unwrap()
            .row_count(),
        0
    );
}

#[tokio::test]
async fn empty_insert_does_not_publish_property_layouts() {
    for durable in [false, true] {
        for (prefix, affected) in [
            ("MATCH (a:Missing)", 0),
            ("INSERT (:Existing) FILTER FALSE", 1),
        ] {
            let dir = Durable::new();
            let db = if durable { dir.open() } else { Database::new() };
            let mut s = session(&db);
            let before = s.query("RETURN 1 AS value").await.unwrap().commit_seq;
            let result = run(
                &mut s,
                &format!("{prefix} INSERT (b:N {{v: 1}}) RETURN b.v AS v"),
            )
            .await;
            assert_eq!(result.row_count(), 0);
            assert_eq!(result.affected_elements, affected);
            assert_eq!(result.commit_seq, before + affected as u64);
            assert_eq!(
                result.schema.field(0).data_type(),
                &graphfusion::arrow::datatypes::DataType::Int64
            );
            db.checkpoint().unwrap();
            let result = run(&mut s, "INSERT (n:N {v: 'ok'}) RETURN n.v AS v").await;
            assert_eq!(rows(&result), vec![vec!["ok"]]);
            assert_eq!(
                s.query("MATCH (n) RETURN ELEMENT_ID(n) AS id")
                    .await
                    .unwrap()
                    .row_count(),
                affected + 1
            );
        }
    }
}

#[tokio::test]
async fn deleting_edges_keeps_surviving_node_bindings_and_rejects_stale_aliases() {
    let db = Database::new();
    let mut s = session(&db);
    run(&mut s, "INSERT (a:N {v: 1})-[:E]->(b:N {v: 2})").await;
    let result = run(
        &mut s,
        "MATCH (a)-[e]->(b) DELETE e RETURN a.v AS a, b.v AS b",
    )
    .await;
    assert_eq!(rows(&result), vec![vec!["1", "2"]]);
    assert!(s
        .run("MATCH (a {v: 1}), (alias {v: 1}) DELETE a RETURN alias.v AS stale")
        .await
        .is_err());
    assert_eq!(
        s.query("MATCH (n) RETURN n.v AS v")
            .await
            .unwrap()
            .row_count(),
        2
    );
    let result = run(
        &mut s,
        "MATCH (a {v: 1}), (b {v: 2}) DELETE a RETURN b.v AS v",
    )
    .await;
    assert_eq!(rows(&result), vec![vec!["2"]]);
}

#[tokio::test]
async fn partial_delete_keeps_live_rows_of_the_same_variable() {
    let db = Database::new();
    let mut s = session(&db);
    run(&mut s, "INSERT (:N {name: 'Ada'}), (:N {name: 'Bea'})").await;
    assert!(s
        .run(
            "MATCH (n:N)
             MATCH (d:N WHERE d.name = 'Ada')
             DETACH DELETE d
             RETURN n.name AS name",
        )
        .await
        .is_err());
    assert_eq!(
        rows(&run(&mut s, "MATCH (n:N) RETURN n.name AS name ORDER BY name").await),
        vec![vec!["Ada"], vec!["Bea"]]
    );
    let result = run(
        &mut s,
        "MATCH (n:N)
         LET label = n.name
         MATCH (d:N WHERE d.name = 'Ada')
         DETACH DELETE d
         FILTER label = 'Bea'
         RETURN n.name AS name, n IS LABELED N AS labeled",
    )
    .await;
    assert_eq!(rows(&result), vec![vec!["Bea", "true"]]);
    assert_eq!(
        rows(&run(&mut s, "MATCH (n:N) RETURN n.name AS name").await),
        vec![vec!["Bea"]]
    );
}

#[tokio::test]
async fn repeated_inserts_do_not_rewrite_one_file_per_previous_row() {
    let dir = Durable::new();
    let db = dir.open();
    let mut s = session(&db);
    for i in 0..8 {
        run(&mut s, &format!("INSERT (:N {{i: {i}}})")).await;
    }
    let files = std::fs::read_dir(&dir.0)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".parquet"))
        .count();
    assert_eq!(files, 0, "small inserts must remain in MemTable");
    drop(s);
    drop(db);
    let db = dir.open();
    let mut s = db.session();
    s.execute("SESSION SET GRAPH g").unwrap();
    let result = s.query("MATCH (n:N) RETURN COUNT(*) AS c").await.unwrap();
    assert_eq!(rows(&result), vec![vec!["8"]]);
}

#[tokio::test]
async fn detach_indexes_handle_parallel_edges_loops_directions_and_separate_id_spaces() {
    use graphfusion::{
        arrow::{
            array::{Int64Array, UInt64Array},
            datatypes::{DataType, Field, Schema},
            record_batch::RecordBatch,
        },
        graph::{EdgeTable, GraphData, NodeTable, DESTINATION, ID, SOURCE},
        StorageOptions,
    };
    use std::sync::Arc;
    for mode in ["memory", "resident", "parquet"] {
        let dir = Durable::new();
        let db = if mode == "memory" {
            Database::new()
        } else {
            dir.open()
        };
        db.configure_storage(StorageOptions {
            memtable_max_rows: usize::MAX,
            memtable_max_bytes: usize::MAX,
        })
        .unwrap();
        let mut s = session(&db);
        let schema = Arc::new(Schema::new(vec![
            Field::new(ID, DataType::UInt64, false),
            Field::new("v", DataType::Int64, false),
        ]));
        let nodes = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(UInt64Array::from(vec![10, 20, 30])),
                Arc::new(Int64Array::from(vec![1, 2, 3])),
            ],
        )
        .unwrap();
        let edges = |ids: Vec<u64>, sources: Vec<u64>, destinations: Vec<u64>, directed| {
            let schema = Arc::new(Schema::new(vec![
                Field::new(ID, DataType::UInt64, false),
                Field::new(SOURCE, DataType::UInt64, false),
                Field::new(DESTINATION, DataType::UInt64, false),
                Field::new("v", DataType::Int64, false),
            ]));
            let batch = RecordBatch::try_new(
                schema.clone(),
                vec![
                    Arc::new(UInt64Array::from(ids.clone())),
                    Arc::new(UInt64Array::from(sources)),
                    Arc::new(UInt64Array::from(destinations)),
                    Arc::new(Int64Array::from(
                        ids.iter().map(|id| *id as i64).collect::<Vec<_>>(),
                    )),
                ],
            )
            .unwrap();
            EdgeTable::try_new(
                vec![if directed { "E" } else { "U" }.into()],
                directed,
                schema,
                vec![batch],
            )
            .unwrap()
        };
        s.replace_graph_data(
            GraphData::try_new(
                vec![NodeTable::try_new(vec!["N".into()], schema, vec![nodes]).unwrap()],
                vec![
                    edges(
                        vec![10, 20, 30, 40],
                        vec![10, 10, 10, 20],
                        vec![20, 20, 10, 30],
                        true,
                    ),
                    edges(vec![50], vec![20], vec![10], false),
                ],
            )
            .unwrap(),
        )
        .unwrap();
        if mode == "parquet" {
            db.checkpoint().unwrap();
        }
        let mut old = db.session();
        old.execute("SESSION SET GRAPH g; START TRANSACTION READ ONLY")
            .unwrap();
        // Property replacement leaves endpoints intact, including in cached adjacency.
        run(&mut s, "MATCH ()-[e:E {v: 10}]->() SET e.v = 11").await;
        assert!(s.run("MATCH (n {v: 1}) DELETE n").await.is_err());
        let deleted = run(&mut s, "MATCH (n {v: 1}) DETACH DELETE n").await;
        assert_eq!(deleted.affected_elements, 5, "{mode}");
        assert!(!deleted.physical_plan.contains("NestedLoopJoinExec"));
        assert_eq!(
            rows(
                &s.query("MATCH (n) RETURN n.v AS v ORDER BY v")
                    .await
                    .unwrap()
            ),
            vec![vec!["2"], vec!["3"]]
        );
        assert_eq!(
            rows(&s.query("MATCH ()-[e:E]->() RETURN e.v AS v").await.unwrap()),
            vec![vec!["40"]]
        );
        assert_eq!(
            old.query("MATCH (n) RETURN n.v AS v")
                .await
                .unwrap()
                .row_count(),
            3
        );
        assert_eq!(
            old.query("MATCH ()-[e:E]->() RETURN e.v AS v")
                .await
                .unwrap()
                .row_count(),
            4
        );
        old.execute("COMMIT").unwrap();
        drop(old);
        if mode != "memory" {
            drop(s);
            drop(db);
            let db = dir.open();
            let mut s = db.session();
            s.execute("SESSION SET GRAPH g").unwrap();
            assert_eq!(
                rows(&s.query("MATCH ()-[e:E]->() RETURN e.v AS v").await.unwrap()),
                vec![vec!["40"]]
            );
            assert_eq!(
                run(&mut s, "MATCH (n {v: 2}) DETACH DELETE n")
                    .await
                    .affected_elements,
                2
            );
        }
    }
}

#[tokio::test]
async fn deleted_elements_fail_in_null_writes_and_direction_tests() {
    for query in [
        "MATCH (n:N), (m:M) DETACH DELETE n SET m.flag = n.missing",
        "MATCH (n:N) DETACH DELETE n INSERT (:Made {flag: n.missing})",
        "MATCH ()-[e:E]->() DELETE e RETURN e IS DIRECTED AS directed",
    ] {
        let db = Database::new();
        let mut s = session(&db);
        s.run("INSERT (n:N {name:'n'}), (m:M {name:'m', flag:true}), (n)-[:E]->(m)")
            .await
            .unwrap();
        let error = s
            .run(query)
            .await
            .err()
            .unwrap_or_else(|| panic!("deleted access committed: {query}"));
        assert!(
            error.to_string().contains("deleted or unavailable"),
            "{query}: {error}"
        );
        assert_eq!(
            rows(&run(&mut s, "MATCH (n) RETURN COUNT(*) AS c").await),
            vec![vec!["2"]]
        );
        assert_eq!(
            rows(&run(&mut s, "MATCH ()-[e:E]->() RETURN COUNT(*) AS c").await),
            vec![vec!["1"]]
        );
        assert_eq!(
            rows(&run(&mut s, "MATCH (m:M) RETURN m.flag AS flag").await),
            vec![vec!["true"]]
        );
    }
}

#[tokio::test]
async fn fragment_comparison_rejects_deleted_elements_and_rolls_back() {
    let mut s = Database::new().session();
    s.run("CREATE GRAPH g ANY; SESSION SET GRAPH g; INSERT (:I {v:1,k:1}),(:I {v:2,k:2}),(:T {v:'text',k:3})").await.unwrap();
    let result = s
        .run("MATCH (n),(m:I {k:1}) DELETE m FILTER n.v = 1 RETURN 1 AS v")
        .await;
    let error = result.expect_err("deleted element comparison unexpectedly succeeded");
    assert!(
        error.to_string().contains("deleted or unavailable"),
        "{error}"
    );
    assert_eq!(s.query("MATCH (n) RETURN n").await.unwrap().row_count(), 3);
}

#[tokio::test]
async fn stored_reference_comparison_rejects_deleted_aliases() {
    let mut s = Database::new().session();
    s.run("CREATE GRAPH g ANY; SESSION SET GRAPH g; INSERT (:P {v:1,k:1}),(:P {v:2,k:2}),(:D {v:'1',k:3})").await.unwrap();
    let error = s
        .run("MATCH (n) LET r=n MATCH (m:P {k:1}) DELETE m FILTER r.v = '1' RETURN 1 AS value")
        .await
        .expect_err("deleted alias comparison committed");
    assert!(
        error.to_string().contains("deleted or unavailable"),
        "{error}"
    );
    assert_eq!(s.query("MATCH (n) RETURN n").await.unwrap().row_count(), 3);
}

#[tokio::test]
async fn stored_row_comparison_rejects_deleted_elements() {
    for variable in ["n", "r"] {
        let mut s = Database::new().session();
        s.run("CREATE GRAPH g ANY; SESSION SET GRAPH g; INSERT (:I {v:1,k:1}),(:I {v:2,k:2}),(:T {v:'1',k:3})").await.unwrap();
        let query = format!("MATCH (n) LET r=n, target='1' MATCH (m:I {{k:1}}) DELETE m FILTER {variable}.v = target RETURN 1 AS v");
        let error = s
            .run(&query)
            .await
            .expect_err("deleted row comparison committed");
        assert!(
            error.to_string().contains("deleted or unavailable"),
            "{error}"
        );
        assert_eq!(s.query("MATCH(n) RETURN n").await.unwrap().row_count(), 3);
    }
}

#[tokio::test]
async fn identifier_case_matches_the_parser() {
    let db = Database::new();
    let mut s = session(&db);
    assert_eq!(
        rows(&run(&mut s, "INSERT (:N {age: 30, name: 'Ann'}), (:N {AGE: 40, name: 'Bea'}) MATCH (n) WHERE n.AGE > 0 RETURN n.name AS name, n.age AS age ORDER BY name").await),
        vec![vec!["Ann", "30"], vec!["Bea", "40"]]
    );
    assert_eq!(
        rows(&run(&mut s, "MATCH (n {AGE: 30}) RETURN N.name AS name").await),
        vec![vec!["Ann"]]
    );
    assert_eq!(
        rows(
            &run(
                &mut s,
                "MATCH (n {name: 'Ann'}) LET r = n RETURN r.AGE AS age, PROPERTY_EXISTS(r, AGE) AS has"
            )
            .await
        ),
        vec![vec!["30", "true"]]
    );
    assert_eq!(
        rows(
            &run(
                &mut s,
                "MATCH (n) SET n.AGE = n.age + 1 RETURN n.name AS name, n.age AS age ORDER BY name"
            )
            .await
        ),
        vec![vec!["Ann", "31"], vec!["Bea", "41"]]
    );
    assert_eq!(
        rows(&run(&mut s, "MATCH (n:N WHERE N.AGE = 41) RETURN n.name AS name").await),
        vec![vec!["Bea"]]
    );
    assert_eq!(
        rows(
            &run(
                &mut s,
                "MATCH (n {name: 'Ann'}) REMOVE n.AGE RETURN PROPERTY_EXISTS(n, age) AS has_age"
            )
            .await
        ),
        vec![vec!["false"]]
    );
    assert_eq!(
        rows(&run(&mut s, "INSERT (a:N {name: 'Self'})-[:E]->(A) MATCH (n {name: 'Self'})-[e:E]->(n) RETURN n.name AS name").await),
        vec![vec!["Self"]]
    );
    assert!(run(
        &mut s,
        "MATCH (a {name: 'Ann'})-[:E]->(A) RETURN a.name AS name"
    )
    .await
    .batches
    .iter()
    .all(|batch| batch.num_rows() == 0));
    assert_eq!(
        rows(&run(&mut s, "LET name = 'Ann' RETURN NAME AS name").await),
        vec![vec!["Ann"]]
    );
    assert_eq!(
        rows(&run(&mut s, "RETURN 2 AS AGE ORDER BY age").await),
        vec![vec!["2"]]
    );
    s.execute("SESSION SET VALUE $name = 'Ann'").unwrap();
    assert_eq!(
        rows(&run(&mut s, "RETURN $NAME AS name").await),
        vec![vec!["Ann"]]
    );
    s.execute("SESSION SET VALUE IF NOT EXISTS $NAME = 'Bea'")
        .unwrap();
    assert_eq!(
        rows(&run(&mut s, "RETURN $name AS name").await),
        vec![vec!["Ann"]]
    );
    s.execute("SESSION RESET $NAME").unwrap();
    assert!(s
        .run("RETURN $name AS name")
        .await
        .unwrap_err()
        .to_string()
        .contains("parameter"));
    assert_eq!(
        rows(&run(&mut s, "RETURN 1 AS name UNION ALL RETURN 2 AS NAME").await),
        vec![vec!["1"], vec!["2"]]
    );
    assert_eq!(
        rows(
            &run(
                &mut s,
                "RETURN 1 AS name NEXT YIELD NAME RETURN NAME AS name"
            )
            .await
        ),
        vec![vec!["1"]]
    );
    assert_eq!(
        rows(&run(&mut s, "RETURN 1 AS k, COUNT(*) AS c GROUP BY K").await),
        vec![vec!["1", "1"]]
    );
    assert_eq!(
        rows(
            &run(
                &mut s,
                "INSERT (:Person {name: 'Pat'}) MATCH (n:person) RETURN n.name AS name"
            )
            .await
        )
        .len(),
        0
    );
    assert_eq!(
        rows(&run(&mut s, "MATCH (n:Person) RETURN n.name AS name").await),
        vec![vec!["Pat"]]
    );
    assert!(s
        .run("SESSION SET GRAPH G")
        .await
        .unwrap_err()
        .to_string()
        .contains("not found"));
}

#[tokio::test]
async fn case_folded_property_names_keep_internal_fields_hidden_and_reject_duplicates() {
    let db = Database::new();
    let mut s = session(&db);
    assert!(s.run("INSERT(:N {age:1,AGE:2})").await.is_err());
    assert!(s.run("INSERT(:N {__GF_hidden:1})").await.is_err());
    s.run("INSERT(:N {Age:1})").await.unwrap();
    s.run("MATCH(n:N) SET n.AGE=2").await.unwrap();
    let r = s.query("MATCH(n:N) LET copy=n RETURN copy.aGe AS age,PROPERTY_EXISTS(copy,__GF_ID) AS hidden ORDER BY age").await.unwrap();
    assert_eq!(rows(&r), vec![vec!["2".to_string(), "false".to_string()]]);
}

#[tokio::test]
async fn removing_the_last_value_releases_nullable_property_layouts() {
    for durable in [false, true] {
        for remove in ["MATCH(n:C {k:1}) REMOVE n.v", "MATCH(n:C {k:1}) DELETE n"] {
            let dir = Durable::new();
            let db = if durable { dir.open() } else { Database::new() };
            let mut s = session(&db);
            run(&mut s, "INSERT (:C {k:1,v:1}),(:C {k:2,v:2})").await;
            run(&mut s, "MATCH(n:C) SET n.v=NULLIF(n.v,2)").await;
            if durable {
                db.checkpoint().unwrap();
            }
            let mut old = db.session();
            old.execute("SESSION SET GRAPH g; START TRANSACTION READ ONLY")
                .unwrap();
            run(&mut s, remove).await;
            run(&mut s, "MATCH(n:C {k:2}) SET n.v='x'").await;
            assert_eq!(
                rows(&s.query("MATCH(n:C {k:2}) RETURN n.v AS v").await.unwrap()),
                vec![vec!["x"]]
            );
            assert_eq!(
                rows(&old.query("MATCH(n:C {k:2}) RETURN n.v AS v").await.unwrap()),
                vec![vec!["NULL"]]
            );
            old.execute("ROLLBACK").unwrap();
            drop(old);
            drop(s);
            drop(db);
            if durable {
                let reopened = dir.open();
                let mut reader = reopened.session();
                reader.execute("SESSION SET GRAPH g").unwrap();
                assert_eq!(
                    rows(
                        &reader
                            .query("MATCH(n:C {k:2}) RETURN n.v AS v")
                            .await
                            .unwrap()
                    ),
                    vec![vec!["x"]]
                );
            }
        }
    }
}

#[tokio::test]
async fn fragment_null_assignments_do_not_inherit_another_fragments_value_type() {
    let db = Database::new();
    let mut s = session(&db);
    run(&mut s, "INSERT (:I {v:1}),(:F {v:2.0})").await;
    run(&mut s, "MATCH(n:I|F) SET n.v=NULLIF(n.v,1.0)").await;
    run(&mut s, "MATCH(n:I) SET n.v='new'").await;
    assert_eq!(
        rows(&s.query("MATCH(n:I) RETURN n.v AS v").await.unwrap()),
        vec![vec!["new"]]
    );
    assert_eq!(
        rows(&s.query("MATCH(n:F) RETURN n.v AS v").await.unwrap()),
        vec![vec!["2"]]
    );
}

#[tokio::test]
async fn insert_destination_can_read_the_connecting_edge() {
    let db = Database::new();
    let mut s = session(&db);
    let result = run(
        &mut s,
        "INSERT (a:N {k: 1})-[e:E {w: 2}]->(b:N {k: e.w}) RETURN b.k AS k",
    )
    .await;
    assert_eq!(rows(&result), vec![vec!["2"]]);
    assert_eq!(result.affected_elements, 3);
    let result = run(
        &mut s,
        "INSERT (a:N)-[e:E {w: b.k}]->(b:N {k: 10}) RETURN e.w AS w",
    )
    .await;
    assert_eq!(rows(&result), vec![vec!["10"]]);
    let result = run(
        &mut s,
        "INSERT (a:N)-[e:E {w: 2}]->(b:N {k: e.w})-[f:F {w: b.k}]->(c:N {k: f.w}) RETURN b.k AS bk, f.w AS fw, c.k AS ck",
    )
    .await;
    assert_eq!(rows(&result), vec![vec!["2", "2", "2"]]);
    let result = run(
        &mut s,
        "INSERT (a:N)-[e:E {w: b.m}]->(b:N {m: 10, k: e.w}) RETURN e.w AS w, b.k AS k, b.m AS m",
    )
    .await;
    assert_eq!(rows(&result), vec![vec!["10", "10", "10"]]);
    let result = run(
        &mut s,
        "INSERT (a:N)<-[e:E {w: 4}]-(b:N {k: e.w}) RETURN b.k AS k",
    )
    .await;
    assert_eq!(rows(&result), vec![vec!["4"]]);
    let result = run(
        &mut s,
        "INSERT (a:N)~[e:E {w: 7}]~(b:N {k: e.w}) RETURN b.k AS k",
    )
    .await;
    assert_eq!(rows(&result), vec![vec!["7"]]);
    let result = run(
        &mut s,
        "INSERT (a:N {k: 1})-[e:E {w: 2}]->(b:N {k: e.w + 1}) RETURN b.k AS k",
    )
    .await;
    assert_eq!(rows(&result), vec![vec!["3"]]);
    let result = run(
        &mut s,
        "INSERT (a:N {k: 1})-[e:E {w: a.k + b.k}]->(b:N {k: 10}) RETURN e.w AS w",
    )
    .await;
    assert_eq!(rows(&result), vec![vec!["11"]]);
    run(
        &mut s,
        "INSERT (a:N)-[e:E {w: 3}]->(:N {name: 'Z', k: e.w})",
    )
    .await;
    assert_eq!(
        rows(
            &s.query("MATCH (n {name: 'Z'}) RETURN n.k AS k")
                .await
                .unwrap()
        ),
        vec![vec!["3"]]
    );
    let result = run(
        &mut s,
        "FOR n IN [1, 2] INSERT (a:N {k: n})-[e:E {w: n}]->(b:N {k: e.w}) RETURN b.k AS k ORDER BY k",
    )
    .await;
    assert_eq!(rows(&result), vec![vec!["1"], vec!["2"]]);
    assert_eq!(result.affected_elements, 6);
    for query in [
        "INSERT (a:N {k: a.k})",
        "INSERT (a:N {k: e.w})-[e:E {w: 2}]->(b:N)",
        "INSERT (b:N {k: e.w})-[e:E {w: 5}]->(c:N)",
    ] {
        let err = s.run(query).await.unwrap_err();
        assert!(
            err.to_string().contains("unbound variable"),
            "{query}: {err}"
        );
    }
}

#[tokio::test]
async fn insert_edge_dependencies_ignore_property_keys() {
    let db = Database::new();
    let mut s = session(&db);
    let result = run(
        &mut s,
        "INSERT (a:N {e:10})-[e:E {w:b.k}]->(b:N {k:a.e}) RETURN e.w AS w,b.k AS k",
    )
    .await;
    assert_eq!(rows(&result), vec![vec!["10", "10"]]);
}

#[tokio::test]
async fn insert_edge_dependencies_match_variable_case() {
    let db = Database::new();
    let mut s = session(&db);
    let result = run(
        &mut s,
        "INSERT (a:N)-[Edge:E {w:7}]->(b:N {k:edge.w}) RETURN b.k AS k",
    )
    .await;
    assert_eq!(rows(&result), vec![vec!["7"]]);
    assert_eq!(result.affected_elements, 3);
}
