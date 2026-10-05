#![cfg(feature = "visualization")]
use graphfusion::{Database, OpenOptions, QueryResult, Session, StatementOutput};

async fn run(session: &mut Session, input: &str) -> QueryResult {
    match session.run(input).await.unwrap().pop().unwrap() {
        StatementOutput::Query(result) => result,
        _ => panic!("expected query"),
    }
}
fn setup(session: &mut Session) {
    session
        .execute("CREATE GRAPH g ANY; SESSION SET GRAPH g")
        .unwrap();
}

#[tokio::test]
async fn projects_returned_paths_and_edge_endpoints_from_the_result_snapshot() {
    let mut session = Database::new().session();
    setup(&mut session);
    run(&mut session, "INSERT (a:Person {name:'A'})-[e:Link {weight:3}]->(b:Person {name:'B'}), (b)-[:Link]->(c:Person {name:'C'})").await;
    let edge = run(&mut session, "MATCH (a {name:'A'})-[e]->(b) RETURN e").await;
    assert_eq!(edge.graph.nodes.len(), 2);
    assert_eq!(edge.graph.edges.len(), 1);
    assert_eq!(edge.graph.edges[0].properties["weight"], 3);
    assert!(edge
        .graph
        .nodes
        .iter()
        .any(|n| n.id == edge.graph.edges[0].source));
    let old = run(
        &mut session,
        "MATCH p = (a {name:'A'})-[:Link]->{2}(b) RETURN p",
    )
    .await;
    assert_eq!((old.graph.nodes.len(), old.graph.edges.len()), (3, 2));
    let updated = run(
        &mut session,
        "MATCH (a {name:'A'}) SET a.name='New' RETURN a",
    )
    .await;
    assert_eq!(updated.graph.nodes[0].properties["name"], "New");
    assert!(old.graph.nodes.iter().any(|n| n.properties["name"] == "A"));
    let deleted = run(&mut session, "MATCH (a {name:'New'}) DETACH DELETE a").await;
    assert!(deleted.graph.nodes.is_empty());
    let remaining = run(&mut session, "MATCH (a) RETURN a").await;
    assert_eq!(remaining.graph.nodes.len(), 2);
    assert!(!remaining
        .graph
        .nodes
        .iter()
        .any(|n| n.properties["name"] == "New"));
    let scalar = run(&mut session, "MATCH (a) RETURN a.name AS name").await;
    assert!(scalar.graph.nodes.is_empty());
}

#[tokio::test]
async fn same_element_ids_in_different_graphs_remain_distinct() {
    let mut session = Database::new().session();
    setup(&mut session);
    run(&mut session, "INSERT (:Person {name:'A'})").await;
    session
        .execute("CREATE GRAPH h ANY; SESSION SET GRAPH h")
        .unwrap();
    run(&mut session, "INSERT (:Person {name:'B'})").await;
    let result = run(
        &mut session,
        "USE g MATCH (n) RETURN n UNION ALL USE h MATCH (n) RETURN n",
    )
    .await;
    assert_eq!(result.graph.nodes.len(), 2);
    assert_ne!(result.graph.nodes[0].id, result.graph.nodes[1].id);
}

#[tokio::test]
async fn parquet_projection_uses_visible_selected_rows_and_exact_integer_ids() {
    use graphfusion::arrow::{
        array::{Int64Array, UInt64Array},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    };
    use graphfusion::{
        graph::{GraphData, NodeTable, ID},
        visualization::cell,
    };
    use std::sync::Arc;
    let directory =
        std::env::temp_dir().join(format!("graphfusion-visualization-{}", std::process::id()));
    let db = Database::open(
        &directory,
        OpenOptions {
            create_if_missing: true,
        },
    )
    .unwrap();
    let mut session = db.session();
    setup(&mut session);
    let schema = Arc::new(Schema::new(vec![
        Field::new(ID, DataType::UInt64, false),
        Field::new("value", DataType::Int64, false),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(UInt64Array::from(vec![u64::MAX - 1, u64::MAX])),
            Arc::new(Int64Array::from(vec![i64::MAX, i64::MIN])),
        ],
    )
    .unwrap();
    session
        .replace_graph_data(
            GraphData::try_new(
                vec![NodeTable::try_new(vec!["Large".into()], schema, vec![batch]).unwrap()],
                vec![],
            )
            .unwrap(),
        )
        .unwrap();
    db.checkpoint().unwrap();
    let result = run(&mut session, "MATCH (n) RETURN n").await;
    assert_eq!(result.graph.nodes.len(), 2);
    assert!(result
        .graph
        .nodes
        .iter()
        .any(|n| n.id.ends_with(&u64::MAX.to_string())));
    let maximum = i64::MAX.to_string();
    assert!(result
        .graph
        .nodes
        .iter()
        .any(|n| n.properties["value"].as_str() == Some(maximum.as_str())));
    let array: graphfusion::arrow::array::ArrayRef = Arc::new(Int64Array::from(vec![42, i64::MIN]));
    assert_eq!(cell(&array, 0).unwrap(), 42);
    assert_eq!(cell(&array, 1).unwrap(), i64::MIN.to_string());
    drop(session);
    drop(db);
    std::fs::remove_dir_all(directory).unwrap();
}
