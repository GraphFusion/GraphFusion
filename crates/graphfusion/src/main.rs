use graphfusion::{
    arrow::{record_batch::RecordBatch, util::pretty::pretty_format_batches},
    gql,
    graph::{EdgeTable, GraphData, NodeTable},
    Database, OpenOptions, StatementOutput,
};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

const HELP: &str = "GraphFusion — GQL on DataFusion
Usage:
  graphfusion run [--database DIR] [--create] (--file FILE | --query GQL) [--explain]
  graphfusion import --database DIR --graph EXPR --manifest FILE
  graphfusion checkpoint --database DIR

run uses an in-memory database if --database is omitted.
--create permits creating a database directory; otherwise it must already exist.
import replaces an existing open graph with validated external Parquet tables.
Statements auto-commit; earlier successful statements survive a later error.
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

#[tokio::main]
async fn main() -> std::process::ExitCode {
    match run().await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("graphfusion: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

async fn run() -> CliResult<()> {
    let mut args = std::env::args().skip(1);
    let Some(command) = args.next() else {
        print!("{HELP}");
        return Ok(());
    };
    if command == "--help" || command == "help" {
        print!("{HELP}");
        return Ok(());
    }
    let allowed: &[&str] = match command.as_str() {
        "run" => &["--database", "--create", "--file", "--query", "--explain"],
        "import" => &["--database", "--graph", "--manifest"],
        "checkpoint" => &["--database"],
        _ => return Err(format!("unknown command {command}; use --help").into()),
    };
    let mut options = BTreeMap::new();
    while let Some(name) = args.next() {
        if name == "--help" {
            print!("{HELP}");
            return Ok(());
        }
        if !allowed.contains(&name.as_str()) {
            return Err(format!("unknown option {name}").into());
        }
        let value = if name == "--create" || name == "--explain" {
            String::new()
        } else {
            args.next()
                .ok_or_else(|| format!("missing value for {name}"))?
        };
        if options.insert(name.clone(), value).is_some() {
            return Err(format!("duplicate option {name}").into());
        }
    }
    if command == "run" {
        let input = match (options.get("--file"), options.get("--query")) {
            (Some(path), None) => fs::read_to_string(path)?,
            (None, Some(query)) => query.clone(),
            _ => return Err("run requires exactly one of --file and --query".into()),
        };
        if options.contains_key("--create") && !options.contains_key("--database") {
            return Err("--create requires --database".into());
        }
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
        for output in db.session().run(&input).await? {
            match output {
                StatementOutput::Command(result) => println!(
                    "OK commit={} affected_objects={}",
                    result.commit_seq, result.affected_objects
                ),
                StatementOutput::Query(result) => {
                    let batches = if result.batches.is_empty() {
                        vec![RecordBatch::new_empty(result.schema)]
                    } else {
                        result.batches
                    };
                    println!("{}", pretty_format_batches(&batches)?);
                    if options.contains_key("--explain") {
                        println!(
                            "Logical plan:\n{}\nPhysical plan:\n{}",
                            result.logical_plan, result.physical_plan
                        );
                    }
                }
            }
        }
    } else if command == "import" {
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
