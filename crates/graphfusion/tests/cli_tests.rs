use datafusion::parquet::arrow::ArrowWriter;
use graphfusion::arrow::{
    array::{ArrayRef, StringArray, UInt64Array},
    datatypes::{DataType, Field, Schema},
    record_batch::RecordBatch,
};
use graphfusion::gql;
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::Arc,
};

struct Fixture(PathBuf);
impl Fixture {
    fn repl(&self, input: &str) -> Output {
        self.interactive(&["db"], input)
    }
    fn interactive(&self, args: &[&str], input: &str) -> Output {
        let mut child = Command::new(env!("CARGO_BIN_EXE_graphfusion"))
            .args(args)
            .current_dir(&self.0)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "graphfusion-cli {} % # [data] * ? [ {}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
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

#[test]
fn run_dump_ast_accepts_queries_and_files_without_executing() {
    let fixture = Fixture::new();
    let input =
        "CREATE GRAPH ast_only ANY GRAPH; START TRANSACTION; MATCH (p:Person) RETURN p.name AS name;";
    fs::write(fixture.0.join("inspect query.gql"), input).unwrap();
    let initial_files = fs::read_dir(&fixture.0).unwrap().count();
    let expected = gql::format_ast(&gql::parse(input).unwrap());
    for args in [
        vec!["run", "--dump-ast", "--query", input],
        vec!["run", "--file", "inspect query.gql", "--dump-ast"],
    ] {
        let output = fixture.interactive(&args, "");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(String::from_utf8(output.stdout).unwrap(), expected);
        assert!(output.stderr.is_empty());
        assert_eq!(fs::read_dir(&fixture.0).unwrap().count(), initial_files);
    }
}

#[test]
fn dump_ast_rejects_execution_options_and_invalid_inputs() {
    let fixture = Fixture::new();
    let initial_files = fs::read_dir(&fixture.0).unwrap().count();
    let invalid: &[&[&str]] = &[
        &["--dump-ast"],
        &["--dump-ast", "db"],
        &["db", "--dump-ast"],
        &["--database", "db", "--dump-ast"],
        &["--dump-ast", "--explain"],
        &["--dump-ast", "--dump-ast"],
        &["run", "--dump-ast", "--create", "--query", "RETURN 1 AS n"],
        &[
            "run",
            "--database",
            "db",
            "--dump-ast",
            "--query",
            "RETURN 1 AS n",
        ],
        &["run", "--dump-ast", "--explain", "--query", "RETURN 1 AS n"],
        &["run", "--dump-ast"],
        &[
            "run",
            "--dump-ast",
            "--dump-ast",
            "--query",
            "RETURN 1 AS n",
        ],
        &["run", "--dump-ast", "--query", "RETURN 1 AS n; RETURN @"],
        &["run", "--dump-ast", "--query", "RETURN 'unfinished"],
        &["run", "--dump-ast", "--file", "missing.gql"],
        &[
            "run",
            "--dump-ast",
            "--file",
            "missing.gql",
            "--query",
            "RETURN 1 AS n",
        ],
        &["import", "--dump-ast"],
        &["checkpoint", "--dump-ast"],
    ];
    for args in invalid {
        let output = fixture.interactive(args, "");
        assert!(!output.status.success(), "{args:?}");
        assert!(output.stdout.is_empty(), "{args:?}");
        assert!(!output.stderr.is_empty(), "{args:?}");
        assert_eq!(fs::read_dir(&fixture.0).unwrap().count(), initial_files);
    }
}

#[test]
fn repl_keeps_graph_parameters_and_transactions_between_inputs() {
    let fixture = Fixture::new();
    let output = fixture.repl("\\help\nCREATE GRAPH g ANY GRAPH;\nSESSION SET GRAPH g;\nSESSION SET VALUE $name STRING = 'Alice;--still text';\nSTART TRANSACTION;\nINSERT (:Person {name: $name});\nMATCH (p:Person)\nRETURN p.name AS name; -- comment;\nCOMMIT;\n\\graphs\n\\checkpoint\n\\q\n");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("Alice;--still text"), "{stdout}");
    assert!(stdout.contains("pending_snapshot="), "{stdout}");
    assert!(stdout.contains("transaction=committed"), "{stdout}");
    assert!(stdout.contains("OK checkpoint"), "{stdout}");
    let persisted = fixture.run(
        "run",
        &["--query", "USE GRAPH g MATCH (n) RETURN n.name AS name"],
    );
    assert!(persisted.contains("Alice;--still text"));
}

#[test]
fn interactive_entry_defaults_to_memory_and_opens_or_creates_a_directory() {
    let fixture = Fixture::new();
    let initial_files = fs::read_dir(&fixture.0).unwrap().count();
    let memory_args: &[&[&str]] = &[&[], &[":memory:"], &["--database", ":memory:"]];
    for args in memory_args {
        let output = fixture.interactive(args, "RETURN 42 AS answer;\n\\quit\n");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("42"));
    }
    assert_eq!(fs::read_dir(&fixture.0).unwrap().count(), initial_files);
    let created = fixture.repl("CREATE GRAPH g ANY GRAPH;\n\\quit\n");
    assert!(
        created.status.success(),
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );
    assert!(fixture.0.join("db/MANIFEST").is_file());
    let reopened = fixture.repl("USE GRAPH g RETURN 42 AS answer;\n\\quit\n");
    assert!(
        reopened.status.success(),
        "{}",
        String::from_utf8_lossy(&reopened.stderr)
    );
    assert!(String::from_utf8_lossy(&reopened.stdout).contains("42"));
}

