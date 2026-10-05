//! Measures individual durable commits and validates recovery. Run with: cargo run -p graphfusion --example wal_write_bench --release -- 1000
use graphfusion::{Database, OpenOptions, Value};
use std::{
    fs,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let rows: usize = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "1000".into())
        .parse()?;
    let path = std::env::temp_dir().join(format!(
        "graphfusion-write-bench-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    let db = Database::open(
        &path,
        OpenOptions {
            create_if_missing: true,
        },
    )?;
    let mut session = db.session();
    session.execute("CREATE GRAPH g ANY GRAPH; SESSION SET GRAPH g")?;
    let started = Instant::now();
    for i in 0..rows {
        session.set_parameter("value", Value::Integer(i as i64))?;
        session.run("INSERT (:N {value: $value})").await?;
    }
    let elapsed = started.elapsed();
    let stats = db.statistics()?;
    let mut files = 0;
    let mut bytes = 0;
    for entry in fs::read_dir(&path)? {
        let entry = entry?;
        if entry.path().extension().is_some_and(|e| e == "parquet") {
            files += 1;
            bytes += entry.metadata()?.len();
        }
    }
    drop(session);
    drop(db);
    let recovery = Instant::now();
    let db = Database::open(&path, OpenOptions::default())?;
    let recovery = recovery.elapsed();
    let result = db
        .session()
        .query("USE GRAPH g MATCH (n) RETURN n.value AS value")
        .await?;
    assert_eq!(result.row_count(), rows);
    println!("rows={rows} elapsed_ms={} rows_per_second={:.1} wal_bytes={} parquet_files={files} parquet_bytes={bytes} recovery_ms={}",
        elapsed.as_millis(), rows as f64 / elapsed.as_secs_f64(), stats.log_bytes, recovery.as_millis());
    drop(db);
    fs::remove_dir_all(&path)?;
    Ok(())
}
