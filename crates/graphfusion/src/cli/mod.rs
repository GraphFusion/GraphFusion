mod repl;

use graphfusion::{
    arrow::{record_batch::RecordBatch, util::pretty::pretty_format_batches},
    gql,
    graph::{EdgeTable, GraphData, NodeTable},
    Database, OpenOptions, StatementOutput, TransactionAction, TransactionStatus,
};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

const HELP: &str = "GraphFusion — GQL on DataFusion
Usage:
  graphfusion [DIR] [--explain]
  graphfusion --database DIR [--explain]
  graphfusion run [--database DIR] [--create] (--file FILE | --query GQL) [--explain]
  graphfusion import --database DIR --graph EXPR --manifest FILE
  graphfusion checkpoint --database DIR

Interactive mode opens or creates DIR; omitting it or using :memory: uses memory.
End GQL with a semicolon. Use -- DIR for paths beginning with '-' or command names.
run uses memory without --database; --create permits creating its database directory.
import replaces an existing open graph with validated external Parquet tables.
Statements auto-commit unless enclosed in START TRANSACTION and COMMIT/ROLLBACK.
With run, an explicit transaction must end before the file/query finishes.
";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ImportManifest {
    nodes: Vec<NodeInput>,
    edges: Vec<EdgeInput>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NodeInput {
    labels: Vec<String>,
    file: PathBuf,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EdgeInput {
    labels: Vec<String>,
    file: PathBuf,
    directed: bool,
}
type CliResult<T> = Result<T, Box<dyn std::error::Error>>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Command {
    Interactive,
    Run,
    Import,
    Checkpoint,
}

pub(super) async fn run() -> CliResult<()> {
    let mut args = std::env::args().skip(1).peekable();
    if args.peek().is_some_and(|arg| arg == "help") {
        print!("{HELP}");
        return Ok(());
    }
    let command = match args.peek().map(String::as_str) {
        Some("run") => Command::Run,
        Some("import") => Command::Import,
        Some("checkpoint") => Command::Checkpoint,
        _ => Command::Interactive,
    };
    if command != Command::Interactive {
        args.next();
    }
    let allowed: &[&str] = match command {
        Command::Interactive => &["--database", "--explain"],
        Command::Run => &["--database", "--create", "--file", "--query", "--explain"],
        Command::Import => &["--database", "--graph", "--manifest"],
        Command::Checkpoint => &["--database"],
    };
    let mut options = BTreeMap::new();
    let mut positional_only = false;
    while let Some(name) = args.next() {
        if command == Command::Interactive && (positional_only || !name.starts_with('-')) {
            if options.insert("--database".into(), name).is_some() {
                return Err("specify only one database path: DIR or --database DIR".into());
            }
            continue;
        }
        if name == "--help" || name == "-h" {
            print!("{HELP}");
            return Ok(());
        }
        if command == Command::Interactive && name == "--" {
            positional_only = true;
            continue;
        }
        if !allowed.contains(&name.as_str()) {
            return Err(format!("unknown option {name}").into());
        }
        let value = if name == "--create" || name == "--explain" {
            String::new()
        } else {
            let value = args
                .next()
                .ok_or_else(|| format!("missing value for {name}"))?;
            // Do not mistake an option for the interactive database path.
            // Paths starting with '-' can use './' or positional '-- DIR'.
            if command == Command::Interactive && value.starts_with('-') {
                return Err(format!("missing value for {name}").into());
            }
            value
        };
        if options.insert(name.clone(), value).is_some() {
            return Err(format!("duplicate option {name}").into());
        }
    }
    if options.contains_key("--create") && !options.contains_key("--database") {
        return Err("--create requires --database".into());
    }
    if options.get("--database").is_some_and(String::is_empty) {
        return Err("database path must not be empty".into());
    }
    if command == Command::Interactive {
        let path = options
            .get("--database")
            .map(String::as_str)
            .filter(|path| *path != ":memory:");
        let database = match path {
            Some(path) => Database::open(
                path,
                OpenOptions {
                    create_if_missing: true,
                },
            )?,
            None => Database::new(),
        };
        repl::run(database, path, options.contains_key("--explain")).await?;
    } else if command == Command::Run {
        let input = match (options.get("--file"), options.get("--query")) {
            (Some(path), None) => fs::read_to_string(path)?,
            (None, Some(query)) => query.clone(),
            _ => return Err("run requires exactly one of --file and --query".into()),
        };
        // Parse before creating a database, and preserve quoted semicolons/comments via the AST.
        gql::parse(&input)?;
        let db = match options.get("--database") {
            Some(path) => Database::open(
                path,
                OpenOptions {
                    create_if_missing: options.contains_key("--create"),
                },
            )?,
            None => Database::new(),
        };
        let mut session = db.session();
        let outputs = session.run(&input).await?;
        if session.transaction_status() != TransactionStatus::Idle {
            return Err("unfinished explicit transaction rolled back; end the file/query with COMMIT or ROLLBACK".into());
        }
        print_outputs(outputs, options.contains_key("--explain"))?;
    } else if command == Command::Import {
        let database = required(&options, "--database")?;
        let graph = required(&options, "--graph")?;
        let path = Path::new(required(&options, "--manifest")?);
        let context = format!("SESSION SET GRAPH {graph}");
        let parsed = gql::parse(&context)?;
        if !matches!(
            parsed.statements.as_slice(),
            [gql::Statement::SessionSet(gql::SessionSetCommand {
                target: gql::SessionSetTarget::Graph(_)
            })]
        ) || parsed.at_schema.is_some()
            || !parsed.definitions.is_empty()
        {
            return Err("--graph requires one graph expression".into());
        }
        let manifest: ImportManifest = serde_json::from_slice(&fs::read(path)?)?;
        let parent = path.parent().unwrap_or(Path::new("."));
        let nodes = manifest
            .nodes
            .into_iter()
            .map(|node| NodeTable::read_parquet(node.labels, parent.join(node.file)))
            .collect::<graphfusion::Result<_>>()?;
        let edges = manifest
            .edges
            .into_iter()
            .map(|edge| EdgeTable::read_parquet(edge.labels, edge.directed, parent.join(edge.file)))
            .collect::<graphfusion::Result<_>>()?;
        let data = GraphData::try_new(nodes, edges)?;
        let (nodes, edges) = (data.node_count(), data.edge_count());
        let db = Database::open(database, OpenOptions::default())?;
        let mut session = db.session();
        session.execute(&context)?;
        let seq = session.replace_graph_data(data)?;
        println!("OK commit={seq} nodes={nodes} edges={edges}");
    } else {
        Database::open(required(&options, "--database")?, OpenOptions::default())?.checkpoint()?;
        println!("OK checkpoint");
    }
    Ok(())
}

fn required<'a>(options: &'a BTreeMap<String, String>, key: &str) -> CliResult<&'a str> {
    options
        .get(key)
        .map(String::as_str)
        .ok_or_else(|| format!("missing required option {key}").into())
}

