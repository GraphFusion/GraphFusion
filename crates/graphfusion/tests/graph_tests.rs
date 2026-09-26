use datafusion::common::ScalarValue;
use graphfusion::{
    arrow::{
        array::{ArrayRef, Int64Array, StringArray, UInt64Array},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    graph::{EdgeTable, GraphData, NodeTable, DESTINATION, ID, SOURCE},
    Database, Error, QueryResult, Session, Value,
};
use std::sync::Arc;

fn nodes(
    labels: &[&str],
    ids: Vec<u64>,
    names: Vec<&str>,
    ages: Option<Vec<Option<i64>>>,
) -> NodeTable {
    let mut fields = vec![
        Field::new(ID, DataType::UInt64, false),
        Field::new("name", DataType::Utf8, false),
    ];
    let mut columns: Vec<ArrayRef> = vec![
        Arc::new(UInt64Array::from(ids)),
        Arc::new(StringArray::from(names)),
    ];
    if let Some(ages) = ages {
        fields.push(Field::new("age", DataType::Int64, true));
        columns.push(Arc::new(Int64Array::from(ages)));
    }
    let schema = Arc::new(Schema::new(fields));
    let batch = RecordBatch::try_new(schema.clone(), columns).unwrap();
    NodeTable::try_new(
        labels.iter().map(|s| (*s).into()).collect(),
        schema,
        vec![batch],
    )
    .unwrap()
}
fn edges(directed: bool, label: &str, rows: &[(u64, u64, u64, i64)]) -> EdgeTable {
    let schema = Arc::new(Schema::new(vec![
        Field::new(ID, DataType::UInt64, false),
        Field::new(SOURCE, DataType::UInt64, false),
        Field::new(DESTINATION, DataType::UInt64, false),
        Field::new("since", DataType::Int64, false),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(UInt64Array::from(
                rows.iter().map(|r| r.0).collect::<Vec<_>>(),
            )),
            Arc::new(UInt64Array::from(
                rows.iter().map(|r| r.1).collect::<Vec<_>>(),
            )),
            Arc::new(UInt64Array::from(
                rows.iter().map(|r| r.2).collect::<Vec<_>>(),
            )),
            Arc::new(Int64Array::from(
                rows.iter().map(|r| r.3).collect::<Vec<_>>(),
            )),
        ],
    )
    .unwrap();
    EdgeTable::try_new(vec![label.into()], directed, schema, vec![batch]).unwrap()
}
fn fixture() -> GraphData {
    GraphData::try_new(
        vec![
            nodes(
                &["Person", "Admin"],
                vec![1],
                vec!["Alice"],
                Some(vec![Some(30)]),
            ),
            nodes(
                &["Person"],
                vec![2, 3],
                vec!["Bob", "Cara"],
                Some(vec![Some(40), None]),
            ),
            nodes(&["City"], vec![4], vec!["London"], None),
        ],
        vec![
            edges(
                true,
                "Knows",
                &[
                    (10, 1, 2, 2019),
                    (11, 1, 2, 2020),
                    (12, 2, 3, 2021),
                    (13, 3, 3, 2022),
                ],
            ),
            edges(false, "Friend", &[(20, 1, 3, 2010), (21, 2, 2, 2011)]),
        ],
    )
    .unwrap()
}
fn database() -> (Database, Session) {
    let db = Database::new();
    let mut session = db.session();
    session
        .execute("CREATE GRAPH social ANY GRAPH; SESSION SET GRAPH social")
        .unwrap();
    session.replace_graph_data(fixture()).unwrap();
    (db, session)
}

#[tokio::test]
async fn parquet_matches_arrow_semantics_after_checkpoint_and_reopen() {
    let path = std::env::temp_dir().join(format!(
        "graphfusion-layouts % # [data] * ?-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    {
        let db = Database::open(
            &path,
            graphfusion::OpenOptions {
                create_if_missing: true,
            },
        )
        .unwrap();
        let mut session = db.session();
        session
            .execute("CREATE GRAPH social ANY GRAPH; SESSION SET GRAPH social")
            .unwrap();
        session.replace_graph_data(fixture()).unwrap();
        db.checkpoint().unwrap();
    }
    {
        let db = Database::open(&path, graphfusion::OpenOptions::default()).unwrap();
        let mut durable = db.session();
        durable.execute("SESSION SET GRAPH social").unwrap();
        let (_memory, mut memory) = database();
        for query in [
            "MATCH (n) RETURN n.name AS name, n.age AS age ORDER BY name",
            "MATCH (n:Person&Admin) RETURN n.name AS name",
            "MATCH (a)-[e:Friend]-(b) RETURN a.name AS a, b.name AS b ORDER BY a, b",
            "MATCH (a)-[:Knows]->(b)-[:Knows]->(c) RETURN a.name AS a, c.name AS c ORDER BY a, c",
            "MATCH (a)<-[:Knows]-(b) RETURN a.name AS a, b.name AS b ORDER BY a, b",
            "MATCH (n) WHERE PROPERTY_EXISTS(n, age) RETURN n.name AS name ORDER BY name",
            "MATCH (n) WHERE n.age IS NULL RETURN n.name AS name ORDER BY name",
            "MATCH (n:Missing) RETURN n.name AS name",
        ] {
            assert_eq!(
                strings(&durable.query(query).await.unwrap()),
                strings(&memory.query(query).await.unwrap()),
                "{query}"
            );
        }
        let empty_table = GraphData::try_new(
            vec![nodes(&["Person"], vec![], vec![], Some(vec![]))],
            vec![],
        )
        .unwrap();
        durable.replace_graph_data(empty_table).unwrap();
        let result = durable
            .query("MATCH (n:Person) RETURN n.age AS age")
            .await
            .unwrap();
        assert_eq!(result.row_count(), 0);
        assert_eq!(result.schema.field(0).data_type(), &DataType::Int64);
        db.checkpoint().unwrap();
    }
    std::fs::remove_dir_all(path).unwrap();
}
fn strings(result: &QueryResult) -> Vec<Vec<String>> {
    result
        .batches
        .iter()
        .flat_map(|batch| {
            (0..batch.num_rows())
                .map(|row| {
                    batch
                        .columns()
                        .iter()
                        .map(|array| ScalarValue::try_from_array(array, row).unwrap().to_string())
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

#[tokio::test]
async fn node_scan_labels_properties_and_real_datafusion_provider() {
    let (_, mut session) = database();
    let result = session
        .query("MATCH (n:Person) RETURN n.name AS name ORDER BY name")
        .await
        .unwrap();
    assert_eq!(
        strings(&result),
        vec![vec!["Alice"], vec!["Bob"], vec!["Cara"]]
    );
    assert!(result.logical_plan.contains("TableScan"));
    assert!(
        result.physical_plan.contains("DataSourceExec"),
        "{}",
        result.physical_plan
    );
    let result = session
        .query("MATCH (n:Person&Admin {age: 30}) RETURN n.name AS name")
        .await
        .unwrap();
    assert_eq!(strings(&result), vec![vec!["Alice"]]);
    let result = session
        .query("MATCH (n:Person|Admin) RETURN n.name AS name ORDER BY name")
        .await
        .unwrap();
    assert_eq!(result.row_count(), 3); // multi-label Alice is one element
    session
        .set_parameter("minimum", Value::Integer(35))
        .unwrap();
    let result = session
        .query("MATCH (n:Person WHERE n.age > $minimum) RETURN n.name AS name")
        .await
        .unwrap();
    assert_eq!(strings(&result), vec![vec!["Bob"]]);
    assert!(matches!(
        session
            .query("MATCH (n {age: '40'}) RETURN n.name AS value")
            .await,
        Err(Error::InvalidQuery(_))
    ));
}

#[tokio::test]
async fn inline_predicates_resolve_all_variables_in_the_match() {
    let (_, mut session) = database();
    for query in [
        "MATCH (a WHERE a.age < b.age)-[:Knows]->(b) RETURN a.name AS a, b.name AS b",
        "MATCH (b)<-[:Knows]-(a WHERE a.age < b.age) RETURN a.name AS a, b.name AS b",
        "MATCH (a)-[e:Knows WHERE e.since > b.age]->(b) RETURN a.name AS a, b.name AS b",
        "MATCH (a WHERE a.age < e.since)-[e:Knows]->(b WHERE b.age > a.age) RETURN a.name AS a, b.name AS b",
    ] {
        let result = session.query(query).await.unwrap();
        assert_eq!(strings(&result), vec![vec!["Alice", "Bob"], vec!["Alice", "Bob"]], "{query}");
    }
    let result = session
        .query("MATCH (a WHERE a.name = b.name), (b:City) RETURN a.name AS name")
        .await
        .unwrap();
    assert_eq!(strings(&result), vec![vec!["London"]]);
    for query in [
        "MATCH (a WHERE a.name = missing.name) RETURN a.name AS name",
        "MATCH (a WHERE a.name = b.name) MATCH (b) RETURN a.name AS name",
    ] {
        assert!(
            matches!(session.query(query).await, Err(Error::InvalidQuery(_))),
            "{query}"
        );
    }
}

#[tokio::test]
async fn property_patterns_resolve_forward_bindings() {
    let (_, mut session) = database();
    let result = session
        .query("MATCH (a {name: b.name}), (b:City) RETURN a.name AS name")
        .await
        .unwrap();
    assert_eq!(strings(&result), vec![vec!["London"]]);
    let result = session
        .query("MATCH (a)-[e:Knows {since: b.age + 1979}]->(b) RETURN a.name AS a, b.name AS b, e.since AS since")
        .await
        .unwrap();
    assert_eq!(strings(&result), vec![vec!["Alice", "Bob", "2019"]]);
}

#[tokio::test]
async fn directed_joins_keep_parallel_edges_and_match_incoming_paths() {
    let (_, mut session) = database();
    for query in [
        "MATCH (a)-[e:Knows]->(b) RETURN a.name AS a, b.name AS b, e.since AS since ORDER BY since",
        "MATCH (b)<-[e:Knows]-(a) RETURN a.name AS a, b.name AS b, e.since AS since ORDER BY since",
    ] {
        let result = session.query(query).await.unwrap();
        assert_eq!(
            strings(&result),
            vec![
                vec!["Alice", "Bob", "2019"],
                vec!["Alice", "Bob", "2020"],
                vec!["Bob", "Cara", "2021"],
                vec!["Cara", "Cara", "2022"]
            ]
        );
        assert!(
            result.physical_plan.contains("JoinExec"),
            "{}",
            result.physical_plan
        );
    }
    let result = session
        .query("MATCH (a {name: 'Alice'})-[:Knows]->(b)-[:Knows]->(c) RETURN c.name AS name")
        .await
        .unwrap();
    assert_eq!(strings(&result), vec![vec!["Cara"], vec!["Cara"]]);
    assert_eq!(
        session
            .query("MATCH (a)-[:Friend]->(b) RETURN a.name AS value")
            .await
            .unwrap()
            .row_count(),
        0
    );
}

#[tokio::test]
async fn undirected_orientations_do_not_duplicate_self_loops() {
    let (_, mut session) = database();
    let result = session
        .query("MATCH (a)~[e:Friend]~(b) RETURN a.name AS a, b.name AS b ORDER BY a, b")
        .await
        .unwrap();
    assert_eq!(
        strings(&result),
        vec![
            vec!["Alice", "Cara"],
            vec!["Bob", "Bob"],
            vec!["Cara", "Alice"]
        ]
    );
    let result = session
        .query("MATCH (a)-[e:Knows]-(b) RETURN a.name AS a, b.name AS b")
        .await
        .unwrap();
    assert_eq!(result.row_count(), 7); // 3 nonloops twice, 1 loop once
    assert_eq!(
        session
            .query("MATCH (a)~[:Knows]~(b) RETURN a.name AS value")
            .await
            .unwrap()
            .row_count(),
        0
    );
}

#[tokio::test]
async fn repeated_bindings_and_match_modes_preserve_identity() {
    let (_, mut session) = database();
    assert_eq!(
        session
            .query("MATCH (a:Person), (a) RETURN a.name AS value")
            .await
            .unwrap()
            .row_count(),
        3
    );
    let result = session
        .query("MATCH (a)-[:Knows]->(a) RETURN a.name AS name")
        .await
        .unwrap();
    assert_eq!(strings(&result), vec![vec!["Cara"]]);
    let path = "(a {name: 'Cara'})-[r:Knows]->(a)-[s:Knows]->(a) RETURN a.name AS name";
    assert_eq!(
        session
            .query(&format!("MATCH DIFFERENT EDGES {path}"))
            .await
            .unwrap()
            .row_count(),
        0
    );
    assert_eq!(
        session
            .query(&format!("MATCH REPEATABLE ELEMENTS {path}"))
            .await
            .unwrap()
            .row_count(),
        1
    );
    assert_eq!(
        session
            .query("MATCH REPEATABLE ELEMENTS (a)-[e:Knows]->(b), (a)-[e]->(b) RETURN e.since AS value")
            .await
            .unwrap()
            .row_count(),
        4
    );
    assert_eq!(
        session
            .query("MATCH DIFFERENT EDGES (a)-[e:Knows]->(b), (a)-[e]->(b) RETURN e.since AS value")
            .await
            .unwrap()
            .row_count(),
        0
    );
    assert_eq!(
        session
            .query("MATCH (a:Person), (b:City) RETURN a.name AS value")
            .await
            .unwrap()
            .row_count(),
        3
    );
    assert!(matches!(
        session
            .query("MATCH (a)-[a]->(b) RETURN b.name AS value")
            .await,
        Err(Error::InvalidQuery(_))
    ));
}

#[tokio::test]
async fn missing_and_null_properties_and_element_predicates() {
    let (_, mut session) = database();
    let result = session.query("MATCH (n) RETURN n.name AS name, PROPERTY_EXISTS(n, age) AS present, n.age AS age ORDER BY name").await.unwrap();
    assert_eq!(
        strings(&result),
        vec![
            vec!["Alice", "true", "30"],
            vec!["Bob", "true", "40"],
            vec!["Cara", "false", "NULL"],
            vec!["London", "false", "NULL"]
        ]
    );
    let result = session.query("MATCH (n:Person) WHERE n IS LABELED Admin RETURN n.name AS name, SAME(n,n) AS same, ALL_DIFFERENT(n,n) AS different, ELEMENT_ID(n) AS id").await.unwrap();
    let rows = strings(&result);
    assert_eq!(&rows[0][..3], &["Alice", "true", "false"]);
    assert!(rows[0][3].ends_with(":n:1"));
    let result = session
        .query("MATCH (a)-[e]-(b) WHERE e IS NOT DIRECTED RETURN e.since AS since ORDER BY since")
        .await
        .unwrap();
    assert_eq!(
        strings(&result),
        vec![vec!["2010"], vec!["2010"], vec!["2011"]]
    );
}

#[tokio::test]
async fn graph_selection_is_session_local_and_stale_ids_never_rebind() {
    let (db, mut session) = database();
    let old_id = session.state().current_graph.unwrap();
    let mut other = db.session();
    assert!(matches!(
        other.query("MATCH (n) RETURN n.name AS value").await,
        Err(Error::InvalidReference(_))
    ));
    assert_eq!(
        other
            .query("USE GRAPH social MATCH (n) RETURN n.name AS value")
            .await
            .unwrap()
            .row_count(),
        4
    );
    assert!(other.state().current_graph.is_none());
    other.set_parameter("g", Value::Graph(old_id)).unwrap();
    assert_eq!(
        other
            .query("USE GRAPH $g MATCH (n) RETURN n.name AS value")
            .await
            .unwrap()
            .row_count(),
        4
    );
    session
        .execute("DROP GRAPH social; CREATE GRAPH social ANY GRAPH")
        .unwrap();
    assert!(matches!(
        other
            .query("USE GRAPH $g MATCH (n) RETURN n.name AS value")
            .await,
        Err(Error::InvalidReference(_))
    ));
    assert!(session
        .query("MATCH (n) RETURN n.name AS value")
        .await
        .is_err());
    assert_eq!(
        other
            .query("USE GRAPH social MATCH (n) RETURN n.name AS value")
            .await
            .unwrap()
            .row_count(),
        0
    );
}

#[tokio::test]
async fn let_and_quoted_names_are_separate_from_physical_columns() {
    let (_, mut session) = database();
    let result = session.query("LET `__gf_id` = 1 MATCH (`a.b`:Person) LET score = `a.b`.age + `__gf_id` FILTER score > 35 RETURN `a.b`.name AS name, score ORDER BY score").await.unwrap();
    assert_eq!(strings(&result), vec![vec!["Bob", "41"]]);
    assert!(matches!(
        session.query("MATCH (n) RETURN __gf_id").await,
        Err(Error::InvalidQuery(_))
    ));
    assert!(session.query("LET n = 1 MATCH (n) RETURN n").await.is_err());
    let result = session
        .query("MATCH `a.b`=(n) RETURN PATH_LENGTH(`a.b`) AS hops")
        .await
        .unwrap();
    assert_eq!(strings(&result), vec![vec!["0"]; 4]);
}

#[test]
fn graph_import_validates_ids_endpoints_and_column_types() {
    assert!(GraphData::try_new(
        vec![nodes(&["A"], vec![1, 1], vec!["a", "b"], None)],
        vec![]
    )
    .is_err());
    assert!(GraphData::try_new(
        vec![nodes(&["A"], vec![1], vec!["a"], None)],
        vec![edges(true, "R", &[(2, 1, 99, 2020)])]
    )
    .is_err());
    assert!(GraphData::try_new(
        vec![nodes(&["A"], vec![1], vec!["a"], None)],
        vec![edges(true, "R", &[(2, 1, 1, 2020), (2, 1, 1, 2021)])]
    )
    .is_err());
    assert!(NodeTable::try_new(
        vec![],
        Arc::new(Schema::new(vec![Field::new(ID, DataType::Int64, false)])),
        vec![]
    )
    .is_err());
    assert!(NodeTable::try_new(
        vec![],
        Arc::new(Schema::new(vec![Field::new(ID, DataType::UInt64, true)])),
        vec![]
    )
    .is_err());
    assert_eq!(fixture().node_count(), 4);
    assert_eq!(fixture().edge_count(), 6);
    assert!(GraphData::try_new(
        vec![
            nodes(&["A"], vec![1], vec!["a"], None),
            nodes(&["B"], vec![1], vec!["b"], None)
        ],
        vec![],
    )
    .is_err());
}

#[tokio::test]
async fn graph_identity_and_schema_context_are_not_just_row_ids() {
    let (_, mut session) = database();
    session
        .execute("CREATE GRAPH mirror ANY GRAPH; SESSION SET GRAPH mirror")
        .unwrap();
    session.replace_graph_data(fixture()).unwrap();
    let result = session.query("USE GRAPH social MATCH (a {name: 'Alice'}) USE GRAPH mirror MATCH (b {name: 'Alice'}) RETURN SAME(a,b) AS same, ELEMENT_ID(a) AS first, ELEMENT_ID(b) AS second").await.unwrap();
    let rows = strings(&result);
    assert_eq!(rows[0][0], "false");
    assert_ne!(rows[0][1], rows[0][2]);
    assert!(rows[0][1].ends_with(":n:1"));
    assert!(rows[0][2].ends_with(":n:1"));
    session.execute("CREATE SCHEMA app; SESSION SET SCHEMA app; CREATE GRAPH social ANY GRAPH; SESSION SET GRAPH social").unwrap();
    session.replace_graph_data(fixture()).unwrap();
    session
        .execute("SESSION SET SCHEMA /main; SESSION RESET GRAPH")
        .unwrap();
    let state = session.state().clone();
    let result = session
        .query("AT SCHEMA app USE GRAPH social MATCH (n:City) RETURN n.name AS name")
        .await
        .unwrap();
    assert_eq!(strings(&result), vec![vec!["London"]]);
    assert_eq!(session.state(), &state);
}

#[tokio::test]
async fn binding_constraints_empty_import_and_explicit_null_ordering() {
    let (_, mut session) = database();
    let query = "MATCH (a {name: 'Cara'})-[e:Knows]->(a) MATCH (a)-[e]->(a) RETURN a.name AS name";
    assert_eq!(session.query(query).await.unwrap().row_count(), 1); // different-edges scope resets per MATCH
    assert_eq!(
        session
            .query("MATCH (n:NoSuchLabel) RETURN n.name AS name")
            .await
            .unwrap()
            .row_count(),
        0
    );
    let elements = session.query("MATCH (n) RETURN n").await.unwrap();
    assert!(matches!(
        elements.schema.field(0).data_type(),
        DataType::Struct(_)
    ));
    assert_eq!(elements.row_count(), 4);
    let result = session
        .query("MATCH (n:Person) RETURN n.name AS name, n.age AS age ORDER BY age ASC NULLS FIRST")
        .await
        .unwrap();
    assert_eq!(
        strings(&result),
        vec![vec!["Cara", "NULL"], vec!["Alice", "30"], vec!["Bob", "40"]]
    );
    session.replace_graph_data(GraphData::default()).unwrap();
    assert_eq!(
        session
            .query("MATCH (n) RETURN ELEMENT_ID(n) AS id")
            .await
            .unwrap()
            .row_count(),
        0
    );
    session
        .replace_graph_data(
            GraphData::try_new(
                vec![nodes(&["Person"], vec![1], vec!["Alice"], None)],
                vec![edges(true, "Loop", &[(1, 1, 1, 2020)])],
            )
            .unwrap(),
        )
        .unwrap();
    let result = session.query("MATCH (a)-[e]->(b) RETURN SAME(a,e) AS same, ELEMENT_ID(a) AS node, ELEMENT_ID(e) AS edge").await.unwrap();
    let rows = strings(&result);
    assert_eq!(rows[0][0], "false");
    assert_ne!(rows[0][1], rows[0][2]);
}
