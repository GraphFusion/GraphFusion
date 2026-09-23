use datafusion::common::ScalarValue;
use graphfusion::{
    catalog::ObjectKind, Database, Error, OpenOptions, QueryResult, Session, StatementOutput,
    TransactionAction, TransactionStatus, Value,
};
use std::path::PathBuf;

struct Fixture {
    db: Database,
    directory: Option<PathBuf>,
}
impl Fixture {
    fn new(durable: bool) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let directory = durable.then(|| {
            std::env::temp_dir().join(format!(
                "graphfusion-transactions-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ))
        });
        let db = directory.as_ref().map_or_else(Database::new, |p| {
            Database::open(
                p,
                OpenOptions {
                    create_if_missing: true,
                },
            )
            .unwrap()
        });
        Self { db, directory }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(path) = &self.directory {
            let _ = std::fs::remove_dir_all(path);
        }
    }
}
fn exists(db: &Database, name: &str) -> bool {
    db.with_catalog(|c| {
        c.lookup(graphfusion::catalog::MAIN_SCHEMA, ObjectKind::Graph, name)
            .is_some()
    })
    .unwrap()
}
fn rows(result: &QueryResult) -> Vec<Vec<String>> {
    result
        .batches
        .iter()
        .flat_map(|b| {
            (0..b.num_rows()).map(move |i| {
                b.columns()
                    .iter()
                    .map(|c| ScalarValue::try_from_array(c, i).unwrap().to_string())
                    .collect()
            })
        })
        .collect()
}
async fn read(s: &mut Session, sql: &str) -> QueryResult {
    s.query(sql).await.unwrap_or_else(|e| panic!("{sql}: {e}"))
}
async fn values(s: &mut Session) -> Vec<Vec<String>> {
    rows(&read(s, "MATCH (n:N) RETURN n.v AS v ORDER BY v").await)
}
async fn seeded(db: &Database) -> Session {
    let mut s = db.session();
    s.run("CREATE GRAPH g ANY GRAPH; CREATE GRAPH h ANY GRAPH; SESSION SET GRAPH g; INSERT (:N {v: 1}); USE GRAPH h INSERT (:N {v: 10})").await.unwrap();
    s
}
fn active(s: &Session) -> bool {
    matches!(s.transaction_status(), TransactionStatus::Active { .. })
}
fn failed(s: &Session) -> bool {
    matches!(s.transaction_status(), TransactionStatus::Failed { .. })
}

#[tokio::test]
async fn explicit_commit_publishes_catalog_and_multiple_graph_statements_once() {
    for durable in [false, true] {
        let f = Fixture::new(durable);
        let mut s = f.db.session();
        let start = s.execute("START TRANSACTION READ WRITE").unwrap();
        assert_eq!(
            start.statements[0].transaction_action,
            Some(TransactionAction::Started)
        );
        assert!(start.statements[0].transaction_pending);
        let commands = s
            .execute("CREATE GRAPH g ANY GRAPH; SESSION SET GRAPH g")
            .unwrap();
        assert!(commands.statements.iter().all(|r| r.transaction_pending));
        assert!(!exists(&f.db, "g"));
        let first = s
            .run("INSERT (:N {v: 1}); INSERT (:N {v: 2}); MATCH (n:N) SET n.v = n.v + 10")
            .await
            .unwrap();
        for result in first {
            assert!(
                matches!(result, StatementOutput::Query(r) if r.transaction_pending && r.commit_seq == 0)
            );
        }
        assert_eq!(values(&mut s).await, &[vec!["11"], vec!["12"]]);
        assert!(read(&mut s, "RETURN 1 AS n").await.transaction_pending);
        assert_eq!(f.db.statistics().unwrap().commits, 0);
        let committed = s.execute("COMMIT").unwrap();
        assert_eq!(committed.statements[0].commit_seq, 1);
        assert_eq!(
            committed.statements[0].transaction_action,
            Some(TransactionAction::Committed)
        );
        assert!(!committed.statements[0].transaction_pending);
        assert_eq!(s.transaction_status(), TransactionStatus::Idle);
        assert!(exists(&f.db, "g"));
        assert_eq!(f.db.statistics().unwrap().commits, 1);
        let mut other = f.db.session();
        other.execute("SESSION SET GRAPH g").unwrap();
        assert_eq!(values(&mut other).await, &[vec!["11"], vec!["12"]]);
        assert!(!read(&mut other, "RETURN 1 AS n").await.transaction_pending);
        f.db.checkpoint().unwrap();
    }
}

#[tokio::test]
async fn rollback_drop_and_close_discard_private_graph_and_catalog_changes() {
    for durable in [false, true] {
        let f = Fixture::new(durable);
        let mut s = seeded(&f.db).await;
        let before = f.db.statistics().unwrap().commits;
        s.run(
            "START TRANSACTION; INSERT (:N {v: 2}); DROP GRAPH h; CREATE GRAPH new_graph ANY GRAPH",
        )
        .await
        .unwrap();
        assert_eq!(values(&mut s).await, &[vec!["1"], vec!["2"]]);
        assert!(exists(&f.db, "h"));
        assert!(!exists(&f.db, "new_graph"));
        assert!(matches!(f.db.checkpoint(), Err(Error::Busy)));
        let rollback = s.execute("ROLLBACK").unwrap();
        assert_eq!(
            rollback.statements[0].transaction_action,
            Some(TransactionAction::RolledBack)
        );
        assert_eq!(values(&mut s).await, &[vec!["1"]]);
        assert_eq!(f.db.statistics().unwrap().commits, before);
        s.run("START TRANSACTION; INSERT (:N {v: 3})")
            .await
            .unwrap();
        drop(s);
        f.db.checkpoint().unwrap();
        let mut s = f.db.session();
        s.execute("SESSION SET GRAPH g").unwrap();
        assert_eq!(values(&mut s).await, &[vec!["1"]]);
        s.run("START TRANSACTION; INSERT (:N {v: 4}); SESSION CLOSE")
            .await
            .unwrap();
        assert!(s.state().closed);
        assert_eq!(s.transaction_status(), TransactionStatus::Idle);
        assert!(matches!(s.execute("COMMIT"), Err(Error::SessionClosed)));
        f.db.checkpoint().unwrap();
    }
}

#[tokio::test]
async fn execution_and_parse_errors_fail_the_entire_explicit_transaction() {
    for durable in [false, true] {
        let f = Fixture::new(durable);
        let mut s = seeded(&f.db).await;
        for bad in [
            "RETURN 1 / 0 AS n",
            "RETURN missing AS n",
            "RETURN (",
            "CALL absent()",
        ] {
            s.run("START TRANSACTION; INSERT (:N {v: 2}); CREATE GRAPH rolled_back ANY GRAPH")
                .await
                .unwrap();
            assert!(s.run(bad).await.is_err(), "{bad}");
            assert!(failed(&s));
            assert!(!exists(&f.db, "rolled_back"));
            assert!(matches!(
                s.query("RETURN 1 AS n").await,
                Err(Error::TransactionFailed)
            ));
            assert!(matches!(s.execute("COMMIT"), Err(Error::TransactionFailed)));
            assert!(s.set_parameter("x", Value::Integer(1)).is_err());
            f.db.checkpoint().unwrap(); // failed transactions release their pins immediately
            s.execute("ROLLBACK").unwrap();
            assert_eq!(values(&mut s).await, &[vec!["1"]]);
        }
        assert!(s.run("CREATE GRAPH durable_before ANY GRAPH; START TRANSACTION; CREATE GRAPH private_after ANY GRAPH; RETURN 1 / 0 AS n; COMMIT").await.is_err());
        assert!(exists(&f.db, "durable_before"));
        assert!(!exists(&f.db, "private_after"));
        assert!(failed(&s));
        s.execute("ROLLBACK").unwrap();
    }
}

#[tokio::test]
async fn read_only_blocks_every_write_path_including_noops_and_imports() {
    for durable in [false, true] {
        let f = Fixture::new(durable);
        let mut s = seeded(&f.db).await;
        let commits = f.db.statistics().unwrap().commits;
        for sql in [
            "INSERT (:N)",
            "MATCH (n:Missing) DELETE n",
            "MATCH (n) SET n.v = 2",
            "MATCH (n) REMOVE n.v",
            "CREATE GRAPH IF NOT EXISTS g ANY GRAPH",
            "DROP GRAPH IF EXISTS absent",
            "CREATE SCHEMA blocked",
            "DROP GRAPH g",
        ] {
            s.execute("START TRANSACTION READ ONLY").unwrap();
            assert!(
                matches!(s.run(sql).await, Err(Error::ReadOnlyTransaction)),
                "{sql}"
            );
            assert!(failed(&s));
            s.execute("ROLLBACK").unwrap();
        }
        s.execute("START TRANSACTION READ ONLY").unwrap();
        assert!(matches!(
            s.replace_graph_data(graphfusion::graph::GraphData::default()),
            Err(Error::ReadOnlyTransaction)
        ));
        s.execute("ROLLBACK; START TRANSACTION READ ONLY; SESSION SET VALUE $n INTEGER = 7")
            .unwrap();
        assert_eq!(rows(&read(&mut s, "RETURN $n AS n").await), &[vec!["7"]]);
        s.execute("COMMIT").unwrap();
        assert_eq!(values(&mut s).await, &[vec!["1"]]);
        assert_eq!(f.db.statistics().unwrap().commits, commits);
    }
}

#[tokio::test]
async fn snapshots_are_repeatable_across_replacements_and_catalog_rebinding() {
    for durable in [false, true] {
        let f = Fixture::new(durable);
        let mut s = seeded(&f.db).await;
        s.execute("START TRANSACTION READ ONLY").unwrap();
        let initial = s.transaction_status();
        let mut writer = f.db.session();
        writer.run("USE GRAPH g INSERT (:N {v: 2}); DROP GRAPH h; CREATE GRAPH h ANY GRAPH; USE GRAPH h INSERT (:N {v: 20})").await.unwrap();
        assert_eq!(values(&mut s).await, &[vec!["1"]]);
        assert_eq!(
            rows(&read(&mut s, "USE GRAPH h MATCH (n) RETURN n.v AS v").await),
            &[vec!["10"]]
        );
        assert_eq!(s.transaction_status(), initial);
        assert!(matches!(f.db.checkpoint(), Err(Error::Busy)));
        s.execute("COMMIT").unwrap();
        assert_eq!(values(&mut s).await, &[vec!["1"], vec!["2"]]);
        assert_eq!(
            rows(&read(&mut s, "USE GRAPH h MATCH (n) RETURN n.v AS v").await),
            &[vec!["20"]]
        );
        f.db.checkpoint().unwrap();
    }
}

#[tokio::test]
async fn graph_conflicts_abort_the_loser_and_disjoint_writes_merge() {
    for durable in [false, true] {
        let f = Fixture::new(durable);
        let mut a = seeded(&f.db).await;
        let mut b = f.db.session();
        b.execute("SESSION SET GRAPH g").unwrap();
        a.execute("START TRANSACTION").unwrap();
        b.execute("START TRANSACTION").unwrap();
        a.run("INSERT (:N {v: 2})").await.unwrap();
        b.run("INSERT (:N {v: 3})").await.unwrap();
        a.execute("COMMIT").unwrap();
        assert!(matches!(b.execute("COMMIT"), Err(Error::Conflict(_))));
        assert!(failed(&b));
        b.execute("ROLLBACK").unwrap();
        assert_eq!(values(&mut b).await, &[vec!["1"], vec!["2"]]);
        a.execute("START TRANSACTION").unwrap();
        b.execute("START TRANSACTION").unwrap();
        a.run("INSERT (:N {v: 4})").await.unwrap();
        b.run("USE GRAPH h INSERT (:N {v: 5})").await.unwrap();
        a.execute("COMMIT").unwrap();
        b.execute("COMMIT").unwrap();
        assert_eq!(values(&mut a).await, &[vec!["1"], vec!["2"], vec!["4"]]);
        assert_eq!(
            rows(&read(&mut b, "USE GRAPH h MATCH (n) RETURN n.v AS v ORDER BY v").await),
            &[vec!["5"], vec!["10"]]
        );
    }
}

#[tokio::test]
async fn cross_statement_reads_prevent_write_skew_and_negative_name_phantoms() {
    for durable in [false, true] {
        let f = Fixture::new(durable);
        let mut a = seeded(&f.db).await;
        let mut b = f.db.session();
        a.execute("START TRANSACTION").unwrap();
        b.execute("START TRANSACTION").unwrap();
        read(&mut a, "USE GRAPH h MATCH (n) RETURN COUNT(*) AS c").await;
        read(&mut b, "USE GRAPH g MATCH (n) RETURN COUNT(*) AS c").await;
        a.run("USE GRAPH g INSERT (:N {v: 2})").await.unwrap();
        b.run("USE GRAPH h INSERT (:N {v: 3})").await.unwrap();
        a.execute("COMMIT").unwrap();
        assert!(matches!(b.execute("COMMIT"), Err(Error::Conflict(_))));
        b.execute("ROLLBACK").unwrap();
        a.execute("START TRANSACTION; DROP GRAPH IF EXISTS phantom")
            .unwrap();
        b.execute("CREATE GRAPH phantom ANY GRAPH").unwrap();
        a.run("USE GRAPH h INSERT (:N {v: 4})").await.unwrap();
        assert!(matches!(a.execute("COMMIT"), Err(Error::Conflict(_))));
        a.execute("ROLLBACK").unwrap();
        assert_eq!(
            rows(&read(&mut a, "USE GRAPH h MATCH (n) RETURN n.v AS v").await),
            &[vec!["10"]]
        );
    }
}

#[tokio::test]
async fn session_characteristics_survive_rollback_but_stale_references_never_rebind() {
    let f = Fixture::new(false);
    let mut s = f.db.session();
    s.execute("START TRANSACTION; CREATE GRAPH g ANY GRAPH; SESSION SET GRAPH g; SESSION SET VALUE $n INTEGER = 7; ROLLBACK").unwrap();
    let old = s.state().current_graph;
    assert_eq!(rows(&read(&mut s, "RETURN $n AS n").await), &[vec!["7"]]);
    s.execute("CREATE GRAPH g ANY GRAPH").unwrap();
    assert!(matches!(
        s.query("MATCH (n) RETURN COUNT(*) AS c").await,
        Err(Error::InvalidReference(_))
    ));
    assert_eq!(s.state().current_graph, old);
    s.execute("SESSION SET GRAPH g").unwrap();
    assert_ne!(s.state().current_graph, old);
    s.execute("START TRANSACTION; SESSION SET VALUE $n INTEGER = 8; COMMIT")
        .unwrap();
    assert_eq!(rows(&read(&mut s, "RETURN $n AS n").await), &[vec!["8"]]);
}

#[tokio::test]
async fn control_errors_do_not_publish_and_query_api_cannot_end_transactions() {
    let db = Database::new();
    let mut s = db.session();
    assert!(matches!(s.execute("COMMIT"), Err(Error::NoTransaction)));
    assert!(matches!(s.execute("ROLLBACK"), Err(Error::NoTransaction)));
    s.execute("START TRANSACTION; CREATE GRAPH g ANY GRAPH")
        .unwrap();
    assert!(active(&s));
    assert!(matches!(
        s.execute("START TRANSACTION"),
        Err(Error::TransactionActive)
    ));
    assert!(failed(&s));
    assert!(!exists(&db, "g"));
    s.execute("ROLLBACK; START TRANSACTION").unwrap();
    assert!(matches!(
        s.query("COMMIT").await,
        Err(Error::UnsupportedFeature(_))
    ));
    assert!(failed(&s));
    s.execute("SESSION CLOSE").unwrap();
    assert!(s.state().closed);
    db.checkpoint().unwrap();
}

#[tokio::test]
async fn import_participates_in_transactions_and_abandoned_files_are_reclaimed() {
    for durable in [false, true] {
        let f = Fixture::new(durable);
        let mut s = seeded(&f.db).await;
        f.db.checkpoint().unwrap();
        let files = || {
            f.directory.as_ref().map(|path| {
                std::fs::read_dir(path)
                    .unwrap()
                    .filter(|e| {
                        e.as_ref()
                            .unwrap()
                            .file_name()
                            .to_string_lossy()
                            .ends_with(".parquet")
                    })
                    .count()
            })
        };
        let original = files();
        s.run("START TRANSACTION; MATCH (n:N) SET n.v = 2; MATCH (n:N) SET n.v = 3")
            .await
            .unwrap();
        if durable {
            assert!(files().unwrap() > original.unwrap());
        }
        s.replace_graph_data(graphfusion::graph::GraphData::default())
            .unwrap();
        assert!(values(&mut s).await.is_empty());
        let mut other = f.db.session();
        other.execute("SESSION SET GRAPH g").unwrap();
        assert_eq!(values(&mut other).await, &[vec!["1"]]);
        s.execute("ROLLBACK").unwrap();
        f.db.checkpoint().unwrap();
        assert_eq!(files(), original);
        s.execute("START TRANSACTION").unwrap();
        s.replace_graph_data(graphfusion::graph::GraphData::default())
            .unwrap();
        s.execute("COMMIT").unwrap();
        assert!(values(&mut other).await.is_empty());
        f.db.checkpoint().unwrap();
        if durable {
            assert_eq!(files(), original.map(|n| n - 1));
        }
    }
}

#[tokio::test]
async fn controls_cannot_be_hidden_inside_atomic_next_groups() {
    let db = Database::new();
    let mut s = seeded(&db).await;
    for control in ["SESSION CLOSE", "COMMIT", "ROLLBACK", "START TRANSACTION"] {
        s.run("START TRANSACTION; INSERT (:N {v: 2})")
            .await
            .unwrap();
        assert!(s
            .execute(&format!("CREATE GRAPH hidden ANY GRAPH NEXT {control}"))
            .is_err());
        assert!(failed(&s));
        assert!(!s.state().closed);
        assert!(!exists(&db, "hidden"));
        db.checkpoint().unwrap();
        s.execute("ROLLBACK").unwrap();
        assert_eq!(values(&mut s).await, &[vec!["1"]]);
    }
}