#[test]
fn interactive_paths_support_options_spaces_and_command_names() {
    let fixture = Fixture::new();
    let cases: &[(&[&str], &str)] = &[
        (&["--explain", "with spaces"], "with spaces"),
        (&["with spaces", "--explain"], "with spaces"),
        (
            &["--database", "named directory", "--explain"],
            "named directory",
        ),
        (&["--", "-data"], "-data"),
        (&["--", "run"], "run"),
        (&["./run"], "run"),
        (&["--database", "run"], "run"),
    ];
    for (args, directory) in cases {
        let output = fixture.interactive(args, "RETURN 42 AS answer;\n\\quit\n");
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("42"), "{stdout}");
        if args.contains(&"--explain") {
            assert!(stdout.contains("Logical plan:"), "{stdout}");
        }
        assert!(fixture.0.join(directory).join("MANIFEST").is_file());
    }
}

#[test]
fn invalid_interactive_arguments_and_help_do_not_create_databases() {
    let fixture = Fixture::new();
    let initial_files = fs::read_dir(&fixture.0).unwrap().count();
    let invalid: &[&[&str]] = &[
        &["db", "second"],
        &["db", "--database", "second"],
        &["--database", "db", "second"],
        &["db", "--unknown"],
        &["db", "--create"],
        &["--database"],
        &["--database", "--explain"],
        &["db", "--explain", "--explain"],
        &["db", "--file", "missing"],
        &[""],
    ];
    for args in invalid {
        let output = fixture.interactive(args, "");
        assert!(!output.status.success(), "{args:?}");
        assert_eq!(
            fs::read_dir(&fixture.0).unwrap().count(),
            initial_files,
            "{args:?}"
        );
    }
    for help in ["-h", "--help"] {
        let output = fixture.interactive(&["db", help], "");
        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).contains("graphfusion [DIR]"));
        assert_eq!(fs::read_dir(&fixture.0).unwrap().count(), initial_files);
    }
}

