use super::*;

fn buffered(dir: &TestDir, rows: usize, bytes: usize) -> Database {
    let db = dir.open();
    db.configure_storage(StorageOptions {
        memtable_max_rows: rows,
        memtable_max_bytes: bytes,
    })
    .unwrap();
    db
}

#[tokio::test]
async fn resident_masks_share_buffers_and_survive_rollback_recovery_and_sealing() {
    let dir = TestDir::new();
    {
        let db = buffered(&dir, 1000, usize::MAX);
        let mut s = db.session();
        s.execute("CREATE GRAPH g ANY GRAPH; SESSION SET GRAPH g")
            .unwrap();
        let schema = Arc::new(arrow::datatypes::Schema::new(vec![
            arrow::datatypes::Field::new(graph::ID, arrow::datatypes::DataType::UInt64, false),
            arrow::datatypes::Field::new("v", arrow::datatypes::DataType::Int64, false),
        ]));
        let batch = arrow::record_batch::RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(arrow::array::UInt64Array::from(vec![1, 2, 3])),
                Arc::new(arrow::array::Int64Array::from(vec![1, 2, 3])),
            ],
        )
        .unwrap();
        s.replace_graph_data(
            graph::GraphData::try_new(
                vec![graph::NodeTable::try_new(vec!["N".into()], schema, vec![batch]).unwrap()],
                vec![],
            )
            .unwrap(),
        )
        .unwrap();
        let id = graph_id(&db, "g").unwrap();
        let before = StatementTxn::begin(&db).unwrap().graph_data(id).unwrap();
        let buffer = before.nodes[0].0.batches[0]
            .column_by_name("v")
            .unwrap()
            .clone();
        let mut old = db.session();
        old.execute("SESSION SET GRAPH g; START TRANSACTION READ ONLY")
            .unwrap();
        assert!(s.run("MATCH (n {v: 1}) DELETE n RETURN n.v").await.is_err());
        assert_eq!(count(&mut s).await, 3);
        s.run("MATCH (n {v: 1}) DELETE n").await.unwrap();
        s.run("MATCH (n {v: 2}) SET n.v = 20").await.unwrap();
        let after = StatementTxn::begin(&db).unwrap().graph_data(id).unwrap();
        assert!(Arc::ptr_eq(
            &buffer,
            after.nodes[0].0.batches[0].column_by_name("v").unwrap()
        ));
        assert_eq!(before.node_count(), 3);
        assert_eq!(after.node_count(), 2);
        assert_eq!(after.nodes[0].0.resident.deleted[0].len(), 2);
        assert_eq!(
            property_values(&s.query("MATCH (n) RETURN count(*) AS c").await.unwrap()),
            vec![2]
        );
        assert_eq!(
            property_values(&old.query("MATCH (n) RETURN n.v AS v").await.unwrap()),
            vec![1, 2, 3]
        );
        old.execute("COMMIT").unwrap();
    }
    let db = buffered(&dir, 1000, usize::MAX);
    let mut s = db.session();
    s.execute("SESSION SET GRAPH g").unwrap();
    assert_eq!(
        property_values(&s.query("MATCH (n) RETURN n.v AS v").await.unwrap()),
        vec![3, 20]
    );
    db.checkpoint().unwrap();
    assert_eq!(
        property_values(&s.query("MATCH (n) RETURN n.v AS v").await.unwrap()),
        vec![3, 20]
    );
}