fn print_outputs(outputs: Vec<StatementOutput>, explain: bool) -> CliResult<()> {
    for output in outputs {
        match output {
            StatementOutput::Command(result) => match result.transaction_action {
                Some(action) => println!(
                    "OK transaction={} {}={}",
                    match action {
                        TransactionAction::Started => "started",
                        TransactionAction::Committed => "committed",
                        TransactionAction::RolledBack => "rolled_back",
                    },
                    if action == TransactionAction::Committed {
                        "commit"
                    } else {
                        "snapshot"
                    },
                    result.commit_seq
                ),
                None => println!(
                    "OK {}={} affected_objects={}",
                    if result.transaction_pending {
                        "pending_snapshot"
                    } else {
                        "commit"
                    },
                    result.commit_seq,
                    result.affected_objects
                ),
            },
            StatementOutput::Query(result) => {
                if result.schema.fields().is_empty() || result.transaction_pending {
                    println!(
                        "OK {}={} affected_elements={}",
                        if result.transaction_pending {
                            "pending_snapshot"
                        } else {
                            "commit"
                        },
                        result.commit_seq,
                        result.affected_elements,
                    );
                }
                let batches = if result.batches.is_empty() {
                    vec![RecordBatch::new_empty(result.schema)]
                } else {
                    result.batches
                };
                if !batches[0].schema().fields().is_empty() {
                    println!("{}", pretty_format_batches(&batches)?);
                }
                if explain {
                    println!(
                        "Logical plan:\n{}\nPhysical plan:\n{}",
                        result.logical_plan, result.physical_plan
                    );
                }
            }
        }
    }
    Ok(())
}