#[test]
fn idle_repl_holds_database_until_exit() {
    let fixture = Fixture::new();
    let mut child = Command::new(env!("CARGO_BIN_EXE_graphfusion"))
        .arg(fixture.0.join("db"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    writeln!(input, "CREATE GRAPH ready ANY GRAPH;").unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut ready = String::new();
    stdout.read_line(&mut ready).unwrap();
    let rejected = fixture.invoke("run", &["--query", "USE GRAPH ready RETURN 1 AS n"]);
    writeln!(input, "\\quit").unwrap();
    drop(input);
    let exited = child.wait_with_output().unwrap();
    assert!(ready.contains("OK commit="), "{ready}");
    assert!(
        exited.status.success(),
        "{}",
        String::from_utf8_lossy(&exited.stderr)
    );
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("already open in another process"));
    fixture.run("run", &["--query", "USE GRAPH ready RETURN 1 AS n"]);
}

#[test]
fn piped_repl_preserves_multiline_string_contents_exactly() {
    let fixture = Fixture::new();
    let output = fixture.repl("CREATE GRAPH g ANY GRAPH;\nSESSION SET GRAPH g;\nINSERT (:N {text: 'first\nsecond;still string'});\n\\quit\n");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result = fixture.run(
        "run",
        &[
            "--query",
            "USE GRAPH g MATCH (n) RETURN n.text = 'first\nsecond;still string' AS exact",
        ],
    );
    assert!(result.contains("| true  |"), "{result}");
}

#[test]
fn repl_recovers_from_errors_and_requires_rollback_after_transaction_failure() {
    let fixture = Fixture::new();
    let output = fixture.repl("START TRANSACTION;\nCREATE GRAPH uncommitted ANY GRAPH;\nRETURN missing;\n\\status\nCOMMIT;\nROLLBACK;\nSTART TRANSACTION;\nCREATE GRAPH lexical_error ANY GRAPH;\nRETURN @;\nCOMMIT;\nROLLBACK;\nCREATE GRAPH committed ANY GRAPH;\nRETURN 42 AS answer;\n\\quit\n");
    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stdout.contains("transaction=Failed"), "{stdout}");
    assert!(stdout.contains("42"), "{stdout}");
    assert!(stderr.contains("ROLLBACK is required"), "{stderr}");
    fixture.run("run", &["--query", "USE GRAPH committed RETURN 1 AS n"]);
    assert!(!fixture
        .invoke("run", &["--query", "USE GRAPH lexical_error RETURN 1 AS n"])
        .status
        .success());
    assert!(!fixture
        .invoke("run", &["--query", "USE GRAPH uncommitted RETURN 1 AS n"])
        .status
        .success());
}

#[test]
fn repl_exit_and_eof_rollback_and_discard_unterminated_input() {
    for ending in ["\\q\n", ""] {
        let fixture = Fixture::new();
        let output = fixture.repl(&format!(
            "START TRANSACTION; CREATE GRAPH uncommitted ANY GRAPH;\n{ending}"
        ));
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("transaction rolled back"));
        assert!(!fixture
            .invoke("run", &["--query", "USE GRAPH uncommitted RETURN 1 AS n"])
            .status
            .success());
    }
    let fixture = Fixture::new();
    let output = fixture.repl("CREATE GRAPH unfinished ANY GRAPH");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unfinished input discarded"));
    assert!(!fixture
        .invoke("run", &["--query", "USE GRAPH unfinished RETURN 1 AS n"])
        .status
        .success());
}

