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

#[tokio::test]
async fn missing_label_does_not_inherit_other_property_types() {
    let mut session = social().await;
    let missing = session
        .query("MATCH (n:Missing) WHERE n.id = 1 RETURN n.id AS id")
        .await
        .unwrap();
    assert_eq!(missing.row_count(), 0);
    let grouped = session
        .query("MATCH (n:Missing) RETURN n.id AS id, COUNT(*) AS c")
        .await
        .unwrap();
    assert_eq!(grouped.row_count(), 0);
}

#[tokio::test]
async fn integer_values_widen_into_existing_float_properties() {
    let mut session = social().await;
    let inserted = statement(
        &mut session,
        "INSERT (m:Metric {name: 'qps', score: 2}) RETURN m.score AS score",
    )
    .await;
    assert_eq!(inserted.schema.field(0).data_type(), &DataType::Float64);
    assert_eq!(strings(&inserted), vec![vec!["2".to_string()]]);

    let updated = statement(
        &mut session,
        "MATCH (m:Metric WHERE m.name = 'load') SET m.score = 4 RETURN m.score AS score",
    )
    .await;
    assert_eq!(updated.schema.field(0).data_type(), &DataType::Float64);
    assert_eq!(strings(&updated), vec![vec!["4".to_string()]]);

    assert!(session
        .run("INSERT (:Metric {name: 'bad', score: 'nope'})")
        .await
        .is_err());
    let scores = session
        .query("MATCH (m:Metric) RETURN m.score AS score ORDER BY score")
        .await
        .unwrap();
    assert_eq!(scores.schema.field(0).data_type(), &DataType::Float64);
    assert_eq!(
        strings(&scores),
        vec![vec!["2".to_string()], vec!["4".to_string()]]
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

    session
        .set_parameter(
            "blank",
            Value::BindingTable(vec![BTreeMap::from([("id".to_string(), Value::Null)])]),
        )
        .unwrap();
    let blank = session
        .query(
            "TABLE $blank
             MATCH (p:Person WHERE p.id = id)
             RETURN p.name AS name",
        )
        .await
        .unwrap();
    assert_eq!(blank.row_count(), 0);

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

#[tokio::test]
async fn refresh_keeps_the_bound_label_set_schema() {
    let mut session = Database::new().session();
    session
        .execute("CREATE GRAPH g ANY GRAPH; SESSION SET GRAPH g")
        .unwrap();
    session
        .run(
            "INSERT (:Person {name: 'Ada', score: 1}),
                    (:Person&Extra {name: 'Bea', score: 2, note: 'hi'})",
        )
        .await
        .unwrap();
    let updated = session
        .run(
            "MATCH (p:Person&!Extra)
             SET p.flag = true
             SET p.note = 1
             RETURN p.name AS name, p.note = 1 AS same, p IS LABELED Extra AS extra",
        )
        .await
        .unwrap()
        .pop()
        .unwrap();
    let graphfusion::StatementOutput::Query(updated) = updated else {
        panic!("query result expected");
    };
    assert_eq!(updated.schema.field(1).data_type(), &DataType::Boolean);
    assert_eq!(
        strings(&updated),
        vec![vec![
            "Ada".to_string(),
            "true".to_string(),
            "false".to_string()
        ]]
    );
    let stored = session
        .query("MATCH (p:Person&!Extra) RETURN p.note AS note, p.flag AS flag")
        .await
        .unwrap();
    assert_eq!(stored.schema.field(0).data_type(), &DataType::Int64);
    assert_eq!(stored.schema.field(1).data_type(), &DataType::Boolean);
    assert_eq!(
        strings(&stored),
        vec![vec!["1".to_string(), "true".to_string()]]
    );
    let labeled = session
        .run(
            "MATCH (p:Person&!Extra WHERE p.name = 'Ada')
             SET p IS Retired
             RETURN p IS LABELED Retired AS retired, p.note = 1 AS same, p.name AS name",
        )
        .await
        .unwrap()
        .pop()
        .unwrap();
    let graphfusion::StatementOutput::Query(labeled) = labeled else {
        panic!("query result expected");
    };
    assert_eq!(
        strings(&labeled),
        vec![vec![
            "true".to_string(),
            "true".to_string(),
            "Ada".to_string()
        ]]
    );
}

#[tokio::test]
async fn binding_table_rejects_non_numeric_mixed_columns() {
    let mut session = Database::new().session();
    session
        .set_parameter(
            "mixed",
            Value::BindingTable(vec![
                BTreeMap::from([("v".to_string(), Value::Boolean(true))]),
                BTreeMap::from([("v".to_string(), Value::Integer(1))]),
            ]),
        )
        .unwrap();
    let error = session
        .query("TABLE $mixed RETURN v AS v")
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("mixed types for binding-table column v"),
        "{error}"
    );

    session
        .set_parameter(
            "nums",
            Value::BindingTable(vec![
                BTreeMap::from([("v".to_string(), Value::Integer(1))]),
                BTreeMap::from([("v".to_string(), Value::Float(1.5))]),
            ]),
        )
        .unwrap();
    let nums = session
        .query("TABLE $nums RETURN v AS v ORDER BY v")
        .await
        .unwrap();
    assert_eq!(nums.schema.field(0).data_type(), &DataType::Float64);
    assert_eq!(
        strings(&nums),
        vec![vec!["1".to_string()], vec!["1.5".to_string()]]
    );
}

#[tokio::test]
async fn mixed_label_filters_compare_stored_types() {
    let mut session = social().await;
    let equal = session
        .query("MATCH (n:Person|Document) WHERE n.id = 1 RETURN n.name AS name")
        .await
        .unwrap();
    assert_eq!(strings(&equal), vec![vec!["Ada".to_string()]]);

    let mapped = session
        .query("MATCH (n:Person|Document {id: 1}) RETURN n.name AS name")
        .await
        .unwrap();
    assert_eq!(strings(&mapped), vec![vec!["Ada".to_string()]]);

    let greater = session
        .query("MATCH (n:Person|Document) WHERE n.id > 1 RETURN n.name AS name")
        .await
        .unwrap();
    assert_eq!(strings(&greater), vec![vec!["Bea".to_string()]]);

    let above_digit = session
        .query("MATCH (n:Person|Document) WHERE n.id > 9 RETURN n.name AS name")
        .await
        .unwrap();
    assert_eq!(above_digit.row_count(), 0);

    let document = session
        .query("MATCH (n:Person|Document) WHERE n.id = 'doc' RETURN n.title AS title")
        .await
        .unwrap();
    assert_eq!(strings(&document), vec![vec!["Spec".to_string()]]);

    let either = session
        .query(
            "MATCH (n:Person|Document) WHERE n.id = 1 OR n.title = 'Spec'
             RETURN n.id AS id, n.title AS title ORDER BY id",
        )
        .await
        .unwrap();
    assert_eq!(either.schema.field(0).data_type(), &DataType::Utf8);
    assert_eq!(
        strings(&either),
        vec![
            vec!["1".to_string(), "NULL".to_string()],
            vec!["doc".to_string(), "Spec".to_string()]
        ]
    );

    let negated = session
        .query("MATCH (n:Person|Document) WHERE NOT n.id = 1 RETURN n.name AS name ORDER BY name")
        .await
        .unwrap();
    assert_eq!(strings(&negated), vec![vec!["Bea".to_string()]]);

    let widened = session
        .query("MATCH (n:Person|Metric) WHERE n.score > 1 RETURN n.score AS score ORDER BY score")
        .await
        .unwrap();
    assert_eq!(widened.schema.field(0).data_type(), &DataType::Float64);
    assert_eq!(
        strings(&widened),
        vec![vec!["1.5".to_string()], vec!["2".to_string()]]
    );

    let both = session
        .query(
            "MATCH (n:Person|Document), (m:Metric)
             WHERE n.id = 1 AND m.name = 'load'
             RETURN n.name AS name, m.score AS score",
        )
        .await
        .unwrap();
    assert_eq!(
        strings(&both),
        vec![vec!["Ada".to_string(), "1.5".to_string()]]
    );
}

#[tokio::test]
async fn element_reference_keeps_the_matched_property_type() {
    let mut session = social().await;
    let typed = session
        .query(
            "MATCH (p:Person WHERE p.name = 'Ada')
             LET r = p
             RETURN r.id AS id, r.score AS score",
        )
        .await
        .unwrap();
    assert_eq!(typed.schema.field(0).data_type(), &DataType::Int64);
    assert_eq!(typed.schema.field(1).data_type(), &DataType::Int64);
    assert_eq!(
        strings(&typed),
        vec![vec!["1".to_string(), "1".to_string()]]
    );
}
