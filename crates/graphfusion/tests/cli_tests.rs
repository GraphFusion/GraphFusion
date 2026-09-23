use datafusion::parquet::arrow::ArrowWriter;
use graphfusion::arrow::{
    array::{ArrayRef, StringArray, UInt64Array},
    datatypes::{DataType, Field, Schema},
    record_batch::RecordBatch,
};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::Arc,
};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "graphfusion-cli {} % # {}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&path).unwrap();
        let nodes = Self::batch(
            &["__gf_id", "name"],
            vec![
                Arc::new(UInt64Array::from(vec![1, 2, 3])),
                Arc::new(StringArray::from(vec!["Alice", "Bob", "Cara"])),
            ],
        );
        let edges = Self::batch(
            &["__gf_id", "__gf_source", "__gf_destination"],
            vec![
                Arc::new(UInt64Array::from(vec![10, 11])),
                Arc::new(UInt64Array::from(vec![1, 2])),
                Arc::new(UInt64Array::from(vec![2, 3])),
            ],
        );
        for (name, batch) in [("nodes.parquet", nodes), ("edges.parquet", edges)] {
            let mut writer = ArrowWriter::try_new(
                fs::File::create(path.join(name)).unwrap(),
                batch.schema(),
                None,
            )
            .unwrap();
            writer.write(&batch).unwrap();
            writer.close().unwrap();
        }
        fs::write(path.join("import.json"), r#"{"nodes":[{"labels":["Person","Named"],"file":"nodes.parquet"}],"edges":[{"labels":["Knows"],"directed":true,"file":"edges.parquet"}]}"#).unwrap();
        Self(path)
    }
    fn batch(names: &[&str], arrays: Vec<ArrayRef>) -> RecordBatch {
        let schema = Arc::new(Schema::new(
            names
                .iter()
                .zip(&arrays)
                .map(|(name, a)| Field::new(*name, a.data_type().clone(), false))
                .collect::<Vec<_>>(),
        ));
        RecordBatch::try_new(schema, arrays).unwrap()
    }
    fn invoke(&self, command: &str, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_graphfusion"))
            .arg(command)
            .arg("--database")
            .arg(self.0.join("db"))
            .args(args)
            .current_dir(&self.0)
            .output()
            .unwrap()
    }
    fn run(&self, command: &str, args: &[&str]) -> String {
        let output = self.invoke(command, args);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn cli_import_query_and_checkpoint_survive_independent_processes() {
    let fixture = Fixture::new();
    fs::write(fixture.0.join("setup.gql"), "CREATE GRAPH social ANY GRAPH; SESSION SET GRAPH social; RETURN 'text;--still text' AS value;").unwrap();
    assert!(fixture
        .run("run", &["--create", "--file", "setup.gql"])
        .contains("text;--still text"));
    assert!(fixture
        .run(
            "import",
            &["--graph", "social", "--manifest", "import.json"]
        )
        .contains("nodes=3 edges=2"));
    let query = "USE GRAPH social MATCH (a:Person {name: 'Alice'})-[:Knows]->(b)-[:Knows]->(c) RETURN c.name AS friend";
    let output = fixture.run("run", &["--query", query, "--explain"]);
    assert!(output.contains("Cara"), "{output}");
    assert!(output.contains("file_type=parquet"), "{output}");
    assert!(output.contains("HashJoinExec"), "{output}");
    fixture.run("checkpoint", &[]);
    assert!(fixture.run("run", &["--query", query]).contains("Cara"));
    // Imported files are copied into managed storage; source deletion cannot break the graph.
    fs::remove_file(fixture.0.join("nodes.parquet")).unwrap();
    fs::remove_file(fixture.0.join("edges.parquet")).unwrap();
    assert!(fixture.run("run", &["--query", query]).contains("Cara"));
    let empty = fixture.run(
        "run",
        &[
            "--query",
            "USE GRAPH social MATCH (n:Missing) RETURN n.name AS empty",
        ],
    );
    assert!(empty.contains("empty"));
}

#[test]
fn cli_errors_do_not_replace_graphs_or_hide_partial_commit_semantics() {
    let fixture = Fixture::new();
    let bad = fixture.invoke("run", &["--create", "--query", "CREATE GRAPH"]);
    assert!(!bad.status.success());
    assert!(!fixture.0.join("db").exists());
    fixture.run(
        "run",
        &["--create", "--query", "CREATE GRAPH social ANY GRAPH"],
    );
    fixture.run(
        "import",
        &["--graph", "social", "--manifest", "import.json"],
    );
    assert!(!fixture
        .invoke(
            "import",
            &[
                "--graph",
                "social; DROP GRAPH social",
                "--manifest",
                "import.json"
            ]
        )
        .status
        .success());
    let schema = Arc::new(Schema::new(vec![Field::new(
        "wrong",
        DataType::UInt64,
        false,
    )]));
    let batch =
        RecordBatch::try_new(schema.clone(), vec![Arc::new(UInt64Array::from(vec![4]))]).unwrap();
    let mut writer = ArrowWriter::try_new(
        fs::File::create(fixture.0.join("nodes.parquet")).unwrap(),
        schema,
        None,
    )
    .unwrap();
    writer.write(&batch).unwrap();
    writer.close().unwrap();
    assert!(!fixture
        .invoke(
            "import",
            &["--graph", "social", "--manifest", "import.json"]
        )
        .status
        .success());
    let rows = fixture.run(
        "run",
        &[
            "--query",
            "USE GRAPH social MATCH (n:Person) RETURN n.name AS name",
        ],
    );
    for name in ["Alice", "Bob", "Cara"] {
        assert!(rows.contains(name));
    }
    assert!(!fixture
        .invoke(
            "run",
            &[
                "--query",
                "CREATE GRAPH committed ANY GRAPH; RETURN missing"
            ]
        )
        .status
        .success());
    fixture.run(
        "run",
        &[
            "--query",
            "USE GRAPH committed MATCH (n) RETURN ELEMENT_ID(n) AS id",
        ],
    );
    assert!(!fixture
        .invoke(
            "run",
            &[
                "--query",
                "CREATE GRAPH rejected ANY GRAPH; START TRANSACTION"
            ]
        )
        .status
        .success());
    assert!(!fixture
        .invoke(
            "run",
            &[
                "--query",
                "USE GRAPH rejected MATCH (n) RETURN ELEMENT_ID(n) AS id"
            ]
        )
        .status
        .success());
}

#[test]
fn cli_usage_and_in_memory_program() {
    let output = Command::new(env!("CARGO_BIN_EXE_graphfusion"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(output.status.success());
    let output = Command::new(env!("CARGO_BIN_EXE_graphfusion"))
        .args([
            "run",
            "--query",
            "SESSION SET VALUE $x INTEGER = 40; RETURN $x + 2 AS result",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("42"));
    let fixture = Fixture::new();
    assert!(!fixture
        .invoke(
            "run",
            &["--create", "--file", "missing", "--query", "RETURN 1 AS n"]
        )
        .status
        .success());
    assert!(!Path::new(&fixture.0.join("db")).exists());
}