#[tokio::test]
async fn legacy_v4_manifests_without_index_sidecars_keep_physical_row_coordinates() {
    let dir = TestDir::new();
    let db = buffered(&dir, 1000, usize::MAX);
    let mut s = db.session();
    s.execute("CREATE GRAPH g ANY GRAPH; SESSION SET GRAPH g")
        .unwrap();
    s.replace_graph_data(arrow_graph(vec![90, 10, 70, 30]))
        .unwrap();
    db.checkpoint().unwrap();
    let tx = StatementTxn::begin(&db).unwrap();
    let manifest = tx
        .base
        .storage
        .generations
        .values()
        .find_map(|g| g.parquet.as_ref())
        .unwrap();
    let mut wire = serde_json::to_value(manifest).unwrap();
    wire["nodes"][0]["Parquet"]
        .as_object_mut()
        .unwrap()
        .remove("index_bytes");
    let legacy: crate::parquet::GraphManifest = serde_json::from_value(wire).unwrap();
    fs::remove_file(parquet_files(&dir)[0].with_extension("index")).unwrap();
    let disk = db.inner.disk.as_ref().unwrap();
    let mut graph = (*legacy.open(disk).unwrap()).clone();
    assert_eq!(
        graph.nodes[0]
            .0
            .mask_rows(&std::collections::BTreeSet::from([10, 70]))
            .unwrap()
            .len(),
        2
    );
    let ctx = datafusion::execution::context::SessionContext::new();
    let batches = ctx
        .read_table(graph.nodes[0].0.provider.clone())
        .unwrap()
        .collect()
        .await
        .unwrap();
    let ids: Vec<_> = batches
        .iter()
        .flat_map(|batch| {
            crate::rows::column(batch, graph::ID)
                .values()
                .iter()
                .copied()
        })
        .collect();
    assert_eq!(ids, vec![90, 30]);
}

#[tokio::test]
async fn row_index_corruption_is_rejected_and_branch_history_retains_sidecars() {
    for damage in ["missing", "truncated", "checksum"] {
        let dir = TestDir::new();
        {
            let db = buffered(&dir, 1000, usize::MAX);
            db.session()
                .run("CREATE GRAPH g ANY GRAPH; SESSION SET GRAPH g; INSERT (:N {v: 1})")
                .await
                .unwrap();
            db.checkpoint().unwrap();
        }
        let index = parquet_files(&dir)[0].with_extension("index");
        match damage {
            "missing" => fs::remove_file(index).unwrap(),
            "truncated" => fs::write(index, b"GFRIDX01").unwrap(),
            _ => {
                let mut bytes = fs::read(&index).unwrap();
                bytes[24] ^= 1;
                fs::write(index, bytes).unwrap();
            }
        }
        assert!(
            matches!(
                Database::open(&dir.0, OpenOptions::default()),
                Err(Error::Corrupt(_))
            ),
            "{damage}"
        );
    }
    let dir = TestDir::new();
    {
        let db = buffered(&dir, 1000, usize::MAX);
        let mut s = db.session();
        s.run("CREATE GRAPH g ANY GRAPH; SESSION SET GRAPH g; INSERT (:N {v: 1})")
            .await
            .unwrap();
        db.checkpoint().unwrap();
        let index = parquet_files(&dir)[0].with_extension("index");
        db.create_branch("dev").unwrap();
        s.execute("DROP GRAPH g").unwrap();
        db.checkpoint().unwrap();
        assert!(index.is_file());
        let mut dev = db.session();
        dev.set_branch("dev").unwrap();
        dev.execute("SESSION SET GRAPH g").unwrap();
        dev.run("MATCH (n {v: 1}) SET n.v = 2").await.unwrap();
    }
    let db = buffered(&dir, 1000, usize::MAX);
    let mut dev = db.session();
    dev.set_branch("dev").unwrap();
    dev.execute("SESSION SET GRAPH g").unwrap();
    assert_eq!(
        property_values(&dev.query("MATCH (n) RETURN n.v AS v").await.unwrap()),
        vec![2]
    );
    db.checkpoint().unwrap();
    assert!(parquet_files(&dir)[0].with_extension("index").is_file());
    // A separate, unreferenced generation and an interrupted index stage are reclaimed.
    let retired = TestDir::new();
    let db = buffered(&retired, 1000, usize::MAX);
    db.session()
        .run("CREATE GRAPH g ANY GRAPH; SESSION SET GRAPH g; INSERT (:N)")
        .await
        .unwrap();
    db.checkpoint().unwrap();
    fs::write(retired.0.join("graph-999999.index"), b"orphan index").unwrap();
    db.session().execute("DROP GRAPH g").unwrap();
    db.checkpoint().unwrap();
    assert!(parquet_files(&retired).is_empty());
    assert!(!fs::read_dir(&retired.0).unwrap().any(|entry| entry
        .unwrap()
        .path()
        .extension()
        .is_some_and(|e| e == "index")));
}

