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
async fn delete_restrict_detach_and_explicit_edges_enforce_endpoints() {
    let db = Database::new();
    let mut s = session(&db);
    run(
        &mut s,
        "INSERT (a:N {name: 'A'})-[:E]->(b:N {name: 'B'}), (b)-[:U]-(b)",
    )
    .await;
    assert!(s
        .run("MATCH (a {name: 'A'}) NODETACH DELETE a")
        .await
        .is_err());
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
        1
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
