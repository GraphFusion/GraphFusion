//! Compare mutation costs in memory, WAL-resident and sealed storage.
//! Run: cargo run -p graphfusion --example mutation_bench --release -- 100000 3
//! Optional arguments: storage mode (all/memory/resident/parquet), operation.
use graphfusion::arrow::{
    array::{Int64Array, StringArray, UInt64Array},
    datatypes::{DataType, Field, Schema},
    record_batch::RecordBatch,
};
use graphfusion::graph::{EdgeTable, GraphData, NodeTable, DESTINATION, ID, SOURCE};
use graphfusion::{Database, OpenOptions, StatementOutput, StorageOptions};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};

type AnyError = Box<dyn std::error::Error>;
fn fixture(n: usize) -> Result<GraphData, AnyError> {
    let node_schema = Arc::new(Schema::new(vec![
        Field::new(ID, DataType::UInt64, false),
        Field::new("v", DataType::Int64, false),
        Field::new("payload", DataType::Utf8, false),
    ]));
    let node_batch = RecordBatch::try_new(
        node_schema.clone(),
        vec![
            Arc::new(UInt64Array::from_iter_values(0..n as u64)),
            Arc::new(Int64Array::from_iter_values(0..n as i64)),
            Arc::new(StringArray::from_iter_values(std::iter::repeat_n(
                "x".repeat(128),
                n,
            ))),
        ],
    )?;
    let e = n - 2;
    let edge_schema = Arc::new(Schema::new(vec![
        Field::new(ID, DataType::UInt64, false),
        Field::new(SOURCE, DataType::UInt64, false),
        Field::new(DESTINATION, DataType::UInt64, false),
        Field::new("v", DataType::Int64, false),
        Field::new("payload", DataType::Utf8, false),
    ]));
    let edge_batch = RecordBatch::try_new(
        edge_schema.clone(),
        vec![
            Arc::new(UInt64Array::from_iter_values(n as u64..(n + e) as u64)),
            Arc::new(UInt64Array::from_iter_values(0..e as u64)),
            Arc::new(UInt64Array::from_iter_values(1..e as u64 + 1)),
            Arc::new(Int64Array::from_iter_values(0..e as i64)),
            Arc::new(StringArray::from_iter_values(std::iter::repeat_n(
                "x".repeat(128),
                e,
            ))),
        ],
    )?;
    Ok(GraphData::try_new(
        vec![NodeTable::try_new(
            vec!["N".into()],
            node_schema,
            vec![node_batch],
        )?],
        vec![EdgeTable::try_new(
            vec!["E".into()],
            true,
            edge_schema,
            vec![edge_batch],
        )?],
    )?)
}
async fn run_case(root: &Path, n: usize, mode: &str, op: &str, rep: usize) -> Result<(), AnyError> {
    let path = root.to_path_buf().join(format!("db-{n}-{mode}-{op}-{rep}"));
    let db = if mode == "memory" {
        Database::new()
    } else {
        let db = Database::open(
            &path,
            OpenOptions {
                create_if_missing: true,
            },
        )?;
        db.configure_storage(StorageOptions {
            memtable_max_rows: usize::MAX,
            memtable_max_bytes: usize::MAX,
        })?;
        db
    };
    let mut s = db.session();
    s.execute("CREATE GRAPH g ANY GRAPH; SESSION SET GRAPH g")?;
    s.replace_graph_data(fixture(n)?)?;
    if mode == "parquet" {
        db.checkpoint()?;
    }
    // Warm filesystem metadata and data cache; setup and warmup are excluded.
    s.query("MATCH (n) RETURN count(n) AS c").await?;
    s.query("MATCH ()-[e]->() RETURN count(e) AS c").await?;
    let sql = match op {
        "set_node" => "MATCH (n {v: 0}) SET n.v = -1".to_owned(),
        "set_edge" => "MATCH ()-[e {v: 0}]->() SET e.v = -1".to_owned(),
        "delete_edge" => "MATCH ()-[e {v: 0}]->() DELETE e".to_owned(),
        "delete_isolated" => format!("MATCH (n {{v: {}}}) DELETE n", n - 1),
        "detach_one" => "MATCH (n {v: 0}) DETACH DELETE n".to_owned(),
        "detach_100" => "MATCH (n) WHERE n.v < 100 DETACH DELETE n".to_owned(),
        "detach_1000" => "MATCH (n) WHERE n.v < 1000 DETACH DELETE n".to_owned(),
        "delete_no_match" => "MATCH (n {v: -999}) DELETE n".to_owned(),
        _ => unreachable!(),
    };
    let before = if mode == "memory" {
        0
    } else {
        db.statistics()?.log_bytes
    };
    let t = Instant::now();
    let mut outputs = s.run(&sql).await?;
    let ms = t.elapsed().as_secs_f64() * 1000.0;
    let after = if mode == "memory" {
        0
    } else {
        db.statistics()?.log_bytes
    };
    let Some(StatementOutput::Query(r)) = outputs.pop() else {
        panic!("missing query result")
    };
    let plans = r
        .physical_plan
        .lines()
        .filter(|line| line.contains("NestedLoopJoinExec"))
        .collect::<Vec<_>>();
    println!(
        "n={n} e={} mode={mode} op={op} rep={rep} ms={ms:.3} wal_delta={} affected={} nested={}",
        n - 2,
        after - before,
        r.affected_elements,
        plans.len()
    );
    if n == 1000 && mode == "memory" && op == "detach_one" && rep == 0 {
        for plan in plans {
            println!("PLAN {}", plan.trim());
        }
    }
    let expected = match op {
        "set_node" | "set_edge" | "delete_edge" | "delete_isolated" => 1,
        "detach_one" => 2,
        "detach_100" => n.min(100) + (n - 2).min(100),
        "detach_1000" => n.min(1000) + (n - 2).min(1000),
        "delete_no_match" => 0,
        _ => 0,
    };
    assert_eq!(r.affected_elements, expected);
    drop(s);
    drop(db);
    if mode != "memory" {
        std::fs::remove_dir_all(path)?;
    }
    Ok(())
}
struct Directory(PathBuf);
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
async fn execute() -> Result<(), AnyError> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let n: usize = args.first().map_or("100000", String::as_str).parse()?;
    let repeats: usize = args.get(1).map_or("3", String::as_str).parse()?;
    let mode = args.get(2).map_or("all", String::as_str);
    let operation = args.get(3).map_or("all", String::as_str);
    let operations = [
        "set_node",
        "set_edge",
        "delete_edge",
        "delete_isolated",
        "detach_one",
        "delete_no_match",
        "detach_100",
        "detach_1000",
    ];
    if n < 3
        || repeats == 0
        || !["all", "memory", "resident", "parquet"].contains(&mode)
        || (operation != "all" && !operations.contains(&operation))
    {
        return Err("expected: nodes >= 3, repeats > 0, valid mode and operation".into());
    }
    let root = Directory(std::env::temp_dir().join(format!(
            "graphfusion-mutation-bench-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        )));
    std::fs::create_dir(&root.0)?;
    for storage in ["memory", "resident", "parquet"]
        .into_iter()
        .filter(|m| mode == "all" || mode == *m)
    {
        for op in operations.into_iter().filter(|op| {
            if operation == "all" {
                !op.starts_with("detach_100")
            } else {
                operation == *op
            }
        }) {
            for rep in 0..repeats {
                run_case(&root.0, n, storage, op, rep).await?;
            }
        }
    }
    Ok(())
}
fn main() -> Result<(), AnyError> {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()?
        .block_on(execute())
}