#[tokio::test]
async fn branch_writes_persist_incremental_records_and_replay_after_checkpoint() {
    let dir = TestDir::new();
    {
        let db = buffered(&dir, 1000, usize::MAX);
        db.session().execute("CREATE GRAPH g ANY GRAPH").unwrap();
        db.create_branch("dev").unwrap();
        let mut dev = db.session();
        dev.set_branch("dev").unwrap();
        dev.execute("SESSION SET GRAPH g").unwrap();
        for i in 0..100 {
            dev.run(&format!("INSERT (:N {{v: {i}}})")).await.unwrap();
        }
        let total: u64 = fs::read_dir(dir.0.join("commits"))
            .unwrap()
            .map(|e| e.unwrap().metadata().unwrap().len())
            .sum();
        assert!(
            total < 250_000,
            "branch snapshots amplified to {total} bytes"
        );
        assert!(parquet_files(&dir).is_empty());
        db.checkpoint().unwrap();
    }
    let db = buffered(&dir, 1000, usize::MAX);
    let mut dev = db.session();
    dev.set_branch("dev").unwrap();
    dev.execute("SESSION SET GRAPH g").unwrap();
    assert_eq!(
        property_values(&dev.query("MATCH (n) RETURN n.v AS v").await.unwrap()),
        (0..100).collect::<Vec<_>>()
    );
    dev.run("INSERT (:N {v: 100})").await.unwrap();
    assert_eq!(count(&mut dev).await, 101);
    let mut main = db.session();
    main.execute("SESSION SET GRAPH g").unwrap();
    assert_eq!(count(&mut main).await, 0);
}

