use graphfusion::{
    arrow::{
        array::{StringArray, UInt64Array},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
        util::pretty::pretty_format_batches,
    },
    graph::{EdgeTable, GraphData, NodeTable, DESTINATION, ID, SOURCE},
    Database, OpenOptions,
};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let node_schema = Arc::new(Schema::new(vec![
        Field::new(ID, DataType::UInt64, false),
        Field::new("name", DataType::Utf8, false),
    ]));
    let nodes = RecordBatch::try_new(
        node_schema.clone(),
        vec![
            Arc::new(UInt64Array::from(vec![1, 2, 3])),
            Arc::new(StringArray::from(vec!["Alice", "Bob", "Cara"])),
        ],
    )?;
    let edge_schema = Arc::new(Schema::new(vec![
        Field::new(ID, DataType::UInt64, false),
        Field::new(SOURCE, DataType::UInt64, false),
        Field::new(DESTINATION, DataType::UInt64, false),
    ]));
    let edges = RecordBatch::try_new(
        edge_schema.clone(),
        vec![
            Arc::new(UInt64Array::from(vec![10, 11])),
            Arc::new(UInt64Array::from(vec![1, 2])),
            Arc::new(UInt64Array::from(vec![2, 3])),
        ],
    )?;
    let graph = GraphData::try_new(
        vec![NodeTable::try_new(
            vec!["Person".into()],
            node_schema,
            vec![nodes],
        )?],
        vec![EdgeTable::try_new(
            vec!["Knows".into()],
            true,
            edge_schema,
            vec![edges],
        )?],
    )?;
    let db = match std::env::args_os().nth(1) {
        Some(path) => Database::open(
            path,
            OpenOptions {
                create_if_missing: true,
            },
        )?,
        None => Database::new(),
    };
    let mut session = db.session();
    session.execute("CREATE GRAPH social ANY GRAPH; SESSION SET GRAPH social")?;
    session.replace_graph_data(graph)?;
    let result = session
        .query(
            "MATCH (a:Person {name: 'Alice'})-[:Knows]->(b)-[:Knows]->(c) RETURN c.name AS friend",
        )
        .await?;
    println!("{}", pretty_format_batches(&result.batches)?);
    println!("DataFusion physical plan:\n{}", result.physical_plan);
    Ok(())
}