#[test]
fn repl_reads_files_clears_input_and_handles_multiline_strings() {
    let fixture = Fixture::new();
    fs::write(
        fixture.0.join("my setup.gql"),
        "CREATE GRAPH g ANY GRAPH; SESSION SET GRAPH g; INSERT (:N {name: 'from file'});",
    )
    .unwrap();
    let output = fixture.repl("\\read my setup.gql\nMATCH (n) RETURN n.name AS name;\nRETURN 'unfinished\n\\clear\n/* incomplete;\ncomment; */ RETURN 'multi\nline;value' AS text;\nRETURN 7 AS n;\n\\explain on\nRETURN 42 AS answer;\n\\q\n");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    for expected in [
        "from file",
        "line;value",
        "7",
        "42",
        "Logical plan:",
        "Physical plan:",
    ] {
        assert!(stdout.contains(expected), "{stdout}");
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn cli_recursive_paths_use_parquet_after_independent_process_reopen() {
    let fixture = Fixture::new();
    fixture.run(
        "run",
        &["--create", "--query", include_str!("fixtures/paths.gql")],
    );
    fixture.run("checkpoint", &[]);
    let query = "USE GRAPH routes MATCH p = ALL SHORTEST (a {name:'A'})-[:Link]->{1,4}(b {name:'D'}) RETURN COUNT(*) AS ties, MIN(PATH_LENGTH(p)) AS hops";
    let output = fixture.run("run", &["--query", query, "--explain"]);
    assert!(output.contains("RecursiveQueryExec"), "{output}");
    assert!(output.contains("WorkTableExec"), "{output}");
    assert!(output.contains("file_type=parquet"), "{output}");
    let compact: String = output.chars().filter(|c| !c.is_whitespace()).collect();
    assert!(compact.contains("|2|2|"), "{output}");
    let output = fixture.run(
        "run",
        &[
            "--query",
            "USE GRAPH routes MATCH p = (a {name:'A'})-[:Link]->{0,2}(b) RETURN COUNT(*) AS paths",
        ],
    );
    let compact: String = output.chars().filter(|c| !c.is_whitespace()).collect();
    assert!(compact.contains("|5|"), "{output}");
}

#[test]
fn cli_reference_lookups_and_mutations_survive_parquet_restart() {
    let fixture = Fixture::new();
    fixture.run(
        "run",
        &[
            "--create",
            "--query",
            include_str!("fixtures/element-references.gql"),
        ],
    );
    fixture.run("checkpoint", &[]);
    let query="USE GRAPH routes MATCH p=(a {name:'A'})-[:Link]->{2}(c) FOR item IN ELEMENTS(p) WITH ORDINALITY i RETURN i,item.name AS name,item IS LABELED Station AS station ORDER BY i";
    let output = fixture.run("run", &["--query", query, "--explain"]);
    assert!(output.contains("file_type=parquet"), "{output}");
    assert!(output.contains("HashJoinExec"), "{output}");
    let compact: String = output.chars().filter(|c| !c.is_whitespace()).collect();
    for row in [
        "|1|A|true|",
        "|2|AB|false|",
        "|3|B|true|",
        "|4|BC|false|",
        "|5|C|true|",
    ] {
        assert!(compact.contains(row), "{output}");
    }
    let output=fixture.run("run", &["--query","USE GRAPH routes MATCH (n) LET r=n RETURN r.name AS name,r.visits AS visits ORDER BY name"]);
    let compact: String = output.chars().filter(|c| !c.is_whitespace()).collect();
    for row in ["|A|0|", "|B|1|", "|C|1|"] {
        assert!(compact.contains(row), "{output}");
    }
}

#[test]
fn cli_complex_patterns_and_unbounded_shortest_survive_parquet_restart() {
    let fixture = Fixture::new();
    fixture.run(
        "run",
        &[
            "--create",
            "--query",
            include_str!("fixtures/path-patterns.gql"),
        ],
    );
    fixture.run("checkpoint", &[]);
    let query = "USE GRAPH routes MATCH p = (a {name:'A'})((x)-[edges:Link]->(y)){0,2}(b) RETURN COUNT(*) AS paths, MAX(PATH_LENGTH(p)) AS longest";
    let output = fixture.run("run", &["--query", query, "--explain"]);
    assert!(output.contains("file_type=parquet"), "{output}");
    assert!(output.contains("RecursiveQueryExec"), "{output}");
    let compact: String = output.chars().filter(|c| !c.is_whitespace()).collect();
    assert!(compact.contains("|5|2|"), "{output}");
    let query = "USE GRAPH routes MATCH REPEATABLE ELEMENTS p = ALL SHORTEST (a {name:'A'})-[:Link]->+(b {name:'D'}) RETURN COUNT(*) AS ties, MAX(PATH_LENGTH(p)) AS hops";
    let output = fixture.run("run", &["--query", query]);
    let compact: String = output.chars().filter(|c| !c.is_whitespace()).collect();
    assert!(compact.contains("|2|2|"), "{output}");
    let query = "USE GRAPH routes MATCH p = (a {name:'A'})(-[e:Link]->(b))? RETURN COUNT(*) AS paths, COUNT(e) AS present";
    let output = fixture.run("run", &["--query", query]);
    let compact: String = output.chars().filter(|c| !c.is_whitespace()).collect();
    assert!(compact.contains("|3|2|"), "{output}");
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
    assert!(output.contains("partition_sizes="), "{output}");
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
                "START TRANSACTION; CREATE GRAPH rejected ANY GRAPH"
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
fn run_prints_committed_statements_before_a_later_error() {
    let fixture = Fixture::new();
    let output = fixture.invoke(
        "run",
        &[
            "--create",
            "--query",
            "CREATE GRAPH g ANY GRAPH; SESSION SET GRAPH g; INSERT (:N {i: 1}); RETURN 1 / 0 AS x",
        ],
    );
    assert!(
        !output.status.success(),
        "expected the last statement to fail"
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("affected_elements=1"),
        "committed insert was not printed:\n{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let again = fixture.run(
        "run",
        &["--query", "SESSION SET GRAPH g; MATCH (n) RETURN n.i AS i"],
    );
    assert!(again.contains('1'), "{again}");
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

#[test]
fn cli_executes_persistent_gql_mutations_without_an_external_import() {
    let fixture = Fixture::new();
    fs::write(fixture.0.join("write.gql"), "CREATE GRAPH g ANY GRAPH; SESSION SET GRAPH g; INSERT (a:Person {name: 'Alice', age: 30})-[:Knows]->(b:Person {name: 'Bob', age: 40}); MATCH (a {name: 'Alice'}) SET a.age = a.age + 1;").unwrap();
    assert!(fixture
        .run("run", &["--create", "--file", "write.gql"])
        .contains("affected_elements=3"));
    let result = fixture.run(
        "run",
        &[
            "--query",
            "USE GRAPH g MATCH (a {name: 'Alice'}) RETURN a.age AS age",
            "--explain",
        ],
    );
    assert!(result.contains("31"));
    assert!(result.contains("partition_sizes="));
    let result = fixture.invoke(
        "run",
        &["--query", "USE GRAPH g MATCH (a {name: 'Alice'}) DELETE a"],
    );
    assert!(!result.status.success());
    fixture.run(
        "run",
        &[
            "--query",
            "USE GRAPH g MATCH (a {name: 'Alice'}) DETACH DELETE a",
        ],
    );
    fixture.run("checkpoint", &[]);
    let result = fixture.run(
        "run",
        &["--query", "USE GRAPH g MATCH (n) RETURN n.name AS name"],
    );
    assert!(result.contains("Bob"));
    assert!(!result.contains("Alice"));
}

#[test]
fn cli_relational_queries_and_rollback_survive_parquet_reopen() {
    let fixture = Fixture::new();
    fs::write(
        fixture.0.join("analytics.gql"),
        include_str!("fixtures/analytics.gql"),
    )
    .unwrap();
    let result = fixture.run("run", &["--create", "--file", "analytics.gql"]);
    assert!(result.contains("median_age"));
    assert!(result.contains("| Cara  | 0"), "{result}");
    fixture.run("checkpoint", &[]);
    let result = fixture.run("run", &["--query", "USE GRAPH analytics MATCH (p:Person) OPTIONAL MATCH (p)-[:Knows]->(f) RETURN p.name AS name, COUNT(f) AS friends GROUP BY name ORDER BY name", "--explain"]);
    assert!(result.contains("| Alice | 1"), "{result}");
    assert!(result.contains("| Bob   | 0"), "{result}");
    assert!(result.contains("| Cara  | 0"), "{result}");
    assert!(result.contains("file_type=parquet"), "{result}");
    assert!(!fixture.invoke("run", &["--query", "USE GRAPH analytics INSERT (:Person {name: 'Rolled back'}) FOR x IN [9223372036854775807, 1] RETURN SUM(x) AS n"]).status.success());
    let result = fixture.run(
        "run",
        &[
            "--query",
            "USE GRAPH analytics MATCH (p:Person) RETURN COUNT(*) AS n",
        ],
    );
    assert!(result.contains("| 3 |"), "{result}");
}

#[test]
fn cli_explicit_transactions_publish_once_and_survive_reopen() {
    let fixture = Fixture::new();
    fs::write(
        fixture.0.join("transactions.gql"),
        include_str!("fixtures/transactions.gql"),
    )
    .unwrap();
    let result = fixture.run("run", &["--create", "--file", "transactions.gql"]);
    assert!(
        result.contains("transaction=committed commit=2"),
        "{result}"
    );
    assert!(result.contains("pending_snapshot=1"), "{result}");
    assert!(result.contains("transaction=rolled_back"), "{result}");
    assert!(result.contains("| Alice | 75"), "{result}");
    assert!(result.contains("| Bob   | 75"), "{result}");
    fixture.run("checkpoint", &[]);
    let result = fixture.run("run", &["--query", "START TRANSACTION READ ONLY; USE GRAPH ledger MATCH (a:Account) RETURN SUM(a.balance) AS total; COMMIT", "--explain"]);
    assert!(result.contains("150"), "{result}");
    assert!(result.contains("file_type=parquet"), "{result}");
    for sql in [
        "START TRANSACTION; USE GRAPH ledger INSERT (:Account {name: 'Failed', balance: 5}); RETURN 1 / 0 AS bad; COMMIT",
        "START TRANSACTION; USE GRAPH ledger INSERT (:Account {name: 'Unfinished', balance: 6})",
        "START TRANSACTION READ ONLY; DROP GRAPH ledger; COMMIT",
    ] { assert!(!fixture.invoke("run", &["--query", sql]).status.success(), "{sql}"); }
    let result = fixture.run("run", &["--query", "USE GRAPH ledger MATCH (a:Account) RETURN COUNT(*) AS accounts, SUM(a.balance) AS total"]);
    assert!(result.contains("| 2        | 150"), "{result}");
}