#[tokio::test]
async fn branch_delta_crashes_and_uncertain_ref_sync_preserve_atomicity() {
    for (point, committed) in [
        ("branch_delta_write", false),
        ("branch_delta_sync", false),
        ("branch_ref_rename", true),
        ("branch_ref_sync", true),
        ("io", true),
    ] {
        let dir = TestDir::new();
        {
            let db = buffered(&dir, 1000, usize::MAX);
            db.session()
                .execute("CREATE GRAPH g ANY GRAPH; CREATE GRAPH doomed ANY GRAPH")
                .unwrap();
            db.create_branch("dev").unwrap();
        }
        let mut cmd = child_command(
            &dir,
            if point == "io" {
                "memtable_branch_io"
            } else {
                "memtable_branch_write"
            },
        );
        if point == "io" {
            cmd.env("GRAPHFUSION_TEST_IO", "branch_ref_sync");
        } else {
            cmd.env("GRAPHFUSION_TEST_CRASH", point);
        }
        let result = cmd.output().unwrap();
        assert_eq!(
            result.status.code(),
            Some(if point == "io" { 0 } else { 86 }),
            "{point}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let db = buffered(&dir, 1000, usize::MAX);
        let mut dev = db.session();
        dev.set_branch("dev").unwrap();
        dev.execute("SESSION SET GRAPH g").unwrap();
        assert_eq!(
            count(&mut dev).await,
            if committed { 2 } else { 0 },
            "{point}"
        );
        let mut tx = StatementTxn::begin_on(&db, "dev").unwrap();
        assert_eq!(
            tx.lookup(MAIN_SCHEMA, ObjectKind::Graph, "atomic_marker")
                .is_some(),
            committed,
            "{point}"
        );
        assert_eq!(
            tx.lookup(MAIN_SCHEMA, ObjectKind::Graph, "doomed")
                .is_none(),
            committed,
            "{point}"
        );
        assert!(graph_id(&db, "atomic_marker").is_none());
        assert!(graph_id(&db, "doomed").is_some());
        assert!(parquet_files(&dir).is_empty());
    }
}
async fn count(s: &mut Session) -> usize {
    s.query("MATCH (n) RETURN ELEMENT_ID(n) AS id")
        .await
        .unwrap()
        .row_count()
}

#[tokio::test]
async fn small_writes_replay_from_wal_without_parquet_or_snapshot_writes() {
    let dir = TestDir::new();
    {
        let db = buffered(&dir, 1000, usize::MAX);
        let mut s = db.session();
        s.execute("CREATE GRAPH g ANY GRAPH; SESSION SET GRAPH g")
            .unwrap();
        for i in 0..100 {
            s.run(&format!("INSERT (:N {{v: {i}}})")).await.unwrap();
        }
        assert_eq!(count(&mut s).await, 100);
        assert!(parquet_files(&dir).is_empty());
        assert!(
            !dir.0.join("commits").exists(),
            "main commits must avoid full snapshots"
        );
        let wal = db.statistics().unwrap().log_bytes;
        // Incremental IPC records, not cumulative table images: bounded bytes per row.
        assert!(
            wal < 250_000,
            "WAL amplified to {wal} bytes for 100 single-row writes"
        );
    }
    let db = buffered(&dir, 1000, usize::MAX);
    let mut s = db.session();
    s.execute("SESSION SET GRAPH g").unwrap();
    let values = property_values(&s.query("MATCH (n) RETURN n.v AS v").await.unwrap());
    assert_eq!(values, (0..100).collect::<Vec<_>>());
    assert!(parquet_files(&dir).is_empty());
    db.checkpoint().unwrap();
    assert_eq!(parquet_files(&dir).len(), 1);
    assert!(s
        .query("MATCH (n) RETURN n.v AS v")
        .await
        .unwrap()
        .physical_plan
        .contains("file_type=parquet"));
}

#[tokio::test]
async fn row_threshold_seals_only_the_new_fragment_and_old_snapshots_stay_consistent() {
    let dir = TestDir::new();
    let db = buffered(&dir, 3, usize::MAX);
    let mut s = db.session();
    s.run("CREATE GRAPH g ANY GRAPH; SESSION SET GRAPH g; INSERT (:N {v: 0}); INSERT (:N {v: 1})")
        .await
        .unwrap();
    assert!(parquet_files(&dir).is_empty());
    let mut old = db.session();
    old.execute("SESSION SET GRAPH g; START TRANSACTION READ ONLY")
        .unwrap();
    s.run("INSERT (:N {v: 2})").await.unwrap();
    let original = parquet_files(&dir);
    assert_eq!(original.len(), 1);
    let before = fs::read(&original[0]).unwrap();
    assert_eq!(count(&mut old).await, 2);
    assert_eq!(count(&mut s).await, 3);
    let result = s.run("INSERT (n:N {v: 3}) RETURN n.v AS v").await.unwrap();
    let StatementOutput::Query(result) = &result[0] else {
        panic!("query")
    };
    assert!(
        !result.physical_plan.contains("file_type=parquet"),
        "append scanned old files: {}",
        result.physical_plan
    );
    assert_eq!(parquet_files(&dir).len(), 1);
    assert_eq!(count(&mut s).await, 4);
    let mixed = s.query("MATCH (n) RETURN n.v AS v").await.unwrap();
    assert!(mixed.physical_plan.contains("file_type=parquet"));
    assert!(mixed.physical_plan.contains("partition_sizes="));
    s.run("INSERT (:N {v: 4}); INSERT (:N {v: 5})")
        .await
        .unwrap();
    assert_eq!(parquet_files(&dir).len(), 2);
    assert_eq!(fs::read(&original[0]).unwrap(), before);
    let row_counts: Vec<_> = parquet_files(&dir)
        .iter()
        .map(|p| {
            datafusion::parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder::try_new(
                fs::File::open(p).unwrap(),
            )
            .unwrap()
            .metadata()
            .file_metadata()
            .num_rows()
        })
        .collect();
    assert_eq!(row_counts, vec![3, 3]);
    assert_eq!(count(&mut old).await, 2);
    assert!(matches!(db.checkpoint(), Err(Error::Busy)));
    old.execute("COMMIT").unwrap();
    db.checkpoint().unwrap();
    drop(s);
    drop(old);
    drop(db);
    let db = buffered(&dir, 3, usize::MAX);
    let mut s = db.session();
    s.execute("SESSION SET GRAPH g").unwrap();
    assert_eq!(
        property_values(&s.query("MATCH (n) RETURN n.v AS v").await.unwrap()),
        (0..6).collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn byte_threshold_and_explicit_transactions_seal_at_commit_only() {
    let dir = TestDir::new();
    let db = buffered(&dir, usize::MAX, 1);
    let mut s = db.session();
    s.execute("CREATE GRAPH g ANY GRAPH; SESSION SET GRAPH g; START TRANSACTION")
        .unwrap();
    s.run("INSERT (:N {v: 1}); INSERT (:N {v: 2})")
        .await
        .unwrap();
    assert!(parquet_files(&dir).is_empty());
    assert_eq!(count(&mut s).await, 2);
    let mut other = db.session();
    other.execute("SESSION SET GRAPH g").unwrap();
    assert_eq!(count(&mut other).await, 0);
    s.execute("ROLLBACK").unwrap();
    assert!(parquet_files(&dir).is_empty());
    s.run("START TRANSACTION; INSERT (:N {v: 3}); INSERT (:N {v: 4}); COMMIT")
        .await
        .unwrap();
    assert_eq!(parquet_files(&dir).len(), 1);
    assert_eq!(
        property_values(&other.query("MATCH (n) RETURN n.v AS v").await.unwrap()),
        vec![3, 4]
    );
}

#[tokio::test]
async fn wal_replay_preserves_updates_deletes_edges_and_branch_snapshots() {
    let dir = TestDir::new();
    {
        let db = buffered(&dir, 1000, usize::MAX);
        let mut s = db.session();
        s.run(
            "CREATE GRAPH g ANY GRAPH; SESSION SET GRAPH g; INSERT (a:N {v: 1})-[:E]->(b:N {v: 2})",
        )
        .await
        .unwrap();
        db.create_branch("dev").unwrap();
        s.run("MATCH (n {v: 1}) SET n.v = 3; MATCH (n {v: 2}) DETACH DELETE n; INSERT (:N {v: 4})")
            .await
            .unwrap();
        let mut dev = db.session();
        dev.set_branch("dev").unwrap();
        dev.execute("SESSION SET GRAPH g").unwrap();
        dev.run("INSERT (:N {v: 5})").await.unwrap();
        assert!(parquet_files(&dir).is_empty());
    }
    let db = buffered(&dir, 1000, usize::MAX);
    let mut s = db.session();
    s.execute("SESSION SET GRAPH g").unwrap();
    assert_eq!(
        property_values(&s.query("MATCH (n) RETURN n.v AS v").await.unwrap()),
        vec![3, 4]
    );
    assert_eq!(
        s.query("MATCH ()-[e]->() RETURN ELEMENT_ID(e) AS id")
            .await
            .unwrap()
            .row_count(),
        0
    );
    let mut dev = db.session();
    dev.set_branch("dev").unwrap();
    dev.execute("SESSION SET GRAPH g").unwrap();
    assert_eq!(
        property_values(&dev.query("MATCH (n) RETURN n.v AS v").await.unwrap()),
        vec![1, 2, 5]
    );
    db.checkpoint().unwrap();
    assert_eq!(
        s.query("MATCH (n) RETURN n.v AS v")
            .await
            .unwrap()
            .row_count(),
        2
    );
    assert_eq!(count(&mut dev).await, 3);
}

#[tokio::test]
async fn memtable_crash_and_io_matrix_replays_only_atomic_commits() {
    for (point, committed) in [
        ("memtable_prepare", false),
        ("memtable_wal_header", false),
        ("memtable_wal_payload", false),
        ("memtable_wal_commit", true),
        ("memtable_wal_sync", true),
        ("before_publish", true),
        ("io", true),
    ] {
        let dir = TestDir::new();
        {
            let db = buffered(&dir, 1000, usize::MAX);
            db.session()
                .execute("CREATE GRAPH g ANY GRAPH; CREATE GRAPH doomed ANY GRAPH")
                .unwrap();
        }
        let mut cmd = child_command(
            &dir,
            if point == "io" {
                "memtable_io"
            } else {
                "memtable_write"
            },
        );
        if point == "io" {
            cmd.env("GRAPHFUSION_TEST_IO", "memtable_wal_sync");
        } else {
            cmd.env("GRAPHFUSION_TEST_CRASH", point);
        }
        let result = cmd.output().unwrap();
        assert_eq!(
            result.status.code(),
            Some(if point == "io" { 0 } else { 86 }),
            "{point}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let db = buffered(&dir, 1000, usize::MAX);
        let mut s = db.session();
        s.execute("SESSION SET GRAPH g").unwrap();
        assert_eq!(
            count(&mut s).await,
            if committed { 2 } else { 0 },
            "{point}"
        );
        assert_eq!(
            graph_id(&db, "atomic_marker").is_some(),
            committed,
            "{point}"
        );
        assert_eq!(graph_id(&db, "doomed").is_none(), committed, "{point}");
        assert!(parquet_files(&dir).is_empty(), "{point}");
    }
}

#[tokio::test]
async fn sealed_updates_and_deletes_log_deltas_without_rewriting_the_base_file() {
    let dir = TestDir::new();
    let db = buffered(&dir, 25, usize::MAX);
    let mut s = db.session();
    s.execute("CREATE GRAPH g ANY GRAPH; SESSION SET GRAPH g")
        .unwrap();
    for i in 0..25 {
        s.run(&format!("INSERT (:N {{v: {i}}})")).await.unwrap();
    }
    let files = parquet_files(&dir);
    assert_eq!(files.len(), 1);
    let original = fs::read(&files[0]).unwrap();
    db.configure_storage(StorageOptions {
        memtable_max_rows: 1000,
        memtable_max_bytes: usize::MAX,
    })
    .unwrap();
    let mut old = db.session();
    old.execute("SESSION SET GRAPH g; START TRANSACTION READ ONLY")
        .unwrap();
    for i in 0..10 {
        s.run(&format!("MATCH (n {{v: {i}}}) SET n.v = {}", 100 + i))
            .await
            .unwrap();
    }
    s.run("MATCH (n {v: 24}) DELETE n").await.unwrap();
    assert_eq!(parquet_files(&dir), files);
    assert_eq!(fs::read(&files[0]).unwrap(), original);
    assert_eq!(
        property_values(&old.query("MATCH (n) RETURN n.v AS v").await.unwrap()),
        (0..25).collect::<Vec<_>>()
    );
    let expected: Vec<_> = (10..24).chain(100..110).collect();
    assert_eq!(
        property_values(&s.query("MATCH (n) RETURN n.v AS v").await.unwrap()),
        expected
    );
    old.execute("COMMIT").unwrap();
    drop(old);
    drop(s);
    drop(db);
    let db = buffered(&dir, 1000, usize::MAX);
    let mut s = db.session();
    s.execute("SESSION SET GRAPH g").unwrap();
    assert_eq!(
        property_values(&s.query("MATCH (n) RETURN n.v AS v").await.unwrap()),
        expected
    );
    db.checkpoint().unwrap();
    assert_eq!(
        parquet_files(&dir).len(),
        2,
        "updates with one layout should seal as one fragment"
    );
    assert_eq!(
        property_values(&s.query("MATCH (n) RETURN n.v AS v").await.unwrap()),
        expected
    );
}

#[tokio::test]
async fn checkpoint_crashes_preserve_unsealed_wal_data_and_allow_future_writes() {
    for point in [
        "parquet_write",
        "parquet_sync",
        "row_index_write",
        "row_index_sync",
        "parquet_directory",
        "checkpoint_catalog",
        "checkpoint_data",
        "manifest_rename",
        "manifest_sync",
        "cleanup",
        "parquet_cleanup",
    ] {
        let dir = TestDir::new();
        {
            let db = buffered(&dir, 1000, usize::MAX);
            db.session().run("CREATE GRAPH g ANY GRAPH; SESSION SET GRAPH g; INSERT (:N {v: 1}); INSERT (:N {v: 2})").await.unwrap();
        }
        if point == "parquet_cleanup" {
            fs::write(dir.0.join("graph-999999.parquet"), b"unpublished orphan").unwrap();
        }
        let result = child_command(&dir, "memtable_checkpoint")
            .env("GRAPHFUSION_TEST_CRASH", point)
            .output()
            .unwrap();
        {
            assert_eq!(
                result.status.code(),
                Some(86),
                "{point}: {}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
        let db = buffered(&dir, 1000, usize::MAX);
        let mut s = db.session();
        s.execute("SESSION SET GRAPH g").unwrap();
        assert_eq!(
            property_values(&s.query("MATCH (n) RETURN n.v AS v").await.unwrap()),
            vec![1, 2],
            "{point}"
        );
        s.run("INSERT (:N {v: 3})").await.unwrap();
        db.checkpoint().unwrap();
        assert_eq!(count(&mut s).await, 3, "{point}");
    }
}

#[tokio::test]
async fn branches_fork_the_current_checkpoint_layout_after_generation_reclamation() {
    let dir = TestDir::new();
    let db = buffered(&dir, 1, usize::MAX);
    let mut s = db.session();
    s.run("CREATE GRAPH g ANY GRAPH; SESSION SET GRAPH g; INSERT (:N {v: 1})")
        .await
        .unwrap();
    s.execute("DROP GRAPH g").unwrap();
    db.branches().unwrap(); // Persist a snapshot containing the now-retired generation.
    db.checkpoint().unwrap();
    assert!(parquet_files(&dir).is_empty());
    db.create_branch("clean").unwrap();
    let mut branch = db.session();
    branch.set_branch("clean").unwrap();
    branch.query("RETURN 1 AS n").await.unwrap();
}

#[tokio::test]
async fn resident_updates_log_only_changed_rows_and_prune_empty_batches() {
    use arrow::{
        array::{Int64Array, UInt64Array},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    };
    let dir = TestDir::new();
    {
        let db = buffered(&dir, 10000, usize::MAX);
        let mut s = db.session();
        s.execute("CREATE GRAPH g ANY GRAPH; SESSION SET GRAPH g")
            .unwrap();
        let schema = Arc::new(Schema::new(vec![
            Field::new(graph::ID, DataType::UInt64, false),
            Field::new("v", DataType::Int64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(UInt64Array::from((0..1000).collect::<Vec<_>>())),
                Arc::new(Int64Array::from((0..1000).collect::<Vec<_>>())),
            ],
        )
        .unwrap();
        s.replace_graph_data(
            graph::GraphData::try_new(
                vec![graph::NodeTable::try_new(vec!["N".into()], schema, vec![batch]).unwrap()],
                vec![],
            )
            .unwrap(),
        )
        .unwrap();
        let before = db.statistics().unwrap().log_bytes;
        for i in 0..100 {
            s.run(&format!(
                "MATCH (n {{v: {}}}) SET n.v = {}",
                if i == 0 { 0 } else { 999 + i },
                1000 + i
            ))
            .await
            .unwrap();
        }
        // Repeated updates to the same identity must not append every complete 1000-row image.
        let bytes = db.statistics().unwrap().log_bytes - before;
        assert!(
            bytes < 200_000,
            "resident update WAL amplified to {bytes} bytes"
        );
        assert!(parquet_files(&dir).is_empty());
        let tx = StatementTxn::begin(&db).unwrap();
        let fragments = tx
            .base
            .storage
            .generations
            .values()
            .flat_map(|g| g.parquet.iter())
            .flat_map(|m| m.fragments());
        for fragment in fragments {
            if let memtable::Fragment::Memory(table) = fragment {
                assert!(table.data.batches.len() <= 2);
            }
        }
        // Multiple replacements of one resident row in one transaction replay removals before
        // its final appended version; old versions and temporary inserted rows stay absent.
        s.run("START TRANSACTION; MATCH (n {v: 1099}) SET n.v = 2000; MATCH (n {v: 2000}) SET n.v = 2001; INSERT (n:N {v: 9999}) DELETE n; COMMIT").await.unwrap();
        assert_eq!(count(&mut s).await, 1000);
    }
    let db = buffered(&dir, 10000, usize::MAX);
    let mut s = db.session();
    s.execute("SESSION SET GRAPH g").unwrap();
    let values = property_values(&s.query("MATCH (n) RETURN n.v AS v").await.unwrap());
    let expected: Vec<_> = (1..1000).chain([2001]).collect();
    assert_eq!(values, expected);
    db.checkpoint().unwrap();
    assert_eq!(parquet_files(&dir).len(), 1);
}
