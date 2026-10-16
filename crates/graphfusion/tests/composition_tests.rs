use datafusion::arrow::datatypes::DataType;
use graphfusion::{Database, Value};
use std::collections::BTreeMap;

fn strings(result: &graphfusion::QueryResult) -> Vec<Vec<String>> {
    result
        .batches
        .iter()
        .flat_map(|batch| {
            (0..batch.num_rows())
                .map(|row| {
                    batch
                        .columns()
                        .iter()
                        .map(|array| {
                            datafusion::common::ScalarValue::try_from_array(array, row)
                                .unwrap()
                                .to_string()
                        })
                        .collect()
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

async fn social() -> graphfusion::Session {
    let mut session = Database::new().session();
    session
        .execute("CREATE GRAPH g ANY GRAPH; SESSION SET GRAPH g")
        .unwrap();
    session
        .run(
            "INSERT (:Person {name: 'Ada', id: 1, score: 1}),
                    (:Person {name: 'Bea', id: 2, score: 2});
             INSERT (:Document {title: 'Spec', id: 'doc'});
             INSERT (:Metric {name: 'load', score: 1.5})",
        )
        .await
        .unwrap();
    session
}

#[tokio::test]
async fn label_filters_scan_only_compatible_fragments() {
    let mut session = social().await;
    let people = session
        .query("MATCH (p:Person) RETURN p.id AS id, p.name AS name ORDER BY name")
        .await
        .unwrap();
    assert_eq!(people.schema.field(0).data_type(), &DataType::Int64);
    assert_eq!(
        strings(&people),
        vec![
            vec!["1".to_string(), "Ada".to_string()],
            vec!["2".to_string(), "Bea".to_string()]
        ]
    );
    assert!(
        !people.logical_plan.contains("title"),
        "person scan included a document column:\n{}",
        people.logical_plan
    );
    let filtered = session
        .query("MATCH (p:Person WHERE p.id = 1) RETURN p.name AS name")
        .await
        .unwrap();
    assert_eq!(strings(&filtered), vec![vec!["Ada".to_string()]]);

    let document = session
        .query("MATCH (d:Document) RETURN d.id AS id, d.title AS title")
        .await
        .unwrap();
    assert_eq!(document.schema.field(0).data_type(), &DataType::Utf8);
    assert_eq!(
        strings(&document),
        vec![vec!["doc".to_string(), "Spec".to_string()]]
    );
    assert!(
        !document.logical_plan.contains("score"),
        "document scan included another type's property:\n{}",
        document.logical_plan
    );

    let excluded = session
        .query("MATCH (n:!Person) RETURN n.title AS title ORDER BY title")
        .await
        .unwrap();
    assert_eq!(
        strings(&excluded),
        vec![vec!["Spec".to_string()], vec!["NULL".to_string()]]
    );

    let widened = session
        .query("MATCH (n:Person|Metric) RETURN n.score AS score ORDER BY score")
        .await
        .unwrap();
    assert_eq!(widened.schema.field(0).data_type(), &DataType::Float64);
    assert_eq!(
        strings(&widened),
        vec![
            vec!["1".to_string()],
            vec!["1.5".to_string()],
            vec!["2".to_string()]
        ]
    );

    let mixed = session
        .query("MATCH (n:Person|Document) RETURN n.id AS id ORDER BY id")
        .await
        .unwrap();
    assert_eq!(mixed.schema.field(0).data_type(), &DataType::Utf8);
    assert_eq!(
        strings(&mixed),
        vec![
            vec!["1".to_string()],
            vec!["2".to_string()],
            vec!["doc".to_string()]
        ]
    );

    let reference = session
        .query("MATCH (p:Person WHERE p.name = 'Ada') LET r = p RETURN r.name AS name, r.id AS id")
        .await
        .unwrap();
    assert_eq!(
        strings(&reference),
        vec![vec!["Ada".to_string(), "1".to_string()]]
    );

    let updated = statement(
        &mut session,
        "MATCH (p:Person WHERE p.name = 'Ada') SET p.score = p.score + 1 RETURN p.id AS id, p.score AS score",
    )
    .await;
    assert_eq!(updated.schema.field(0).data_type(), &DataType::Int64);
    assert_eq!(updated.schema.field(1).data_type(), &DataType::Int64);
    assert_eq!(
        strings(&updated),
        vec![vec!["1".to_string(), "2".to_string()]]
    );

    let inserted = statement(
        &mut session,
        "INSERT (p:Person {name: 'Cy', id: 3, score: 3})
         SET p.score = 4
         RETURN p.id AS id, p.score AS score",
    )
    .await;
    assert_eq!(inserted.schema.field(0).data_type(), &DataType::Int64);
    assert_eq!(
        strings(&inserted),
        vec![vec!["3".to_string(), "4".to_string()]]
    );
}

async fn statement(session: &mut graphfusion::Session, query: &str) -> graphfusion::QueryResult {
    let output = session.run(query).await.unwrap().pop().unwrap();
    match output {
        graphfusion::StatementOutput::Query(result) => result,
        _ => panic!("query output expected"),
    }
}

#[tokio::test]
async fn next_passes_the_working_table() {
    let mut session = social().await;
    let chained = session
        .query(
            "MATCH (p:Person) RETURN p.name AS name, p.id AS id
             NEXT FILTER name = 'Bea'
             RETURN id",
        )
        .await
        .unwrap();
    assert_eq!(strings(&chained), vec![vec!["2".to_string()]]);

    let yielded = session
        .query("RETURN 1 AS n, 2 AS m NEXT YIELD n AS kept RETURN kept * 3 AS value")
        .await
        .unwrap();
    assert_eq!(strings(&yielded), vec![vec!["3".to_string()]]);

    let continued = session
        .query("RETURN 1 AS n NEXT RETURN n + 1 AS n NEXT RETURN n * 10 AS n")
        .await
        .unwrap();
    assert_eq!(strings(&continued), vec![vec!["20".to_string()]]);

    let empty = session
        .query("RETURN 1 AS n NEXT FILTER n > 1 RETURN n")
        .await
        .unwrap();
    assert_eq!(empty.row_count(), 0);

    assert!(session
        .query("RETURN 1 AS n, 2 AS m NEXT YIELD n RETURN m")
        .await
        .is_err());
    assert!(session
        .run("CREATE GRAPH rolled ANY GRAPH NEXT RETURN 1 AS n")
        .await
        .is_err());
}

#[tokio::test]
async fn next_keeps_element_references_readable() {
    let mut session = Database::new().session();
    session
        .execute("CREATE GRAPH g ANY; SESSION SET GRAPH g")
        .unwrap();
    session
        .run(
            "INSERT (a:P {name:'Ada', age:36}), (b:P {name:'Bea', age:41}),
                    (a)-[:E {kind:'knows'}]->(b)",
        )
        .await
        .unwrap();

    let people = session
        .query(
            "MATCH (p:P) RETURN p AS person
             NEXT RETURN person.name AS name, person.age AS age ORDER BY name",
        )
        .await
        .unwrap();
    assert_eq!(
        people.schema.field_with_name("age").unwrap().data_type(),
        &DataType::Int64
    );
    assert_eq!(
        strings(&people),
        vec![
            vec!["Ada".to_string(), "36".to_string()],
            vec!["Bea".to_string(), "41".to_string()]
        ]
    );

    let renamed = session
        .query(
            "MATCH (p:P) RETURN p
             NEXT YIELD p AS q FILTER q.name = 'Ada'
             RETURN q.age AS age",
        )
        .await
        .unwrap();
    assert_eq!(strings(&renamed), vec![vec!["36".to_string()]]);

    let continued = session
        .query(
            "MATCH (p:P) RETURN p AS person
             NEXT RETURN person AS q
             NEXT RETURN q.name AS name ORDER BY name",
        )
        .await
        .unwrap();
    assert_eq!(
        strings(&continued),
        vec![vec!["Ada".to_string()], vec!["Bea".to_string()]]
    );

    let edge = session
        .query(
            "MATCH ()-[e:E]->() RETURN e
             NEXT RETURN e.kind AS kind, e IS DIRECTED AS directed",
        )
        .await
        .unwrap();
    assert_eq!(
        strings(&edge),
        vec![vec!["knows".to_string(), "true".to_string()]]
    );

    let hopped = session
        .query(
            "MATCH (p:P {name:'Ada'}) RETURN p
             NEXT MATCH (p)-[:E]->(f)
             RETURN f.name AS name",
        )
        .await
        .unwrap();
    assert_eq!(strings(&hopped), vec![vec!["Bea".to_string()]]);

    let missing = session
        .query(
            "MATCH (p:P)
             OPTIONAL MATCH (p)-[:E]->(f)
             RETURN p AS person, f AS friend
             NEXT RETURN person.name AS name, friend.name AS friend
             ORDER BY name, friend",
        )
        .await
        .unwrap();
    assert_eq!(
        strings(&missing),
        vec![
            vec!["Ada".to_string(), "Bea".to_string()],
            vec!["Bea".to_string(), "NULL".to_string()]
        ]
    );
}

#[tokio::test]
async fn binding_table_inputs_join_graph_matches() {
    let mut session = social().await;
    session
        .set_parameter(
            "seeds",
            Value::BindingTable(vec![
                BTreeMap::from([("id".to_string(), Value::Integer(2))]),
                BTreeMap::from([("id".to_string(), Value::Integer(1))]),
            ]),
        )
        .unwrap();
    let joined = session
        .query(
            "TABLE $seeds
             MATCH (p:Person WHERE p.id = id)
             RETURN p.name AS name ORDER BY name",
        )
        .await
        .unwrap();
    assert_eq!(
        strings(&joined),
        vec![vec!["Ada".to_string()], vec!["Bea".to_string()]]
    );

    let inline = session
        .query("TABLE { MATCH (d:Document) RETURN d.title AS title } RETURN title")
        .await
        .unwrap();
    assert_eq!(strings(&inline), vec![vec!["Spec".to_string()]]);

    session
        .set_parameter("empty", Value::BindingTable(vec![]))
        .unwrap();
    let none = session.query("TABLE $empty RETURN 1 AS n").await.unwrap();
    assert_eq!(none.row_count(), 0);

    let piped = session
        .query(
            "TABLE { RETURN 'Ada' AS name }
             RETURN name
             NEXT MATCH (p:Person WHERE p.name = name)
             RETURN p.id AS id",
        )
        .await
        .unwrap();
    assert_eq!(strings(&piped), vec![vec!["1".to_string()]]);
}
