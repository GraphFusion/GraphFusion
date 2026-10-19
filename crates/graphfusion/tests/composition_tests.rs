use datafusion::arrow::datatypes::DataType;
use graphfusion::{Database, Error, Value};
use std::collections::BTreeMap;

async fn pair_rows(session: &mut graphfusion::Session, query: &str) -> Vec<Vec<String>> {
    strings(&session.query(query).await.unwrap())
}

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
    let widened_or = session
        .query(
            "MATCH (n:Person|Metric) WHERE n.score > 1 OR FALSE
             RETURN n.score AS score ORDER BY score",
        )
        .await
        .unwrap();
    assert_eq!(
        strings(&widened_or),
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
async fn mixed_label_where_compares_bound_values() {
    let mut session = social().await;
    let equal = session
        .query(
            "LET k = 1
             MATCH (n:Person|Document) WHERE n.id = k
             RETURN n.name AS name",
        )
        .await
        .unwrap();
    assert_eq!(strings(&equal), vec![vec!["Ada".to_string()]]);

    let flipped = session
        .query(
            "LET k = 1
             MATCH (n:Person|Document) WHERE k = n.id
             RETURN n.name AS name",
        )
        .await
        .unwrap();
    assert_eq!(strings(&flipped), vec![vec!["Ada".to_string()]]);

    let text = session
        .query(
            "LET k = 'doc'
             MATCH (n:Person|Document) WHERE n.id = k
             RETURN n.title AS title",
        )
        .await
        .unwrap();
    assert_eq!(strings(&text), vec![vec!["Spec".to_string()]]);

    let not_stringified = session
        .query(
            "LET k = '1'
             MATCH (n:Person|Document) WHERE n.id = k
             RETURN n.name AS name",
        )
        .await
        .unwrap();
    assert_eq!(not_stringified.row_count(), 0);

    let inline = session
        .query(
            "LET k = 1
             MATCH (n:Person|Document WHERE n.id = k)
             RETURN n.name AS name",
        )
        .await
        .unwrap();
    assert_eq!(strings(&inline), vec![vec!["Ada".to_string()]]);

    let disjunct = session
        .query(
            "LET keep = FALSE
             MATCH (n:Person|Document) WHERE n.id = 1 OR keep
             RETURN n.name AS name",
        )
        .await
        .unwrap();
    assert_eq!(strings(&disjunct), vec![vec!["Ada".to_string()]]);

    let widened = session
        .query(
            "LET k = 1
             MATCH (n:Person|Metric) WHERE n.score > k
             RETURN n.score AS score ORDER BY score",
        )
        .await
        .unwrap();
    assert_eq!(widened.schema.field(0).data_type(), &DataType::Float64);
    assert_eq!(
        strings(&widened),
        vec![vec!["1.5".to_string()], vec!["2".to_string()]]
    );

    let both = session
        .query(
            "LET k = 1
             MATCH (n:Person|Document), (m:Metric)
             WHERE n.id = k AND m.name = 'load'
             RETURN n.name AS name, m.score AS score",
        )
        .await
        .unwrap();
    assert_eq!(
        strings(&both),
        vec![vec!["Ada".to_string(), "1.5".to_string()]]
    );

    session
        .run(
            "MATCH (a:Person {name: 'Ada'}), (b:Person {name: 'Bea'})
             INSERT (a)-[:Knows {since: 1}]->(b);
             MATCH (d:Document)
             INSERT (d)-[:Cites {since: 'old'}]->(d)",
        )
        .await
        .unwrap();
    let edge = session
        .query(
            "LET k = 1
             MATCH ()-[e:Knows|Cites]->() WHERE e.since = k
             RETURN e.since AS since",
        )
        .await
        .unwrap();
    assert_eq!(strings(&edge), vec![vec!["1".to_string()]]);

    let incompatible = session
        .query(
            "LET k = 1
             MATCH (d:Document) WHERE d.id = k
             RETURN d.title AS title",
        )
        .await;
    assert!(
        matches!(incompatible, Err(graphfusion::Error::InvalidQuery(_))),
        "{incompatible:?}"
    );
}

#[tokio::test]
async fn mixed_label_where_keeps_boolean_literals() {
    let mut session = social().await;
    let or_false = session
        .query(
            "MATCH (n:Person|Document) WHERE n.id = 1 OR FALSE
             RETURN n.name AS name",
        )
        .await
        .unwrap();
    assert_eq!(strings(&or_false), vec![vec!["Ada".to_string()]]);

    let or_true = session
        .query(
            "MATCH (n:Person|Document) WHERE FALSE OR n.id = 1 OR TRUE
             RETURN n.id AS id ORDER BY id",
        )
        .await
        .unwrap();
    assert_eq!(
        strings(&or_true),
        vec![
            vec!["1".to_string()],
            vec!["2".to_string()],
            vec!["doc".to_string()]
        ]
    );

    let and_true = session
        .query(
            "MATCH (n:Person|Document) WHERE TRUE AND n.id = 1
             RETURN n.name AS name",
        )
        .await
        .unwrap();
    assert_eq!(strings(&and_true), vec![vec!["Ada".to_string()]]);

    let and_false = session
        .query("MATCH (n:Person|Document) WHERE n.id = 1 AND FALSE RETURN n.name AS name")
        .await
        .unwrap();
    assert_eq!(and_false.row_count(), 0);

    let not_stringified = session
        .query("MATCH (n:Person|Document) WHERE n.id = '1' OR FALSE RETURN n.name AS name")
        .await
        .unwrap();
    assert_eq!(not_stringified.row_count(), 0);

    let xor_false = session
        .query(
            "MATCH (n:Person|Document) WHERE n.id = 1 XOR FALSE
             RETURN n.name AS name",
        )
        .await
        .unwrap();
    assert_eq!(strings(&xor_false), vec![vec!["Ada".to_string()]]);

    let xor_true = session
        .query(
            "MATCH (n:Person|Document) WHERE n.id = 1 XOR TRUE
             RETURN n.name AS name",
        )
        .await
        .unwrap();
    assert_eq!(strings(&xor_true), vec![vec!["Bea".to_string()]]);

    let unknown = session
        .query(
            "MATCH (n:Person|Document) WHERE n.id = 1 OR UNKNOWN
             RETURN n.name AS name",
        )
        .await
        .unwrap();
    assert_eq!(strings(&unknown), vec![vec!["Ada".to_string()]]);

    let inline = session
        .query(
            "MATCH (n:Person|Document WHERE NOT FALSE AND n.id = 1)
             RETURN n.name AS name",
        )
        .await
        .unwrap();
    assert_eq!(strings(&inline), vec![vec!["Ada".to_string()]]);

    let widened = session
        .query(
            "MATCH (n:Person|Metric) WHERE n.score > 1 OR FALSE
             RETURN n.score AS score ORDER BY score",
        )
        .await
        .unwrap();
    assert_eq!(
        strings(&widened),
        vec![vec!["1.5".to_string()], vec!["2".to_string()]]
    );

    let both = session
        .query(
            "MATCH (n:Person|Document), (m:Metric)
             WHERE (n.id = 1 OR FALSE) AND m.name = 'load' AND TRUE
             RETURN n.name AS name, m.score AS score",
        )
        .await
        .unwrap();
    assert_eq!(
        strings(&both),
        vec![vec!["Ada".to_string(), "1.5".to_string()]]
    );

    session
        .execute("SESSION SET VALUE $keep BOOLEAN = FALSE")
        .unwrap();
    let parameter = session
        .query(
            "MATCH (n:Person|Document) WHERE n.id = 1 OR $keep
             RETURN n.name AS name",
        )
        .await
        .unwrap();
    assert_eq!(strings(&parameter), vec![vec!["Ada".to_string()]]);

    let incompatible = session
        .query("MATCH (d:Document) WHERE d.id = 1 OR TRUE RETURN d.title AS title")
        .await;
    assert!(
        matches!(incompatible, Err(graphfusion::Error::InvalidQuery(_))),
        "{incompatible:?}"
    );
}

#[tokio::test]
async fn filter_clause_compares_stored_types() {
    let mut session = social().await;
    let equal = session
        .query("MATCH (n:Person|Document) FILTER n.id = 1 RETURN n.name AS name")
        .await
        .unwrap();
    assert_eq!(strings(&equal), vec![vec!["Ada".to_string()]]);

    let flipped = session
        .query("MATCH (n:Person|Document) FILTER 1 = n.id RETURN n.name AS name")
        .await
        .unwrap();
    assert_eq!(strings(&flipped), vec![vec!["Ada".to_string()]]);

    let either = session
        .query(
            "MATCH (n:Person|Document) FILTER n.id = 1 OR n.id = 'doc'
             RETURN n.id AS id ORDER BY id",
        )
        .await
        .unwrap();
    assert_eq!(either.schema.field(0).data_type(), &DataType::Utf8);
    assert_eq!(
        strings(&either),
        vec![vec!["1".to_string()], vec!["doc".to_string()]]
    );

    let not_stringified = session
        .query("MATCH (n:Person|Document) FILTER n.id = '1' RETURN n.name AS name")
        .await
        .unwrap();
    assert_eq!(not_stringified.row_count(), 0);

    let negated = session
        .query("MATCH (n:Person|Document) FILTER NOT n.id = 1 RETURN n.name AS name ORDER BY name")
        .await
        .unwrap();
    assert_eq!(strings(&negated), vec![vec!["Bea".to_string()]]);

    let widened = session
        .query("MATCH (n:Person|Metric) FILTER n.score > 1 RETURN n.score AS score ORDER BY score")
        .await
        .unwrap();
    assert_eq!(widened.schema.field(0).data_type(), &DataType::Float64);
    assert_eq!(
        strings(&widened),
        vec![vec!["1.5".to_string()], vec!["2".to_string()]]
    );

    let with_scalar = session
        .query(
            "MATCH (n:Person|Document) LET keep = TRUE
             FILTER n.id = 1 OR NOT keep
             RETURN n.name AS name",
        )
        .await
        .unwrap();
    assert_eq!(strings(&with_scalar), vec![vec!["Ada".to_string()]]);

    let across = session
        .query(
            "MATCH (n:Person|Document), (m:Metric)
             FILTER n.id = 1 AND m.score = 1.5
             RETURN n.name AS name, m.score AS score",
        )
        .await
        .unwrap();
    assert_eq!(
        strings(&across),
        vec![vec!["Ada".to_string(), "1.5".to_string()]]
    );

    let exclusive = session
        .query(
            "MATCH (n:Person|Document) FILTER n.id = 1 XOR n.id <> 1
             RETURN n.name AS name ORDER BY name",
        )
        .await
        .unwrap();
    assert_eq!(
        strings(&exclusive),
        vec![vec!["Ada".to_string()], vec!["Bea".to_string()]]
    );

    let incompatible = session
        .query("MATCH (d:Document) FILTER d.id = 1 RETURN d.title AS title")
        .await;
    assert!(
        matches!(incompatible, Err(graphfusion::Error::InvalidQuery(_))),
        "{incompatible:?}"
    );

    let unchanged = session
        .query("MATCH (p:Person) FILTER FALSE RETURN p.name AS name")
        .await
        .unwrap();
    assert_eq!(unchanged.row_count(), 0);

    session
        .run(
            "MATCH (a:Person {name: 'Ada'}), (b:Person {name: 'Bea'})
             INSERT (a)-[:Knows {since: 1}]->(b);
             MATCH (d:Document)
             INSERT (d)-[:Cites {since: 'old'}]->(d)",
        )
        .await
        .unwrap();
    let edge = session
        .query(
            "MATCH ()-[e:Knows|Cites]->() FILTER e.since = 1
             RETURN e.since AS since",
        )
        .await
        .unwrap();
    assert_eq!(strings(&edge), vec![vec!["1".to_string()]]);
    let edge_text = session
        .query(
            "MATCH ()-[e:Knows|Cites]->() FILTER e.since = '1'
             RETURN e.since AS since",
        )
        .await
        .unwrap();
    assert_eq!(edge_text.row_count(), 0);
}

#[tokio::test]
async fn mixed_label_xor_compares_stored_types() {
    let mut session = social().await;
    let either = session
        .query(
            "MATCH (n:Person|Document) WHERE n.id = 1 XOR n.id <> 1
             RETURN n.name AS name ORDER BY name",
        )
        .await
        .unwrap();
    assert_eq!(
        strings(&either),
        vec![vec!["Ada".to_string()], vec!["Bea".to_string()]]
    );

    let one_of = session
        .query(
            "MATCH (n:Person|Document WHERE n.id = 1 XOR n.id = 2)
             RETURN n.name AS name ORDER BY name",
        )
        .await
        .unwrap();
    assert_eq!(
        strings(&one_of),
        vec![vec!["Ada".to_string()], vec!["Bea".to_string()]]
    );

    let crossed = session
        .query(
            "MATCH (n:Person|Document) WHERE n.id = 1 XOR n.id = 'doc'
             RETURN n.name AS name",
        )
        .await
        .unwrap();
    assert_eq!(crossed.row_count(), 0);

    let widened = session
        .query(
            "MATCH (n:Person|Metric) WHERE n.score > 1 XOR n.score < 1
             RETURN n.score AS score ORDER BY score",
        )
        .await
        .unwrap();
    assert_eq!(widened.schema.field(0).data_type(), &DataType::Float64);
    assert_eq!(
        strings(&widened),
        vec![vec!["1.5".to_string()], vec!["2".to_string()]]
    );

    let incompatible = session
        .query("MATCH (d:Document) WHERE d.id = 1 XOR d.id <> 1 RETURN d.title AS title")
        .await;
    assert!(
        matches!(incompatible, Err(graphfusion::Error::InvalidQuery(_))),
        "{incompatible:?}"
    );
}

async fn names(session: &mut graphfusion::Session, query: &str) -> Vec<String> {
    strings(&session.query(query).await.unwrap())
        .into_iter()
        .map(|row| row.into_iter().next().unwrap())
        .collect()
}

#[tokio::test]
async fn truth_predicates_compare_stored_types() {
    let mut session = Database::new().session();
    session
        .execute("CREATE GRAPH g ANY GRAPH; SESSION SET GRAPH g")
        .unwrap();
    session
        .run(
            "INSERT (:P {name: 'ten', v: 10}),
                    (:P {name: 'nine', v: 9}),
                    (:D {name: 'text', v: '10'})",
        )
        .await
        .unwrap();
    session.execute("SESSION SET VALUE $digits = '10'").unwrap();

    assert_eq!(
        names(
            &mut session,
            "MATCH (n) WHERE (n.v = '10') IS TRUE RETURN n.name AS name ORDER BY name"
        )
        .await,
        vec!["text".to_string()]
    );
    assert_eq!(
        names(
            &mut session,
            "MATCH (n) WHERE (n.v = '10') IS FALSE RETURN n.name AS name ORDER BY name"
        )
        .await,
        Vec::<String>::new()
    );
    assert_eq!(
        names(
            &mut session,
            "MATCH (n) WHERE (n.v = '10') IS UNKNOWN RETURN n.name AS name ORDER BY name"
        )
        .await,
        vec!["nine".to_string(), "ten".to_string()]
    );
    assert_eq!(
        names(
            &mut session,
            "MATCH (n) WHERE (n.v = '10') IS NOT TRUE RETURN n.name AS name ORDER BY name"
        )
        .await,
        vec!["nine".to_string(), "ten".to_string()]
    );
    assert_eq!(
        names(
            &mut session,
            "MATCH (n) WHERE (n.v = '10') IS NOT UNKNOWN RETURN n.name AS name ORDER BY name"
        )
        .await,
        vec!["text".to_string()]
    );
    assert_eq!(
        names(
            &mut session,
            "MATCH (n) WHERE (n.v = $digits) IS TRUE RETURN n.name AS name"
        )
        .await,
        vec!["text".to_string()]
    );
    assert_eq!(
        names(
            &mut session,
            "MATCH (n WHERE (n.v = '10') IS UNKNOWN) RETURN n.name AS name ORDER BY name"
        )
        .await,
        vec!["nine".to_string(), "ten".to_string()]
    );
    assert_eq!(
        names(
            &mut session,
            "MATCH (n:P) WHERE (n.v > 9) IS TRUE RETURN n.name AS name"
        )
        .await,
        vec!["ten".to_string()]
    );
    let mismatch = session
        .query("MATCH (n:D) WHERE (n.v > 9) IS TRUE RETURN n.name AS name")
        .await
        .unwrap_err();
    assert!(
        mismatch.to_string().contains("incomparable types"),
        "{mismatch}"
    );

    let mut session = social().await;
    assert_eq!(
        names(
            &mut session,
            "MATCH (n:Person|Metric) WHERE (n.score > 1) IS TRUE
             RETURN n.name AS name ORDER BY name"
        )
        .await,
        vec!["Bea".to_string(), "load".to_string()]
    );
}

#[tokio::test]
async fn parenthesized_where_compares_stored_types() {
    let mut session = Database::new().session();
    session
        .execute("CREATE GRAPH g ANY GRAPH; SESSION SET GRAPH g")
        .unwrap();
    session
        .run(
            "INSERT (:P {name: 'ten', v: 10}),
                    (:P {name: 'nine', v: 9}),
                    (:D {name: 'doc', v: 'x'}),
                    (:D {name: 'text', v: '10'})",
        )
        .await
        .unwrap();

    let greater = session
        .query("MATCH ((n) WHERE n.v > 9) RETURN n.name AS name ORDER BY name")
        .await
        .unwrap();
    assert!(
        greater.logical_plan.contains("__gf_frag_"),
        "parenthesized predicate stayed on the unified column:\n{}",
        greater.logical_plan
    );
    assert_eq!(strings(&greater), vec![vec!["ten".to_string()]]);

    let either = session
        .query(
            "MATCH ((n) WHERE n.v > 9 OR n.name = 'doc')
             RETURN n.name AS name ORDER BY name",
        )
        .await
        .unwrap();
    assert_eq!(
        strings(&either),
        vec![vec!["doc".to_string()], vec!["ten".to_string()]]
    );

    let text = session
        .query("MATCH ((n) WHERE n.v = '10') RETURN n.name AS name ORDER BY name")
        .await
        .unwrap();
    assert_eq!(strings(&text), vec![vec!["text".to_string()]]);

    let negated = session
        .query("MATCH ((n) WHERE NOT n.v = 9) RETURN n.name AS name ORDER BY name")
        .await
        .unwrap();
    assert_eq!(strings(&negated), vec![vec!["ten".to_string()]]);

    let same_label = session
        .query("MATCH ((n:P) WHERE n.v > 9) RETURN n.name AS name")
        .await
        .unwrap();
    assert!(
        !same_label.logical_plan.contains("__gf_frag_"),
        "single-type parenthesized predicate joined fragments:\n{}",
        same_label.logical_plan
    );
    assert_eq!(strings(&same_label), vec![vec!["ten".to_string()]]);

    let mismatch = session
        .query("MATCH ((n:D) WHERE n.v > 9) RETURN n.name AS name")
        .await
        .unwrap_err();
    assert!(
        mismatch.to_string().contains("incomparable types"),
        "{mismatch}"
    );
}

#[tokio::test]
async fn mixed_label_boolean_filters_do_not_stringify() {
    let mut session = Database::new().session();
    session
        .execute("CREATE GRAPH g ANY GRAPH; SESSION SET GRAPH g")
        .unwrap();
    session
        .run(
            "INSERT (:Person {name: 'Ada', id: 1}),
                    (:Person {name: 'Bea', id: 2}),
                    (:Document {title: 'Spec', id: '1'})",
        )
        .await
        .unwrap();

    let string_or_false = session
        .query(
            "MATCH (n:Person|Document) WHERE n.id = '1' OR FALSE
             RETURN n.name AS name, n.title AS title",
        )
        .await
        .unwrap();
    assert_eq!(
        strings(&string_or_false),
        vec![vec!["NULL".to_string(), "Spec".to_string()]]
    );

    let number_or_false = session
        .query("MATCH (n:Person|Document) WHERE n.id = 1 OR FALSE RETURN n.name AS name")
        .await
        .unwrap();
    assert_eq!(strings(&number_or_false), vec![vec!["Ada".to_string()]]);

    let or_true = session
        .query(
            "MATCH (n:Person|Document) WHERE n.id = 1 OR TRUE
             RETURN n.name AS name ORDER BY name",
        )
        .await
        .unwrap();
    assert_eq!(
        strings(&or_true),
        vec![
            vec!["Ada".to_string()],
            vec!["Bea".to_string()],
            vec!["NULL".to_string()]
        ]
    );

    let and_true = session
        .query("MATCH (n:Person|Document) WHERE n.id = 1 AND TRUE RETURN n.name AS name")
        .await
        .unwrap();
    assert_eq!(strings(&and_true), vec![vec!["Ada".to_string()]]);

    let xor_false = session
        .query("MATCH (n:Person|Document) WHERE n.id = 1 XOR FALSE RETURN n.name AS name")
        .await
        .unwrap();
    assert_eq!(strings(&xor_false), vec![vec!["Ada".to_string()]]);

    let xor_true = session
        .query("MATCH (n:Person|Document) WHERE n.id = 1 XOR TRUE RETURN n.name AS name")
        .await
        .unwrap();
    assert_eq!(strings(&xor_true), vec![vec!["Bea".to_string()]]);

    let negated = session
        .query("MATCH (n:Person|Document) WHERE NOT (n.id = 1 OR FALSE) RETURN n.name AS name")
        .await
        .unwrap();
    assert_eq!(strings(&negated), vec![vec!["Bea".to_string()]]);

    let filtered = session
        .query("MATCH (n:Person|Document) FILTER n.id = '1' RETURN n.title AS title")
        .await
        .unwrap();
    assert_eq!(strings(&filtered), vec![vec!["Spec".to_string()]]);

    let filtered_number = session
        .query("MATCH (n:Person|Document) FILTER n.id = 1 OR FALSE RETURN n.name AS name")
        .await
        .unwrap();
    assert_eq!(strings(&filtered_number), vec![vec!["Ada".to_string()]]);

    let defined = session
        .query(
            "MATCH (n:Person|Document) LET eq = n.id = '1'
             RETURN n.name AS name, eq AS eq ORDER BY name",
        )
        .await
        .unwrap();
    assert_eq!(
        strings(&defined),
        vec![
            vec!["Ada".to_string(), "NULL".to_string()],
            vec!["Bea".to_string(), "NULL".to_string()],
            vec!["NULL".to_string(), "true".to_string()]
        ]
    );

    let inline = session
        .query("MATCH (n:Person|Document WHERE n.id = '1' OR FALSE) RETURN n.title AS title")
        .await
        .unwrap();
    assert_eq!(strings(&inline), vec![vec!["Spec".to_string()]]);

    let selective = session
        .query("MATCH p = SHORTEST (n:Person|Document) WHERE n.id = '1' RETURN n.title AS title")
        .await
        .unwrap();
    assert_eq!(strings(&selective), vec![vec!["Spec".to_string()]]);

    let single = session
        .query("MATCH (n:Person) WHERE n.id = 1 AND TRUE RETURN n.name AS name")
        .await
        .unwrap();
    assert_eq!(strings(&single), vec![vec!["Ada".to_string()]]);
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

#[tokio::test]
async fn chained_element_copies_keep_stored_property_types() {
    let mut session = social().await;
    let result = session
        .query("MATCH (p:Person) LET r = p, s = r RETURN r.id AS rid, s.id AS sid, s.score AS score ORDER BY rid")
        .await
        .unwrap();
    for field in result.schema.fields() {
        assert_eq!(field.data_type(), &DataType::Int64);
    }
    assert_eq!(
        strings(&result),
        vec![
            vec!["1".to_string(), "1".to_string(), "1".to_string()],
            vec!["2".to_string(), "2".to_string(), "2".to_string()]
        ]
    );
}

#[tokio::test]
async fn successive_label_edits_keep_elements_and_copies_live() {
    let mut session = social().await;
    let result = statement(
        &mut session,
        "MATCH (p:Person WHERE p.name = 'Ada') LET r = p, s = r SET p:Extra SET p:Other RETURN p.id AS pid, r.id AS rid, s.id AS sid, s IS LABELED Other AS labeled",
    ).await;
    assert_eq!(
        strings(&result),
        vec![vec![
            "1".to_string(),
            "1".to_string(),
            "1".to_string(),
            "true".to_string()
        ]]
    );
    for field in result.schema.fields().iter().take(3) {
        assert_eq!(field.data_type(), &DataType::Int64);
    }
}

#[tokio::test]
async fn refresh_reads_properties_added_after_a_label_move() {
    let mut session = social().await;
    let result = statement(&mut session,
        "MATCH (p:Person WHERE p.name = 'Ada') LET r = p SET p:Retired SET p.flag = true RETURN p.flag AS flag, r.flag AS copied"
    ).await;
    assert_eq!(
        strings(&result),
        vec![vec!["true".to_string(), "true".to_string()]]
    );
}

#[tokio::test]
async fn refresh_adopts_recreated_property_types() {
    let mut session = social().await;
    let result = statement(&mut session,
        "MATCH (p:Person) LET r = p REMOVE p.score SET p.score = 'new' RETURN p.score AS score, r.score AS copied ORDER BY p.id"
    ).await;
    assert_eq!(
        strings(&result),
        vec![vec!["new".to_string(), "new".to_string()]; 2]
    );
    for field in result.schema.fields() {
        assert_eq!(field.data_type(), &DataType::Utf8);
    }
}

#[tokio::test]
async fn parenthesized_predicate_includes_later_alternative_fragments() {
    let mut s = Database::new().session();
    s.run("CREATE GRAPH g ANY; SESSION SET GRAPH g; INSERT (:P {name:'number',v:10}),(:D {name:'doc',v:'ten'}),(:Q {name:'late',v:'ten'})").await.unwrap();
    let r = s
        .query("MATCH ((n:P|D) | (n:Q|D) WHERE n.v = 'ten') RETURN n.name AS name ORDER BY name")
        .await
        .unwrap();
    assert_eq!(
        strings(&r),
        vec![vec!["doc".to_string()], vec!["late".to_string()]]
    );
}

#[tokio::test]
async fn stored_truth_values_use_the_matching_fragment() {
    let mut s = Database::new().session();
    s.run("CREATE GRAPH g ANY; SESSION SET GRAPH g; INSERT (:P {name:'ten',v:10}),(:P {name:'nine',v:9}),(:D {name:'text',v:'10'})").await.unwrap();
    let r = s.query("MATCH (n) LET classified = (n.v = '10') IS UNKNOWN RETURN n.name AS name, classified ORDER BY name").await.unwrap();
    assert_eq!(
        strings(&r),
        vec![
            vec!["nine".to_string(), "true".to_string()],
            vec!["ten".to_string(), "true".to_string()],
            vec!["text".to_string(), "false".to_string()]
        ]
    );
}

#[tokio::test]
async fn optional_misses_and_reference_aliases_keep_stored_boolean_semantics() {
    let mut s = Database::new().session();
    s.run("CREATE GRAPH g ANY; SESSION SET GRAPH g; INSERT (:P {name:'number',v:1}),(:D {name:'text',v:'1'})").await.unwrap();
    for q in [
        "OPTIONAL MATCH (n) WHERE FALSE LET classified = (n.v = '1') IS UNKNOWN RETURN classified",
        "OPTIONAL MATCH (n) WHERE FALSE FILTER n.v = '1' OR TRUE RETURN TRUE AS classified",
    ] {
        assert_eq!(
            strings(&s.query(q).await.unwrap()),
            vec![vec!["true".to_string()]],
            "{q}"
        );
    }
    for q in [
        "MATCH (n) LET r=n FILTER r.v = '1' RETURN r.name AS name",
        "MATCH (n) LET r=n LET eq=r.v = '1' FILTER eq RETURN r.name AS name",
    ] {
        assert_eq!(
            strings(&s.query(q).await.unwrap()),
            vec![vec!["text".to_string()]],
            "{q}"
        );
    }
}

async fn column(session: &mut graphfusion::Session, query: &str) -> Vec<String> {
    strings(&session.query(query).await.expect(query))
        .into_iter()
        .map(|row| row[0].clone())
        .collect()
}

#[tokio::test]
async fn coalesce_and_nullif_use_stored_types() {
    let mut session = Database::new().session();
    session
        .execute("CREATE GRAPH g ANY GRAPH; SESSION SET GRAPH g")
        .unwrap();
    session
        .run(
            "INSERT (:P {name:'ten', v:10}),
                    (:P {name:'nine', v:9}),
                    (:P {name:'blank'}),
                    (:D {name:'text', v:'10'}),
                    (:Q {name:'empty'})",
        )
        .await
        .unwrap();
    session
        .execute("SESSION SET VALUE $digits STRING = '10'")
        .unwrap();

    let text_only = [
        "MATCH (n) WHERE COALESCE(n.v = '10', FALSE) RETURN n.name AS name ORDER BY name",
        "MATCH (n) WHERE NULLIF(n.v = '10', FALSE) RETURN n.name AS name ORDER BY name",
        "MATCH (n WHERE COALESCE(n.v = '10', FALSE)) RETURN n.name AS name ORDER BY name",
        "MATCH (n) WHERE COALESCE(n.v = $digits, FALSE) RETURN n.name AS name ORDER BY name",
        "MATCH (n) WHERE COALESCE(n.v, 'zz') = '10' RETURN n.name AS name ORDER BY name",
        "MATCH (n) WHERE '10' = COALESCE(n.v, 'zz') RETURN n.name AS name ORDER BY name",
    ];
    for query in text_only {
        let names = column(&mut session, query).await;
        assert_eq!(names, vec!["text".to_string()], "{query}");
    }
    for query in [
        "MATCH (n) WHERE COALESCE(NOT n.v = '10', FALSE) RETURN n.name AS name ORDER BY name",
        "MATCH (n) WHERE NULLIF(n.v, '10') IS NOT NULL RETURN n.name AS name ORDER BY name",
    ] {
        let names = column(&mut session, query).await;
        assert!(names.is_empty(), "{query}: {names:?}");
    }
    assert_eq!(
        column(
            &mut session,
            "MATCH (n) WHERE COALESCE(n.v = '10', n.name = 'nine') RETURN n.name AS name ORDER BY name",
        )
        .await,
        vec!["nine".to_string(), "text".to_string()]
    );
    assert_eq!(
        column(
            &mut session,
            "MATCH (n) WHERE COALESCE(n.v, '10') = '10' RETURN n.name AS name ORDER BY name",
        )
        .await,
        vec!["blank".to_string(), "empty".to_string(), "text".to_string()]
    );
    assert_eq!(
        column(
            &mut session,
            "MATCH (n) WHERE NULLIF(n.v, '10') IS NULL RETURN n.name AS name ORDER BY name",
        )
        .await,
        vec!["blank".to_string(), "empty".to_string(), "text".to_string()]
    );
    assert_eq!(
        column(
            &mut session,
            "MATCH (n) WHERE NULLIF(n.v, 10) IS NULL RETURN n.name AS name ORDER BY name",
        )
        .await,
        vec!["blank".to_string(), "empty".to_string(), "ten".to_string()]
    );
    assert_eq!(
        column(
            &mut session,
            "MATCH (n) WHERE NULLIF(n.v, 10) IS NOT NULL RETURN n.name AS name ORDER BY name",
        )
        .await,
        vec!["nine".to_string()]
    );
    assert_eq!(
        column(
            &mut session,
            "MATCH (n:P) WHERE NULLIF(n.v, 10) IS NULL RETURN n.name AS name ORDER BY name",
        )
        .await,
        vec!["blank".to_string(), "ten".to_string()]
    );

    let text_literal = session
        .query("MATCH (n:D) WHERE NULLIF(n.v, 10) IS NULL RETURN n.name AS name")
        .await
        .unwrap_err()
        .to_string();
    assert!(
        text_literal.contains("incomparable types Utf8 and Int64"),
        "{text_literal}"
    );
    session.run("INSERT (:R {name:'only', v:3})").await.unwrap();
    let int_fallback = session
        .query("MATCH (n:R) WHERE COALESCE(n.v, 'zz') = '10' RETURN n.name AS name")
        .await
        .unwrap_err()
        .to_string();
    assert!(
        int_fallback.contains("incomparable types Int64 and Utf8"),
        "{int_fallback}"
    );

    session
        .run(
            "INSERT (:P {name:'low', score:1}),
                    (:P {name:'high', score:10}),
                    (:M {name:'mid', score:9.5})",
        )
        .await
        .unwrap();
    assert_eq!(
        column(
            &mut session,
            "MATCH (n:P|M) WHERE COALESCE(n.score, 0) > 9 RETURN n.name AS name ORDER BY name",
        )
        .await,
        vec!["high".to_string(), "mid".to_string()]
    );

    let mut widened = social().await;
    assert_eq!(
        column(
            &mut widened,
            "MATCH (n:Person|Metric) WHERE COALESCE(n.score > 1, FALSE) RETURN n.name AS name ORDER BY name",
        )
        .await,
        vec!["Bea".to_string(), "load".to_string()]
    );
}

#[tokio::test]
async fn stored_value_wrappers_keep_filters_aliases_and_optional_nulls() {
    let mut s = Database::new().session();
    s.run("CREATE GRAPH g ANY; SESSION SET GRAPH g; INSERT (:P {name:'ten',v:10}),(:P {name:'blank'}),(:D {name:'text',v:'10'})").await.unwrap();
    for q in [
        "MATCH (n) FILTER COALESCE(n.v, 'zz') = '10' RETURN n.name AS name",
        "MATCH (n) LET ok = COALESCE(n.v = '10', FALSE) FILTER ok RETURN n.name AS name",
    ] {
        assert_eq!(
            strings(&s.query(q).await.unwrap()),
            vec![vec!["text".to_string()]],
            "{q}"
        );
    }
    let r=s.query("MATCH (n) LET r=n FILTER NULLIF(r.v, '10') IS NULL RETURN r.name AS name ORDER BY name").await.unwrap();
    assert_eq!(
        strings(&r),
        vec![vec!["blank".to_string()], vec!["text".to_string()]]
    );
    let r = s
        .query("OPTIONAL MATCH (n) WHERE FALSE LET ok = COALESCE(n.v = '10', TRUE) RETURN ok")
        .await
        .unwrap();
    assert_eq!(strings(&r), vec![vec!["true".to_string()]]);
}

#[tokio::test]
async fn stored_wrappers_handle_absent_properties_and_nested_boolean_nulls() {
    let mut s = Database::new().session();
    s.run("CREATE GRAPH g ANY; SESSION SET GRAPH g; INSERT (:P {name:'number',v:1}),(:Q {name:'missing'})").await.unwrap();
    for predicate in ["NULLIF(n.v, '1') IS NULL", "COALESCE(n.v, '1') = '1'"] {
        for q in [
            format!("MATCH (n) WHERE {predicate} RETURN n.name AS name"),
            format!("MATCH (n) FILTER {predicate} RETURN n.name AS name"),
            format!("MATCH (n) LET ok={predicate} FILTER ok RETURN n.name AS name"),
        ] {
            assert_eq!(
                strings(&s.query(&q).await.unwrap()),
                vec![vec!["missing".to_string()]],
                "{q}"
            );
        }
    }
    s.run("INSERT (:D {name:'text',v:'10'})").await.unwrap();
    let r = s
        .query(
            "MATCH (n) WHERE COALESCE(NULLIF(NULL, NULL), n.v = '10', FALSE) RETURN n.name AS name",
        )
        .await
        .unwrap();
    assert_eq!(strings(&r), vec![vec!["text".to_string()]]);
}

#[tokio::test]
async fn closed_null_predicates_keep_stored_comparisons() {
    let mut s = Database::new().session();
    s.run("CREATE GRAPH g ANY; SESSION SET GRAPH g; INSERT (:P {name:'number',v:10}),(:D {name:'text',v:'10'})").await.unwrap();
    for predicate in [
        "n.v = '10' OR 1 IS NULL",
        "n.v = '10' OR NULL IS NOT NULL",
        "n.v = '10' AND 1 IS NOT NULL",
    ] {
        for q in [
            format!("MATCH (n) WHERE {predicate} RETURN n.name AS name"),
            format!("MATCH (n) FILTER {predicate} RETURN n.name AS name"),
            format!("MATCH (n) LET keep = {predicate} FILTER keep RETURN n.name AS name"),
        ] {
            assert_eq!(
                strings(&s.query(&q).await.unwrap()),
                vec![vec!["text".to_string()]],
                "{q}"
            );
        }
    }
}

#[tokio::test]
async fn stored_row_comparisons_keep_optional_misses_and_reference_types() {
    let mut s = social().await;
    let r = s.query("LET k = 1, keep = TRUE OPTIONAL MATCH (n:Person|Document {name:'absent'}) FILTER n.id = k OR keep RETURN 1 AS v").await.unwrap();
    assert_eq!(r.row_count(), 1);
    let r = s
        .query("MATCH (n:Person|Document) LET r=n, k='doc' FILTER r.id = k RETURN r.title AS title")
        .await
        .unwrap();
    assert_eq!(strings(&r), vec![vec!["Spec".to_string()]]);
}

#[tokio::test]
async fn stored_row_comparisons_support_wrappers_and_repeated_predicates() {
    let mut s = social().await;
    for query in [
        "MATCH (n:Person|Document) LET k='doc' FILTER n.id=k FILTER n.id=k RETURN n.title AS title",
        "LET k='doc' MATCH (n:Person|Document WHERE n.id=k) WHERE n.id=k RETURN n.title AS title",
        "LET k='doc' MATCH (n:Person|Document) WHERE (n.id=k) IS TRUE RETURN n.title AS title",
        "MATCH (n:Person|Document) LET k='doc' FILTER COALESCE(n.id=k,FALSE) RETURN n.title AS title",
    ] {
        let r = s.query(query).await.unwrap();
        assert_eq!(strings(&r), vec![vec!["Spec".to_string()]], "{query}");
    }
    for query in [
        "MATCH (n:Person|Document) LET k='1' FILTER (n.id=k) IS TRUE RETURN n.name AS name",
        "LET k='1' MATCH (n:Person|Document) WHERE (n.id=k) IS TRUE RETURN n.name AS name",
    ] {
        assert_eq!(s.query(query).await.unwrap().row_count(), 0, "{query}");
    }
}

#[tokio::test]
async fn closed_conjuncts_apply_across_pattern_alternatives() {
    let mut s = social().await;
    let result = s
        .query("MATCH (n:Person) | (m:Metric) WHERE n.id=1 AND FALSE RETURN 1 AS x")
        .await
        .unwrap();
    assert_eq!(result.row_count(), 0);
}

#[tokio::test]
async fn where_constant_comparisons_keep_stored_types() {
    let mut session = social().await;
    let either = session
        .query(
            "MATCH (n:Person|Document) WHERE n.id = 1 OR 1 = 1
             RETURN n.name AS name ORDER BY name",
        )
        .await
        .unwrap();
    assert_eq!(
        strings(&either),
        vec![
            vec!["Ada".to_string()],
            vec!["Bea".to_string()],
            vec!["NULL".to_string()]
        ]
    );

    let only_ada = session
        .query("MATCH (n:Person|Document) WHERE n.id = 1 OR 1 = 0 RETURN n.name AS name")
        .await
        .unwrap();
    assert_eq!(strings(&only_ada), vec![vec!["Ada".to_string()]]);

    let still_ada = session
        .query("MATCH (n:Person|Document) WHERE n.id = 1 AND 1 = 1 RETURN n.name AS name")
        .await
        .unwrap();
    assert_eq!(strings(&still_ada), vec![vec!["Ada".to_string()]]);

    let excluded = session
        .query("MATCH (n:Person|Document) WHERE n.id = 1 AND 1 = 0 RETURN n.name AS name")
        .await
        .unwrap();
    assert_eq!(excluded.row_count(), 0);

    let exclusive = session
        .query(
            "MATCH (n:Person|Document) WHERE n.id = 1 XOR 1 = 1
             RETURN n.name AS name ORDER BY name",
        )
        .await
        .unwrap();
    assert_eq!(strings(&exclusive), vec![vec!["Bea".to_string()]]);

    let negated = session
        .query("MATCH (n:Person|Document) WHERE NOT (1 = 0) AND n.id = 1 RETURN n.name AS name")
        .await
        .unwrap();
    assert_eq!(strings(&negated), vec![vec!["Ada".to_string()]]);

    let inline = session
        .query(
            "MATCH (n:Person|Document WHERE n.id = 1 OR 1 = 1)
             RETURN n.name AS name ORDER BY name",
        )
        .await
        .unwrap();
    assert_eq!(
        strings(&inline),
        vec![
            vec!["Ada".to_string()],
            vec!["Bea".to_string()],
            vec!["NULL".to_string()]
        ]
    );

    let document = session
        .query("MATCH (n:Person|Document) WHERE n.id = 'doc' OR 2 > 3 RETURN n.title AS title")
        .await
        .unwrap();
    assert_eq!(strings(&document), vec![vec!["Spec".to_string()]]);

    let widened = session
        .query(
            "MATCH (n:Person|Document) WHERE n.id = 1 OR 2 > 0
             RETURN n.name AS name ORDER BY name",
        )
        .await
        .unwrap();
    assert_eq!(
        strings(&widened),
        vec![
            vec!["Ada".to_string()],
            vec!["Bea".to_string()],
            vec!["NULL".to_string()]
        ]
    );

    let joined = session
        .query(
            "MATCH (n:Person|Document), (m:Metric)
             WHERE n.id = 1 AND m.name = 'load' AND 1 = 1
             RETURN n.name AS name, m.score AS score",
        )
        .await
        .unwrap();
    assert_eq!(
        strings(&joined),
        vec![vec!["Ada".to_string(), "1.5".to_string()]]
    );

    let grouped = session
        .query(
            "MATCH (n:Person|Document), (m:Metric)
             WHERE n.id = 1 AND 1 = 1 AND m.name = 'load'
             RETURN n.name AS name, m.score AS score",
        )
        .await
        .unwrap();
    assert_eq!(
        strings(&grouped),
        vec![vec!["Ada".to_string(), "1.5".to_string()]]
    );

    session
        .execute("SESSION SET VALUE $one INTEGER = 1")
        .unwrap();
    let parameter = session
        .query(
            "MATCH (n:Person|Document) WHERE n.id = 1 OR $one = 1
             RETURN n.name AS name ORDER BY name",
        )
        .await
        .unwrap();
    assert_eq!(
        strings(&parameter),
        vec![
            vec!["Ada".to_string()],
            vec!["Bea".to_string()],
            vec!["NULL".to_string()]
        ]
    );

    let error = session
        .query("MATCH (d:Document) WHERE d.id = 1 OR 1 = 1 RETURN d.title AS title")
        .await
        .unwrap_err();
    match error {
        Error::InvalidQuery(message) => assert!(message.contains("incomparable"), "{message}"),
        other => panic!("expected invalid query, got {other}"),
    }
}

#[tokio::test]
async fn where_null_tests_keep_stored_types() {
    let mut session = social().await;
    session
        .run("INSERT (:Person {name: 'Cara'})")
        .await
        .unwrap();

    let either = session
        .query(
            "MATCH (n:Person|Document) WHERE n.id = 1 OR n.id IS NULL
             RETURN n.name AS name ORDER BY name",
        )
        .await
        .unwrap();
    assert_eq!(
        strings(&either),
        vec![vec!["Ada".to_string()], vec!["Cara".to_string()]]
    );

    let present = session
        .query(
            "MATCH (n:Person|Document) WHERE n.id IS NOT NULL AND n.id = 1
             RETURN n.name AS name",
        )
        .await
        .unwrap();
    assert_eq!(strings(&present), vec![vec!["Ada".to_string()]]);

    let negated = session
        .query(
            "MATCH (n:Person|Document) WHERE NOT n.id IS NULL AND n.id = 1
             RETURN n.name AS name",
        )
        .await
        .unwrap();
    assert_eq!(strings(&negated), vec![vec!["Ada".to_string()]]);

    let others = session
        .query(
            "MATCH (n:Person|Document) WHERE n.id = 1 OR NOT n.id IS NULL
             RETURN n.name AS name ORDER BY name",
        )
        .await
        .unwrap();
    assert_eq!(
        strings(&others),
        vec![
            vec!["Ada".to_string()],
            vec!["Bea".to_string()],
            vec!["NULL".to_string()]
        ]
    );

    let inline = session
        .query(
            "MATCH (n:Person|Document WHERE n.id = 1 OR n.id IS NULL)
             RETURN n.name AS name ORDER BY name",
        )
        .await
        .unwrap();
    assert_eq!(
        strings(&inline),
        vec![vec!["Ada".to_string()], vec!["Cara".to_string()]]
    );

    let element = session
        .query(
            "MATCH (n:Person|Document) WHERE n IS NULL OR n.id = 1
             RETURN n.name AS name",
        )
        .await
        .unwrap();
    assert_eq!(strings(&element), vec![vec!["Ada".to_string()]]);

    let element_present = session
        .query(
            "MATCH (n:Person|Document) WHERE NOT n IS NULL AND n.id = 1
             RETURN n.name AS name",
        )
        .await
        .unwrap();
    assert_eq!(strings(&element_present), vec![vec!["Ada".to_string()]]);

    let missing = session
        .query(
            "MATCH (n:Person|Document) WHERE n.title IS NULL OR n.id = 1
             RETURN n.name AS name ORDER BY name",
        )
        .await
        .unwrap();
    assert_eq!(
        strings(&missing),
        vec![
            vec!["Ada".to_string()],
            vec!["Bea".to_string()],
            vec!["Cara".to_string()]
        ]
    );

    let titled = session
        .query(
            "MATCH (n:Person|Document) WHERE n.title IS NOT NULL AND n.id = 'doc'
             RETURN n.title AS title",
        )
        .await
        .unwrap();
    assert_eq!(strings(&titled), vec![vec!["Spec".to_string()]]);

    let only_null = session
        .query("MATCH (n:Person|Document) WHERE n.id IS NULL RETURN n.name AS name")
        .await
        .unwrap();
    assert_eq!(strings(&only_null), vec![vec!["Cara".to_string()]]);

    let empty = session
        .query("MATCH (n:Person|Document) WHERE n.id = 1 AND n.id IS NULL RETURN n.name AS name")
        .await
        .unwrap();
    assert_eq!(empty.row_count(), 0);

    let joined = session
        .query(
            "MATCH (n:Person|Document), (m:Metric)
             WHERE n.id IS NULL AND m.name = 'load'
             RETURN n.name AS name, m.score AS score",
        )
        .await
        .unwrap();
    assert_eq!(
        strings(&joined),
        vec![vec!["Cara".to_string(), "1.5".to_string()]]
    );

    let error = session
        .query("MATCH (d:Document) WHERE d.id = 1 OR d.id IS NULL RETURN d.title AS title")
        .await
        .unwrap_err();
    match error {
        Error::InvalidQuery(message) => assert!(message.contains("incomparable"), "{message}"),
        other => panic!("expected invalid query, got {other}"),
    }
}

#[tokio::test]
async fn null_pushdown_keeps_internal_fields_hidden_and_conditional_rows_checked() {
    let mut s = social().await;
    assert_eq!(
        s.query("MATCH(n) WHERE n.__gf_id IS NULL RETURN n")
            .await
            .unwrap()
            .row_count(),
        4
    );
    s.run("MATCH(a:Person {name:'Ada'}),(b:Person {name:'Bea'}) INSERT(a)-[:E]->(b)")
        .await
        .unwrap();
    assert_eq!(
        s.query("MATCH(a)(-[e:E]->(b))? WHERE e IS NOT NULL RETURN e")
            .await
            .unwrap()
            .row_count(),
        1
    );
}

#[tokio::test]
async fn inline_bound_comparison_keeps_a_null_test_conjunct() {
    let mut s = social().await;
    let r=s.query("LET k=1 MATCH(n:Person|Document WHERE n.id=k AND n.name IS NOT NULL) RETURN n.name AS name").await.unwrap();
    assert_eq!(strings(&r), vec![vec!["Ada".to_string()]]);
}

#[tokio::test]
async fn where_property_exists_keeps_stored_types() {
    let mut session = social().await;
    session
        .run("INSERT (:Person {name: 'Cara'})")
        .await
        .unwrap();

    let present = session
        .query(
            "MATCH (n:Person|Document) WHERE PROPERTY_EXISTS(n, id) AND n.id = 1
             RETURN n.name AS name",
        )
        .await
        .unwrap();
    assert_eq!(strings(&present), vec![vec!["Ada".to_string()]]);

    let titled = session
        .query(
            "MATCH (n:Person|Document) WHERE n.id = 1 OR PROPERTY_EXISTS(n, title)
             RETURN n.name AS name ORDER BY name",
        )
        .await
        .unwrap();
    assert_eq!(
        strings(&titled),
        vec![vec!["Ada".to_string()], vec!["NULL".to_string()]]
    );

    let missing_id = session
        .query(
            "MATCH (n:Person|Document) WHERE NOT PROPERTY_EXISTS(n, id) OR n.id = 1
             RETURN n.name AS name ORDER BY name",
        )
        .await
        .unwrap();
    assert_eq!(
        strings(&missing_id),
        vec![vec!["Ada".to_string()], vec!["Cara".to_string()]]
    );

    let document = session
        .query(
            "MATCH (n:Person|Document)
             WHERE PROPERTY_EXISTS(n, title) AND n.id = 'doc'
             RETURN n.title AS title",
        )
        .await
        .unwrap();
    assert_eq!(strings(&document), vec![vec!["Spec".to_string()]]);

    let no_title = session
        .query(
            "MATCH (n:Person|Document) WHERE PROPERTY_EXISTS(n, title) AND n.id = 1
             RETURN n.name AS name",
        )
        .await
        .unwrap();
    assert_eq!(no_title.row_count(), 0);

    let inline = session
        .query(
            "MATCH (n:Person|Document WHERE n.id = 1 OR PROPERTY_EXISTS(n, title))
             RETURN n.name AS name ORDER BY name",
        )
        .await
        .unwrap();
    assert_eq!(
        strings(&inline),
        vec![vec!["Ada".to_string()], vec!["NULL".to_string()]]
    );

    let string_id = session
        .query(
            "MATCH (n:Person|Document) WHERE PROPERTY_EXISTS(n, id) OR n.id = 'doc'
             RETURN n.name AS name ORDER BY name",
        )
        .await
        .unwrap();
    assert_eq!(
        strings(&string_id),
        vec![
            vec!["Ada".to_string()],
            vec!["Bea".to_string()],
            vec!["NULL".to_string()]
        ]
    );

    let joined = session
        .query(
            "MATCH (n:Person|Document), (m:Metric)
             WHERE n.id = 1 AND PROPERTY_EXISTS(m, name)
             RETURN n.name AS name, m.score AS score",
        )
        .await
        .unwrap();
    assert_eq!(
        strings(&joined),
        vec![vec!["Ada".to_string(), "1.5".to_string()]]
    );

    let absent = session
        .query(
            "MATCH (n:Person|Document) WHERE NOT PROPERTY_EXISTS(n, id)
             RETURN n.name AS name",
        )
        .await
        .unwrap();
    assert_eq!(strings(&absent), vec![vec!["Cara".to_string()]]);

    let error = session
        .query(
            "MATCH (d:Document) WHERE d.id = 1 OR PROPERTY_EXISTS(d, title)
             RETURN d.title AS title",
        )
        .await
        .unwrap_err();
    match error {
        Error::InvalidQuery(message) => assert!(message.contains("incomparable"), "{message}"),
        other => panic!("expected invalid query, got {other}"),
    }
}

#[tokio::test]
async fn stored_exists_keeps_internal_fields_hidden_and_optional_misses_unknown() {
    let mut s = social().await;
    assert_eq!(
        s.query("MATCH(n) WHERE PROPERTY_EXISTS(n,__gf_id) RETURN n")
            .await
            .unwrap()
            .row_count(),
        0
    );
    let r=s.query("LET keep=TRUE OPTIONAL MATCH(n:Person|Document {name:'absent'}) FILTER PROPERTY_EXISTS(n,id) OR keep RETURN 1 AS v").await.unwrap();
    assert_eq!(r.row_count(), 1);
    let r=s.query("MATCH(n:Person|Document) LET r=n,k=1 FILTER r.id=k AND PROPERTY_EXISTS(r,id) RETURN r.name AS name").await.unwrap();
    assert_eq!(strings(&r), vec![vec!["Ada".to_string()]]);
}

#[tokio::test]
async fn scalar_null_tests_and_other_element_metadata_stay_on_the_working_row() {
    let mut s = social().await;
    for query in [
        "LET k=1 MATCH(n:Person|Document) WHERE n.id=1 OR k IS NULL RETURN n.name AS name",
        "LET r=NULL MATCH(n:Person|Document) WHERE n.id=1 OR PROPERTY_EXISTS(r,id) RETURN n.name AS name",
        "MATCH(n:Person|Document) LET k=1 FILTER n.id=1 OR k IS NULL RETURN n.name AS name",
        "LET k=1 MATCH(n:Person|Document WHERE n.id=1 OR k IS NULL) RETURN n.name AS name",
        "MATCH(n:Person|Document),(m:Metric) FILTER n.id=1 OR PROPERTY_EXISTS(m,missing) RETURN n.name AS name",
    ] {
        let r=s.query(query).await.unwrap();
        assert_eq!(strings(&r),vec![vec!["Ada".to_string()]],"{query}");
    }
}

#[tokio::test]
async fn where_label_tests_keep_stored_types() {
    let mut session = social().await;

    let person = session
        .query(
            "MATCH (n:Person|Document) WHERE n IS LABELED Person AND n.id = 1
             RETURN n.name AS name",
        )
        .await
        .unwrap();
    assert_eq!(strings(&person), vec![vec!["Ada".to_string()]]);

    let colon = session
        .query("MATCH (n:Person|Document) WHERE n:Person AND n.id = 1 RETURN n.name AS name")
        .await
        .unwrap();
    assert_eq!(strings(&colon), vec![vec!["Ada".to_string()]]);

    let document = session
        .query(
            "MATCH (n:Person|Document) WHERE n.id = 1 OR n IS LABELED Document
             RETURN n.name AS name ORDER BY name",
        )
        .await
        .unwrap();
    assert_eq!(
        strings(&document),
        vec![vec!["Ada".to_string()], vec!["NULL".to_string()]]
    );

    let not_person = session
        .query(
            "MATCH (n:Person|Document) WHERE n IS NOT LABELED Person OR n.id = 1
             RETURN n.name AS name ORDER BY name",
        )
        .await
        .unwrap();
    assert_eq!(
        strings(&not_person),
        vec![vec!["Ada".to_string()], vec!["NULL".to_string()]]
    );

    let keep_people = session
        .query(
            "MATCH (n:Person|Document) WHERE NOT n IS LABELED Document AND n.id = 1
             RETURN n.name AS name",
        )
        .await
        .unwrap();
    assert_eq!(strings(&keep_people), vec![vec!["Ada".to_string()]]);

    let inline = session
        .query(
            "MATCH (n:Person|Document WHERE n.id = 1 OR n IS LABELED Document)
             RETURN n.name AS name ORDER BY name",
        )
        .await
        .unwrap();
    assert_eq!(
        strings(&inline),
        vec![vec!["Ada".to_string()], vec!["NULL".to_string()]]
    );

    let excluded = session
        .query(
            "MATCH (n:Person|Document) WHERE n IS LABELED !Person AND n.id = 'doc'
             RETURN n.title AS title",
        )
        .await
        .unwrap();
    assert_eq!(strings(&excluded), vec![vec!["Spec".to_string()]]);

    let either_label = session
        .query(
            "MATCH (n:Person|Document) WHERE n IS LABELED Person|Document AND n.id = 'doc'
             RETURN n.title AS title",
        )
        .await
        .unwrap();
    assert_eq!(strings(&either_label), vec![vec!["Spec".to_string()]]);

    let both_labels = session
        .query(
            "MATCH (n:Person|Document) WHERE n IS LABELED Person&Document OR n.id = 1
             RETURN n.name AS name",
        )
        .await
        .unwrap();
    assert_eq!(strings(&both_labels), vec![vec!["Ada".to_string()]]);

    let wildcard = session
        .query(
            "MATCH (n:Person|Document) WHERE n IS LABELED % AND n.id = 1
             RETURN n.name AS name",
        )
        .await
        .unwrap();
    assert_eq!(strings(&wildcard), vec![vec!["Ada".to_string()]]);

    let people = session
        .query(
            "MATCH (n:Person|Document) WHERE n.id = 'doc' OR n IS LABELED Person
             RETURN n.name AS name ORDER BY name",
        )
        .await
        .unwrap();
    assert_eq!(
        strings(&people),
        vec![
            vec!["Ada".to_string()],
            vec!["Bea".to_string()],
            vec!["NULL".to_string()]
        ]
    );

    let joined = session
        .query(
            "MATCH (n:Person|Document), (m:Metric)
             WHERE n.id = 1 AND m IS LABELED Metric
             RETURN n.name AS name, m.score AS score",
        )
        .await
        .unwrap();
    assert_eq!(
        strings(&joined),
        vec![vec!["Ada".to_string(), "1.5".to_string()]]
    );

    let error = session
        .query(
            "MATCH (d:Document) WHERE d.id = 1 OR d IS LABELED Document
             RETURN d.title AS title",
        )
        .await
        .unwrap_err();
    match error {
        Error::InvalidQuery(message) => assert!(message.contains("incomparable"), "{message}"),
        other => panic!("expected invalid query, got {other}"),
    }
}

#[tokio::test]
async fn stored_labels_keep_absent_alternatives_and_scalar_nulls_correct() {
    let mut s = social().await;
    assert_eq!(
        s.query("MATCH(n:Person)|(m:Document) WHERE n IS LABELED Person RETURN 1 AS v")
            .await
            .unwrap()
            .row_count(),
        2
    );
    let r=s.query("MATCH(n:Person|Document),(m:Metric) FILTER n.id=1 OR m IS LABELED Missing RETURN n.name AS name").await.unwrap();
    assert_eq!(strings(&r), vec![vec!["Ada".to_string()]]);
    let r=s.query("LET r=NULL MATCH(n:Person|Document) WHERE n.id=1 OR r IS LABELED Person RETURN n.name AS name").await.unwrap();
    assert_eq!(strings(&r), vec![vec!["Ada".to_string()]]);
}

#[tokio::test]
async fn where_direction_tests_keep_stored_types() {
    let mut session = social().await;
    session
        .run(
            "INSERT (a:Person {name: 'Cy', id: 3})-[:Knows {id: 1}]->(a);
             INSERT (b:Person {name: 'Knot', id: 4})-[:Notes {id: 2}]-(b);
             INSERT (c:Document {title: 'Paper', id: 'p'})-[:Cites {id: 'doc'}]->(c)",
        )
        .await
        .unwrap();

    let directed = session
        .query(
            "MATCH ()-[e:Knows|Notes|Cites]-()
             WHERE e IS DIRECTED AND e.id = 1
             RETURN e.id AS id",
        )
        .await
        .unwrap();
    assert_eq!(directed.schema.field(0).data_type(), &DataType::Utf8);
    assert_eq!(strings(&directed), vec![vec!["1".to_string()]]);

    let undirected = session
        .query(
            "MATCH ()-[e:Knows|Notes|Cites]-()
             WHERE e IS NOT DIRECTED AND e.id = 2
             RETURN e.id AS id",
        )
        .await
        .unwrap();
    assert_eq!(strings(&undirected), vec![vec!["2".to_string()]]);

    let either = session
        .query(
            "MATCH ()-[e:Knows|Notes|Cites]-()
             WHERE e.id = 'doc' OR e IS NOT DIRECTED
             RETURN e.id AS id ORDER BY id",
        )
        .await
        .unwrap();
    assert_eq!(
        strings(&either),
        vec![vec!["2".to_string()], vec!["doc".to_string()]]
    );

    let all_directed = session
        .query(
            "MATCH ()-[e:Knows|Notes|Cites]-()
             WHERE e IS DIRECTED OR e.id = 2
             RETURN e.id AS id ORDER BY id",
        )
        .await
        .unwrap();
    assert_eq!(
        strings(&all_directed),
        vec![
            vec!["1".to_string()],
            vec!["2".to_string()],
            vec!["doc".to_string()]
        ]
    );

    let excluded = session
        .query(
            "MATCH ()-[e:Knows|Notes|Cites]-()
             WHERE NOT e IS DIRECTED AND e.id = 1
             RETURN e.id AS id",
        )
        .await
        .unwrap();
    assert_eq!(excluded.row_count(), 0);

    let inline = session
        .query(
            "MATCH ()-[e:Knows|Notes|Cites WHERE e IS DIRECTED AND e.id = 1]-()
             RETURN e.id AS id",
        )
        .await
        .unwrap();
    assert_eq!(strings(&inline), vec![vec!["1".to_string()]]);

    let joined = session
        .query(
            "MATCH (n:Person|Document)-[e:Knows|Notes|Cites]-()
             WHERE n.id = 3 AND e IS DIRECTED
             RETURN n.name AS name, e.id AS id",
        )
        .await
        .unwrap();
    assert_eq!(
        strings(&joined),
        vec![vec!["Cy".to_string(), "1".to_string()]]
    );

    let node = session
        .query("MATCH (n:Person) WHERE n IS DIRECTED RETURN n.name AS name")
        .await
        .unwrap_err();
    match node {
        Error::InvalidQuery(message) => assert!(message.contains("edge"), "{message}"),
        other => panic!("expected invalid query, got {other}"),
    }

    let error = session
        .query("MATCH ()-[e:Cites]-() WHERE e.id = 1 OR e IS DIRECTED RETURN e.id AS id")
        .await
        .unwrap_err();
    match error {
        Error::InvalidQuery(message) => assert!(message.contains("incomparable"), "{message}"),
        other => panic!("expected invalid query, got {other}"),
    }
}

#[tokio::test]
async fn direction_kind_is_checked_before_an_empty_fragment_scan() {
    let mut s = Database::new().session();
    s.execute("CREATE GRAPH g ANY; SESSION SET GRAPH g")
        .unwrap();
    for q in [
        "MATCH(n:Missing) WHERE n IS DIRECTED RETURN n",
        "MATCH(n:Missing WHERE n IS DIRECTED) RETURN n",
    ] {
        let error = s.query(q).await.unwrap_err();
        assert!(
            error.to_string().contains("requires an edge"),
            "{q}: {error}"
        );
    }
    let mut s = social().await;
    let r=s.query("LET r=NULL MATCH(n:Person|Document) WHERE n.id=1 OR r IS DIRECTED RETURN n.name AS name").await.unwrap();
    assert_eq!(strings(&r), vec![vec!["Ada".to_string()]]);
}

#[tokio::test]
async fn where_identity_tests_keep_stored_types() {
    let mut session = social().await;

    let same = session
        .query(
            "MATCH (n:Person|Document) WHERE SAME(n, n) AND n.id = 1
             RETURN n.name AS name",
        )
        .await
        .unwrap();
    assert_eq!(strings(&same), vec![vec!["Ada".to_string()]]);

    let repeated = session
        .query(
            "MATCH (n:Person|Document) WHERE SAME(n, n, n) AND n.id = 1
             RETURN n.name AS name",
        )
        .await
        .unwrap();
    assert_eq!(strings(&repeated), vec![vec!["Ada".to_string()]]);

    let different = session
        .query(
            "MATCH (n:Person|Document) WHERE n.id = 1 OR ALL_DIFFERENT(n, n)
             RETURN n.name AS name",
        )
        .await
        .unwrap();
    assert_eq!(strings(&different), vec![vec!["Ada".to_string()]]);

    let everyone = session
        .query(
            "MATCH (n:Person|Document) WHERE n.id = 1 OR SAME(n, n)
             RETURN n.name AS name ORDER BY name",
        )
        .await
        .unwrap();
    assert_eq!(
        strings(&everyone),
        vec![
            vec!["Ada".to_string()],
            vec!["Bea".to_string()],
            vec!["NULL".to_string()]
        ]
    );

    let negated = session
        .query(
            "MATCH (n:Person|Document) WHERE NOT SAME(n, n) OR n.id = 1
             RETURN n.name AS name",
        )
        .await
        .unwrap();
    assert_eq!(strings(&negated), vec![vec!["Ada".to_string()]]);

    let document = session
        .query(
            "MATCH (n:Person|Document) WHERE n.id = 'doc' OR ALL_DIFFERENT(n, n, n)
             RETURN n.title AS title",
        )
        .await
        .unwrap();
    assert_eq!(strings(&document), vec![vec!["Spec".to_string()]]);

    let inline = session
        .query(
            "MATCH (n:Person|Document WHERE SAME(n, n) AND n.id = 1)
             RETURN n.name AS name",
        )
        .await
        .unwrap();
    assert_eq!(strings(&inline), vec![vec!["Ada".to_string()]]);

    let error = session
        .query("MATCH (d:Document) WHERE d.id = 1 OR SAME(d, d) RETURN d.title AS title")
        .await
        .unwrap_err();
    match error {
        Error::InvalidQuery(message) => assert!(message.contains("incomparable"), "{message}"),
        other => panic!("expected invalid query, got {other}"),
    }
}

#[tokio::test]
async fn identity_metadata_keeps_scalar_nulls_and_conditional_types() {
    let mut s = social().await;
    let r = s
        .query(
            "LET r=NULL MATCH(n:Person|Document) WHERE n.id=1 OR SAME(r,r) RETURN n.name AS name",
        )
        .await
        .unwrap();
    assert_eq!(strings(&r), vec![vec!["Ada".to_string()]]);
    assert_eq!(
        s.query("MATCH(a)(-[e]->(b))?(c) WHERE ALL_DIFFERENT(b,b) RETURN c")
            .await
            .unwrap()
            .row_count(),
        0
    );
    assert!(s
        .query("MATCH(a)((x)-[:E]->(y)){1}(b) WHERE SAME(x,x) RETURN b")
        .await
        .is_err());
}

#[tokio::test]
async fn where_element_id_tests_keep_stored_types() {
    let mut session = social().await;
    let ada = session
        .query("MATCH (n:Person {name: 'Ada'}) RETURN ELEMENT_ID(n) AS id")
        .await
        .unwrap();
    let ada_id = strings(&ada)[0][0].clone();

    let present = session
        .query(
            "MATCH (n:Person|Document) WHERE ELEMENT_ID(n) IS NOT NULL AND n.id = 1
             RETURN n.name AS name",
        )
        .await
        .unwrap();
    assert_eq!(strings(&present), vec![vec!["Ada".to_string()]]);

    let negated = session
        .query(
            "MATCH (n:Person|Document) WHERE NOT ELEMENT_ID(n) IS NULL AND n.id = 1
             RETURN n.name AS name",
        )
        .await
        .unwrap();
    assert_eq!(strings(&negated), vec![vec!["Ada".to_string()]]);

    let everyone = session
        .query(
            "MATCH (n:Person|Document) WHERE n.id = 1 OR ELEMENT_ID(n) = ELEMENT_ID(n)
             RETURN n.name AS name ORDER BY name",
        )
        .await
        .unwrap();
    assert_eq!(
        strings(&everyone),
        vec![
            vec!["Ada".to_string()],
            vec!["Bea".to_string()],
            vec!["NULL".to_string()]
        ]
    );

    let only_ada = session
        .query(
            "MATCH (n:Person|Document) WHERE ELEMENT_ID(n) <> ELEMENT_ID(n) OR n.id = 1
             RETURN n.name AS name",
        )
        .await
        .unwrap();
    assert_eq!(strings(&only_ada), vec![vec!["Ada".to_string()]]);

    let by_id = session
        .query(&format!(
            "MATCH (n:Person|Document) WHERE n.id = 9 OR ELEMENT_ID(n) = '{ada_id}'
             RETURN n.name AS name"
        ))
        .await
        .unwrap();
    assert_eq!(strings(&by_id), vec![vec!["Ada".to_string()]]);

    let missing = session
        .query(
            "MATCH (n:Person|Document) WHERE n.id = 1 OR ELEMENT_ID(n) = 'missing'
             RETURN n.name AS name",
        )
        .await
        .unwrap();
    assert_eq!(strings(&missing), vec![vec!["Ada".to_string()]]);

    let inline = session
        .query(&format!(
            "MATCH (n:Person|Document WHERE ELEMENT_ID(n) = '{ada_id}' OR n.id = 2)
             RETURN n.name AS name ORDER BY name"
        ))
        .await
        .unwrap();
    assert_eq!(
        strings(&inline),
        vec![vec!["Ada".to_string()], vec!["Bea".to_string()]]
    );

    let error = session
        .query(
            "MATCH (d:Document) WHERE d.id = 1 OR ELEMENT_ID(d) IS NOT NULL
             RETURN d.title AS title",
        )
        .await
        .unwrap_err();
    match error {
        Error::InvalidQuery(message) => assert!(message.contains("incomparable"), "{message}"),
        other => panic!("expected invalid query, got {other}"),
    }

    let typed = session
        .query(
            "MATCH (n:Person|Document) WHERE ELEMENT_ID(n) = 1 AND n.id = 1
             RETURN n.name AS name",
        )
        .await
        .unwrap_err();
    match typed {
        Error::InvalidQuery(message) => assert!(message.contains("incomparable"), "{message}"),
        other => panic!("expected invalid query, got {other}"),
    }
}

#[tokio::test]
async fn stored_element_identity_keeps_uri_null_and_branch_semantics() {
    let mut s = social().await;
    let identity = strings(
        &s.query("MATCH(p:Person {id:1}) RETURN ELEMENT_ID(p) AS id")
            .await
            .unwrap(),
    )[0][0]
        .clone();
    let q=format!("MATCH(n:Person|Document) WHERE n.id='missing' OR ELEMENT_ID(n)='{identity}' RETURN n.name AS name");
    assert_eq!(
        strings(&s.query(&q).await.unwrap()),
        vec![vec!["Ada".to_string()]]
    );
    assert_eq!(
        s.query("MATCH(n:Person)|(m:Document) WHERE ELEMENT_ID(n) IS NOT NULL RETURN n")
            .await
            .unwrap()
            .row_count(),
        2
    );
    assert_eq!(s.query("OPTIONAL MATCH(n:Person|Document {name:'absent'}) FILTER n.id=1 OR ELEMENT_ID(n) IS NULL RETURN 1 AS v").await.unwrap().row_count(),1);
}

#[tokio::test]
async fn cross_element_or_compares_stored_types() {
    let mut session = Database::new().session();
    session
        .execute("CREATE GRAPH g ANY GRAPH; SESSION SET GRAPH g")
        .unwrap();
    session
        .run(
            "INSERT (:A {name: 'A', id: 1}),
                    (:B {name: 'B', id: 'x'}),
                    (:C {name: 'C', id: 2}),
                    (:D {name: 'D', id: '1'})",
        )
        .await
        .unwrap();
    let either = "MATCH (n), (m) WHERE n.id = 1 OR m.id = 'x'
         RETURN n.name AS n, m.name AS m ORDER BY n, m";
    let pairs = vec![
        vec!["A".to_string(), "A".to_string()],
        vec!["A".to_string(), "B".to_string()],
        vec!["A".to_string(), "C".to_string()],
        vec!["A".to_string(), "D".to_string()],
        vec!["B".to_string(), "B".to_string()],
        vec!["C".to_string(), "B".to_string()],
        vec!["D".to_string(), "B".to_string()],
    ];
    assert_eq!(pair_rows(&mut session, either).await, pairs);
    assert_eq!(
        pair_rows(
            &mut session,
            "MATCH (n), (m) WHERE 1 = n.id OR 'x' = m.id
             RETURN n.name AS n, m.name AS m ORDER BY n, m"
        )
        .await,
        pairs
    );
    assert_eq!(
        pair_rows(
            &mut session,
            "MATCH (n), (m) WHERE n.id = 1 OR m.id = 'x' OR FALSE
             RETURN n.name AS n, m.name AS m ORDER BY n, m"
        )
        .await,
        pairs
    );
    session.execute("SESSION SET VALUE $k INTEGER = 1").unwrap();
    assert_eq!(
        pair_rows(
            &mut session,
            "MATCH (n), (m) WHERE n.id = $k OR m.id = 'x'
             RETURN n.name AS n, m.name AS m ORDER BY n, m"
        )
        .await,
        pairs
    );

    assert_eq!(
        pair_rows(
            &mut session,
            "MATCH (n), (m) WHERE n.id = 1 XOR m.id = 'x'
             RETURN n.name AS n, m.name AS m ORDER BY n, m"
        )
        .await,
        vec![
            vec!["A".to_string(), "D".to_string()],
            vec!["C".to_string(), "B".to_string()],
        ]
    );
    assert_eq!(
        pair_rows(
            &mut session,
            "MATCH (n), (m) WHERE NOT n.id = 1 OR m.id = 'x'
             RETURN n.name AS n, m.name AS m ORDER BY n, m"
        )
        .await,
        vec![
            vec!["A".to_string(), "B".to_string()],
            vec!["B".to_string(), "B".to_string()],
            vec!["C".to_string(), "A".to_string()],
            vec!["C".to_string(), "B".to_string()],
            vec!["C".to_string(), "C".to_string()],
            vec!["C".to_string(), "D".to_string()],
            vec!["D".to_string(), "B".to_string()],
        ]
    );
    assert_eq!(
        pair_rows(
            &mut session,
            "MATCH (n:A|C), (m:A|C) WHERE NOT (n.id = 1 OR m.id = 2)
             RETURN n.name AS n, m.name AS m"
        )
        .await,
        vec![vec!["C".to_string(), "A".to_string()]]
    );
    assert_eq!(
        pair_rows(
            &mut session,
            "MATCH (n), (m) WHERE (n.id = 1 AND m.id = 2) OR m.id = 'x'
             RETURN n.name AS n, m.name AS m ORDER BY n, m"
        )
        .await,
        vec![
            vec!["A".to_string(), "B".to_string()],
            vec!["A".to_string(), "C".to_string()],
            vec!["B".to_string(), "B".to_string()],
            vec!["C".to_string(), "B".to_string()],
            vec!["D".to_string(), "B".to_string()],
        ]
    );
    // Integer 1 does not match the string '1', and 2 is not greater than 9.
    assert_eq!(
        pair_rows(
            &mut session,
            "MATCH (n), (m) WHERE n.id = 1 OR m.id = 'missing'
             RETURN n.name AS n, m.name AS m ORDER BY n, m"
        )
        .await,
        vec![
            vec!["A".to_string(), "A".to_string()],
            vec!["A".to_string(), "B".to_string()],
            vec!["A".to_string(), "C".to_string()],
            vec!["A".to_string(), "D".to_string()],
        ]
    );
    assert_eq!(
        pair_rows(
            &mut session,
            "MATCH (n), (m) WHERE n.id > 9 OR m.id = 'x'
             RETURN n.name AS n, m.name AS m ORDER BY n, m"
        )
        .await,
        vec![
            vec!["A".to_string(), "B".to_string()],
            vec!["B".to_string(), "B".to_string()],
            vec!["C".to_string(), "B".to_string()],
            vec!["D".to_string(), "B".to_string()],
        ]
    );
    assert_eq!(
        pair_rows(
            &mut session,
            "MATCH (n), (m) WHERE n.id = 1 AND m.id = 'x'
             RETURN n.name AS n, m.name AS m"
        )
        .await,
        vec![vec!["A".to_string(), "B".to_string()]]
    );
    assert_eq!(
        pair_rows(
            &mut session,
            "MATCH (n) WHERE n.id = 1 OR n.id = 'x' RETURN n.name AS n ORDER BY n"
        )
        .await,
        vec![vec!["A".to_string()], vec!["B".to_string()]]
    );

    let incompatible = session
        .query("MATCH (n:B), (m:B) WHERE n.id = 1 OR m.id = 1 RETURN n.name AS n")
        .await;
    assert!(
        matches!(incompatible, Err(graphfusion::Error::InvalidQuery(ref message)) if message.contains("incomparable")),
        "{incompatible:?}"
    );

    session
        .run("MATCH (a:A), (b:B), (c:C) INSERT (a)-[:E]->(b), (c)-[:E]->(:Y {name: 'Y', id: 'y'})")
        .await
        .unwrap();
    assert_eq!(
        pair_rows(
            &mut session,
            "MATCH (n)-[:E]->(m) WHERE n.id = 1 OR m.id = 'y'
             RETURN n.name AS n, m.name AS m ORDER BY n, m"
        )
        .await,
        vec![
            vec!["A".to_string(), "B".to_string()],
            vec!["C".to_string(), "Y".to_string()],
        ]
    );

    let mut numeric = Database::new().session();
    numeric
        .execute("CREATE GRAPH g ANY GRAPH; SESSION SET GRAPH g")
        .unwrap();
    numeric
        .run(
            "INSERT (:P {name: 'ten', v: 10}),
                    (:Q {name: 'text', v: '10'}),
                    (:R {name: 'nine', v: 9})",
        )
        .await
        .unwrap();
    assert_eq!(
        pair_rows(
            &mut numeric,
            "MATCH (n), (m) WHERE n.v > 9 OR m.v = '10'
             RETURN n.name AS n, m.name AS m ORDER BY n, m"
        )
        .await,
        vec![
            vec!["nine".to_string(), "text".to_string()],
            vec!["ten".to_string(), "nine".to_string()],
            vec!["ten".to_string(), "ten".to_string()],
            vec!["ten".to_string(), "text".to_string()],
            vec!["text".to_string(), "text".to_string()],
        ]
    );
    numeric
        .run("INSERT (:F {name: 'two', v: 2}), (:G {name: 'half', v: 2.5})")
        .await
        .unwrap();
    assert_eq!(
        pair_rows(
            &mut numeric,
            "MATCH (n:F|G), (m:F|G) WHERE n.v > 2 OR m.v = 2
             RETURN n.name AS n, m.name AS m ORDER BY n, m"
        )
        .await,
        vec![
            vec!["half".to_string(), "half".to_string()],
            vec!["half".to_string(), "two".to_string()],
            vec!["two".to_string(), "two".to_string()],
        ]
    );
}

#[tokio::test]
async fn cross_element_fragments_preserve_wrappers_optional_rows_and_alternatives() {
    let mut s = social().await;
    for predicate in [
        "n.id=1 OR m.id='doc'",
        "(n.id=1 OR m.id='doc') IS TRUE",
        "COALESCE(n.id=1 OR m.id='doc',FALSE)",
        "NULLIF(n.id=1 OR m.id='doc',FALSE)",
    ] {
        let q =
            format!("MATCH(n:Person|Document),(m:Person|Document) FILTER {predicate} RETURN n,m");
        assert_eq!(s.query(&q).await.unwrap().row_count(), 5, "{q}");
    }
    let r=s.query("MATCH(n:Person) OPTIONAL MATCH(m:Person|Document {name:'absent'}) FILTER n.id=1 OR m.id='doc' RETURN n.name AS name").await.unwrap();
    assert_eq!(strings(&r), vec![vec!["Ada".to_string()]]);
    assert_eq!(
        s.query("MATCH(n:Person)|(m:Document) WHERE n.id=1 OR m.id='doc' RETURN 1 AS v")
            .await
            .unwrap()
            .row_count(),
        2
    );
    assert_eq!(
        s.query("LET k=1 MATCH(n:Person|Document) WHERE n.id='1' OR k IS NULL RETURN n")
            .await
            .unwrap()
            .row_count(),
        0
    );
}

#[tokio::test]
async fn next_keeps_the_matched_property_schema_with_other_fragment_types_present() {
    let mut s = social().await;
    let direct = s.query("MATCH(p:Person) RETURN p.id AS id").await.unwrap();
    assert_eq!(direct.schema.field(0).data_type(), &DataType::Int64);
    let next = s
        .query("MATCH(p:Person) RETURN p NEXT RETURN p.id AS id")
        .await
        .unwrap();
    assert_eq!(next.schema.field(0).data_type(), &DataType::Int64);
}

#[tokio::test]
async fn next_keeps_numeric_properties_executable() {
    let mut s = social().await;
    s.query("MATCH(p:Person) RETURN p NEXT RETURN p.id+0 AS id")
        .await
        .unwrap();
}

#[tokio::test]
async fn next_yield_and_repeated_reference_copies_keep_source_types() {
    let mut s = social().await;
    let r = s.query("MATCH(p:Person) LET copy=p RETURN copy AS first NEXT YIELD first AS second RETURN second AS third NEXT RETURN third.id+0 AS id ORDER BY id").await.unwrap();
    assert_eq!(r.schema.field(0).data_type(), &DataType::Int64);
    assert_eq!(
        strings(&r),
        vec![vec!["1".to_string()], vec!["2".to_string()]]
    );
}

#[tokio::test]
async fn next_composite_references_do_not_adopt_one_branchs_layout() {
    let mut s = social().await;
    let r = s.query("MATCH(p:Person) RETURN p UNION ALL MATCH(d:Document) RETURN d AS p NEXT RETURN p.id AS id ORDER BY id").await.unwrap();
    assert_eq!(r.schema.field(0).data_type(), &DataType::Utf8);
    assert_eq!(r.row_count(), 3);
}

#[tokio::test]
async fn next_except_and_intersect_keep_the_left_reference_layout() {
    let mut s = social().await;
    s.run("CREATE GRAPH other ANY; USE GRAPH other INSERT (:Document {id:'other'})")
        .await
        .unwrap();
    for query in [
        "MATCH(p:Person) RETURN p EXCEPT MATCH(d:Document) RETURN d AS p NEXT RETURN p.id+0 AS id ORDER BY id",
        "MATCH(p:Person) RETURN p INTERSECT MATCH(d) RETURN d AS p NEXT RETURN p.id+0 AS id ORDER BY id",
        "MATCH(p:Person) RETURN p EXCEPT USE GRAPH other MATCH(d) RETURN d AS p NEXT RETURN p.id+0 AS id ORDER BY id",
    ] {
        let r = s.query(query).await.unwrap();
        assert_eq!(r.schema.field(0).data_type(), &DataType::Int64, "{query}");
        assert_eq!(strings(&r), vec![vec!["1".to_string()],vec!["2".to_string()]], "{query}");
    }
}

#[tokio::test]
async fn early_where_does_not_repeat_native_integer_comparisons_after_widening() {
    let mut s = Database::new().session();
    s.run("CREATE GRAPH g ANY; SESSION SET GRAPH g; INSERT (:I {v:9007199254740993}),(:F {v:0.0}),(:M {k:1})").await.unwrap();
    for q in [
        "MATCH(n:I|F) WHERE n.v>9007199254740992 RETURN COUNT(*) AS hits",
        "MATCH(n:I|F),(m:M) WHERE n.v>9007199254740992 AND m.k=1 RETURN COUNT(*) AS hits",
    ] {
        assert_eq!(
            strings(&s.query(q).await.unwrap()),
            vec![vec!["1".to_string()]],
            "{q}"
        );
    }
}

#[tokio::test]
async fn alternatives_align_property_values_and_presence_without_ascii_case() {
    let mut s = Database::new().session();
    s.run("CREATE GRAPH g ANY; SESSION SET GRAPH g; INSERT(:A {age:1}),(:B {AGE:2}),(:C {name:'empty'})").await.unwrap();
    let r = s
        .query(
            "MATCH(n:A)|(N:B) RETURN n.age AS age,PROPERTY_EXISTS(N,aGe) AS present ORDER BY age",
        )
        .await
        .unwrap();
    assert_eq!(
        strings(&r),
        vec![
            vec!["1".to_string(), "true".to_string()],
            vec!["2".to_string(), "true".to_string()]
        ]
    );
    let r = s
        .query(
            "MATCH(n:A)|(N:C) RETURN n.AGE AS age,PROPERTY_EXISTS(N,aGe) AS present ORDER BY age",
        )
        .await
        .unwrap();
    assert_eq!(
        strings(&r),
        vec![
            vec!["1".to_string(), "true".to_string()],
            vec!["NULL".to_string(), "false".to_string()]
        ]
    );
    let r = s
        .query("MATCH(n:A)|(N:B) WHERE n.aGe>0 RETURN N.AGE+0 AS age ORDER BY age")
        .await
        .unwrap();
    assert_eq!(
        strings(&r),
        vec![vec!["1".to_string()], vec!["2".to_string()]]
    );
}

#[tokio::test]
async fn result_expressions_compare_mixed_properties_on_each_fragment() {
    let mut session = Database::new().session();
    session
        .execute("CREATE GRAPH g ANY GRAPH; SESSION SET GRAPH g")
        .unwrap();
    session
        .run(
            "INSERT (:Person {name: 'Ada', id: 1}),
                    (:Person {name: 'Bea', id: 2}),
                    (:Document {title: 'Spec', id: '1'})",
        )
        .await
        .unwrap();

    let projected = session
        .query(
            "MATCH (n:Person|Document)
             RETURN n.name AS name, n.id = '1' AS text_id, n.id = 1 AS number_id
             ORDER BY name",
        )
        .await
        .unwrap();
    assert_eq!(
        strings(&projected),
        vec![
            vec!["Ada".to_string(), "NULL".to_string(), "true".to_string()],
            vec!["Bea".to_string(), "NULL".to_string(), "false".to_string()],
            vec!["NULL".to_string(), "true".to_string(), "NULL".to_string()],
        ]
    );

    let wrapped = session
        .query(
            "MATCH (n:Person|Document)
             RETURN n.name AS name, n.id = '1' OR FALSE AS eq
             ORDER BY name",
        )
        .await
        .unwrap();
    assert_eq!(
        strings(&wrapped),
        vec![
            vec!["Ada".to_string(), "NULL".to_string()],
            vec!["Bea".to_string(), "NULL".to_string()],
            vec!["NULL".to_string(), "true".to_string()],
        ]
    );

    let selected = session
        .query(
            "MATCH (n:Person|Document)
             SELECT n.name AS name WHERE n.id = '1'",
        )
        .await
        .unwrap();
    assert_eq!(strings(&selected), vec![vec!["NULL".to_string()]]);

    let excluded = session
        .query(
            "MATCH (n:Person|Document)
             SELECT n.name AS name WHERE NOT n.id = '1'",
        )
        .await
        .unwrap();
    assert_eq!(excluded.row_count(), 0);

    let ordered = session
        .query(
            "MATCH (n:Person|Document)
             ORDER BY n.id = '1', n.name
             RETURN n.name AS name",
        )
        .await
        .unwrap();
    assert_eq!(
        strings(&ordered),
        vec![
            vec!["NULL".to_string()],
            vec!["Ada".to_string()],
            vec!["Bea".to_string()]
        ]
    );

    let grouped = session
        .query(
            "MATCH (n:Person|Document)
             RETURN n.name AS name, n.id = '1' AS eq, COUNT(*) AS c
             GROUP BY n
             ORDER BY name",
        )
        .await
        .unwrap();
    assert_eq!(
        strings(&grouped),
        vec![
            vec!["Ada".to_string(), "NULL".to_string(), "1".to_string()],
            vec!["Bea".to_string(), "NULL".to_string(), "1".to_string()],
            vec!["NULL".to_string(), "true".to_string(), "1".to_string()],
        ]
    );

    let having = session
        .query(
            "MATCH (n:Person|Document)
             SELECT n.name AS name GROUP BY n HAVING n.id = '1'",
        )
        .await
        .unwrap();
    assert_eq!(strings(&having), vec![vec!["NULL".to_string()]]);

    let numeric = session
        .query("MATCH (n:Person) RETURN n.name AS name, n.id = 1 AS eq ORDER BY name")
        .await
        .unwrap();
    assert_eq!(
        strings(&numeric),
        vec![
            vec!["Ada".to_string(), "true".to_string()],
            vec!["Bea".to_string(), "false".to_string()]
        ]
    );
}

#[tokio::test]
async fn repeated_stored_comparisons_keep_distinct_ordering_and_group_checks() {
    let mut session = social().await;
    for query in [
        "MATCH (n:Person|Document) RETURN DISTINCT n.id = 'doc' AS eq ORDER BY n.id = 'doc'",
        "MATCH (n:Person|Document) RETURN DISTINCT n.id = 'doc' AS eq ORDER BY N.ID = 'doc'",
        "MATCH (n:Person|Document) LET r=n RETURN DISTINCT r.id = 'doc' AS eq ORDER BY R.ID = 'doc'",
    ] {
        assert_eq!(strings(&session.query(query).await.unwrap()), vec![vec!["true".to_string()], vec!["NULL".to_string()]]);
    }
    let invalid = session.query("MATCH (n:Person|Document), (m:Person) LET k=m.name RETURN n.id=1 OR k IS NULL AS b, COUNT(*) AS c GROUP BY n").await;
    assert!(invalid.is_err(), "ungrouped scalar was accepted");
}

#[tokio::test]
async fn mixed_case_element_id_self_comparison_preserves_native_integer_filter() {
    let mut session = Database::new().session();
    session
        .execute("CREATE GRAPH g ANY GRAPH; SESSION SET GRAPH g")
        .unwrap();
    session
        .run("INSERT (:I {v:9007199254740993}), (:F {v:0.0})")
        .await
        .unwrap();
    for query in [
        "MATCH(N:I|F) WHERE N.v>9007199254740992 OR ELEMENT_ID(N)<>ELEMENT_ID(N) RETURN COUNT(*) AS hits",
        "MATCH(N:I|F) WHERE N.v>9007199254740992 OR ELEMENT_ID(n)<>ELEMENT_ID(N) RETURN COUNT(*) AS hits",
    ] {
        assert_eq!(strings(&session.query(query).await.unwrap()), vec![vec!["1".to_string()]]);
    }
}

#[tokio::test]
async fn mixed_property_pairs_compare_stored_types() {
    let mut session = Database::new().session();
    session
        .execute("CREATE GRAPH g ANY GRAPH; SESSION SET GRAPH g")
        .unwrap();
    session
        .run(
            "INSERT (:P {name: 'ten', v: 10}),
                    (:P {name: 'nine', v: 9}),
                    (:D {name: 'doc', v: 'x'})",
        )
        .await
        .unwrap();

    let greater = session
        .query(
            "MATCH (n), (m) WHERE n.v > m.v
             RETURN n.name AS n, m.name AS m ORDER BY n, m",
        )
        .await
        .unwrap();
    assert!(
        greater.logical_plan.contains("__gf_frag_"),
        "mixed property comparison stayed on the unified column:\n{}",
        greater.logical_plan
    );
    assert_eq!(
        strings(&greater),
        vec![vec!["ten".to_string(), "nine".to_string()]]
    );
    session
        .run("MATCH (n {name: 'ten'}), (m {name: 'nine'}) INSERT (n)-[:E]->(m)")
        .await
        .unwrap();
    let parenthesized = session
        .query(
            "MATCH ((n)-[:E]-(m) WHERE n.v > m.v)
             RETURN n.name AS n, m.name AS m ORDER BY n, m",
        )
        .await
        .unwrap();
    assert_eq!(
        strings(&parenthesized),
        vec![vec!["ten".to_string(), "nine".to_string()]]
    );

    let ten_nine = session
        .query(
            "MATCH (n {name: 'ten'}), (m {name: 'nine'})
             RETURN n.v > m.v AS gt, n.v > m.v OR FALSE AS or_false, n.v > m.v XOR FALSE AS xor_false",
        )
        .await
        .unwrap();
    assert_eq!(
        strings(&ten_nine),
        vec![vec![
            "true".to_string(),
            "true".to_string(),
            "true".to_string()
        ]]
    );
    let nine_ten = session
        .query("MATCH (n {name: 'nine'}), (m {name: 'ten'}) RETURN n.v > m.v AS gt")
        .await
        .unwrap();
    assert_eq!(strings(&nine_ten), vec![vec!["false".to_string()]]);

    let negated = session
        .query(
            "MATCH (n), (m) WHERE NOT n.v = m.v
             RETURN n.name AS n, m.name AS m ORDER BY n, m",
        )
        .await
        .unwrap();
    assert_eq!(
        strings(&negated),
        vec![
            vec!["nine".to_string(), "ten".to_string()],
            vec!["ten".to_string(), "nine".to_string()]
        ]
    );

    for query in [
        "MATCH (n), (m) FILTER n.v > m.v RETURN n.name AS n, m.name AS m ORDER BY n, m",
        "MATCH (n), (m) LET gt = n.v > m.v FILTER gt RETURN n.name AS n, m.name AS m ORDER BY n, m",
        "MATCH (n), (m) SELECT n.name AS n, m.name AS m WHERE n.v > m.v ORDER BY n, m",
        "MATCH (n), (m) ORDER BY n.v > m.v DESC NULLS LAST, n.name, m.name RETURN n.name AS n, m.name AS m LIMIT 1",
        "MATCH (n), (m) RETURN n.name AS left_name, m.name AS right_name ORDER BY n.v > m.v DESC NULLS LAST, left_name, right_name LIMIT 1",
    ] {
        let result = session.query(query).await.unwrap();
        assert_eq!(
            strings(&result),
            vec![vec!["ten".to_string(), "nine".to_string()]],
            "{query}"
        );
    }

    let grouped = session
        .query(
            "MATCH (n {name: 'ten'}), (m {name: 'nine'})
             RETURN n.v > m.v AS gt, COUNT(*) AS c GROUP BY n, m",
        )
        .await
        .unwrap();
    assert_eq!(
        strings(&grouped),
        vec![vec!["true".to_string(), "1".to_string()]]
    );
    let having = session
        .query(
            "MATCH (n), (m)
             SELECT n.name AS left_name, m.name AS right_name, COUNT(*) AS c
             GROUP BY n, m HAVING n.v > m.v ORDER BY left_name, right_name",
        )
        .await
        .unwrap();
    assert_eq!(
        strings(&having),
        vec![vec!["ten".to_string(), "nine".to_string(), "1".to_string()]]
    );

    let same_label = session
        .query(
            "MATCH (n:P), (m:P) WHERE n.v > m.v
             RETURN n.name AS n, m.name AS m ORDER BY n, m",
        )
        .await
        .unwrap();
    assert!(
        !same_label.logical_plan.contains("__gf_frag_"),
        "same-type comparison joined fragments:\n{}",
        same_label.logical_plan
    );
    assert_eq!(
        strings(&same_label),
        vec![vec!["ten".to_string(), "nine".to_string()]]
    );

    session
        .run("INSERT (:D {name: 'text', v: '10'})")
        .await
        .unwrap();
    let equals = session
        .query(
            "MATCH (n), (m) WHERE n.v = m.v
             RETURN n.name AS n, m.name AS m ORDER BY n, m",
        )
        .await
        .unwrap();
    assert_eq!(
        strings(&equals),
        vec![
            vec!["doc".to_string(), "doc".to_string()],
            vec!["nine".to_string(), "nine".to_string()],
            vec!["ten".to_string(), "ten".to_string()],
            vec!["text".to_string(), "text".to_string()]
        ]
    );
    let mapped = session
        .query(
            "MATCH (m:D), (n {v: m.v})
             RETURN n.name AS n, m.name AS m ORDER BY n, m",
        )
        .await
        .unwrap();
    assert_eq!(
        strings(&mapped),
        vec![
            vec!["doc".to_string(), "doc".to_string()],
            vec!["text".to_string(), "text".to_string()]
        ]
    );
    let named = session
        .query(
            "MATCH (n), (m) WHERE n.v = m.v AND n.name = 'ten'
             RETURN m.name AS m",
        )
        .await
        .unwrap();
    assert_eq!(strings(&named), vec![vec!["ten".to_string()]]);
    let mismatch = session
        .query("MATCH (n:P), (m:D) WHERE n.v = m.v RETURN n.name AS n")
        .await
        .unwrap_err();
    assert!(
        mismatch.to_string().contains("incomparable types"),
        "{mismatch}"
    );

    session
        .run(
            "INSERT (:P {name: 'tenw', v: 10, w: 9}),
                    (:D {name: 'docw', v: 'x', w: 1})",
        )
        .await
        .unwrap();
    let same_element = session
        .query(
            "MATCH (n) WHERE n.name = 'tenw' OR n.name = 'docw'
             RETURN n.name AS name, n.v > n.w AS gt ORDER BY name",
        )
        .await
        .unwrap();
    assert_eq!(
        strings(&same_element),
        vec![
            vec!["docw".to_string(), "NULL".to_string()],
            vec!["tenw".to_string(), "true".to_string()]
        ]
    );

    session
        .execute("CREATE GRAPH h ANY GRAPH; SESSION SET GRAPH h")
        .unwrap();
    session
        .run(
            "INSERT (:P {name: 'low', score: 1}),
                    (:M {name: 'high', score: 1.5})",
        )
        .await
        .unwrap();
    let widened = session
        .query(
            "MATCH (n), (m) WHERE n.score > m.score
             RETURN n.name AS n, m.name AS m ORDER BY n, m",
        )
        .await
        .unwrap();
    assert!(
        !widened.logical_plan.contains("__gf_frag_"),
        "numeric widening joined fragments:\n{}",
        widened.logical_plan
    );
    assert_eq!(
        strings(&widened),
        vec![vec!["high".to_string(), "low".to_string()]]
    );
}

#[tokio::test]
async fn repeated_property_pairs_keep_distinct_ordering_wrappers_and_references() {
    let mut session = social().await;
    for query in [
        "MATCH(n),(m) RETURN DISTINCT n.id=m.id AS eq ORDER BY n.id=m.id",
        "MATCH(n),(m) RETURN DISTINCT n.id=m.id AS eq ORDER BY N.ID=M.ID",
        "MATCH(n),(m) LET r=n, s=m RETURN DISTINCT r.id=s.id AS eq ORDER BY R.ID=S.ID",
        "MATCH(n),(m) RETURN DISTINCT (n.id=m.id) IS UNKNOWN AS b ORDER BY (N.ID=M.ID) IS UNKNOWN",
    ] {
        let result = strings(&session.query(query).await.unwrap());
        assert!(!result.is_empty(), "{query}");
    }
    let result = session.query("OPTIONAL MATCH(n:Missing) MATCH(m:Person|Document) RETURN (n.id=m.id) IS UNKNOWN AS b, COALESCE(n.id=m.id,TRUE) AS c").await.unwrap();
    assert!(strings(&result)
        .iter()
        .all(|r| r == &vec!["true".to_string(), "true".to_string()]));
}

#[tokio::test]
async fn stored_result_comparisons_keep_alias_and_aggregate_binding_context() {
    let mut session = social().await;
    let result = session
        .query("MATCH(n:Person|Document) RETURN FALSE AS ok ORDER BY n.id='doc' OR ok")
        .await
        .unwrap();
    assert_eq!(strings(&result), vec![vec!["false".to_string()]; 3]);
    let result = session.query("MATCH(n:Person|Document) SELECT n.id AS id, COUNT(*) AS c GROUP BY n HAVING n.id='doc' OR COUNT(*)>1").await.unwrap();
    assert_eq!(
        strings(&result),
        vec![vec!["doc".to_string(), "1".to_string()]]
    );
    let result = session.query("MATCH(n:Person|Document),(m:Person|Document) RETURN FALSE AS ok ORDER BY n.id=m.id OR ok").await.unwrap();
    assert_eq!(strings(&result).len(), 9);
}

#[tokio::test]
async fn stored_result_comparisons_preserve_an_earlier_order_and_page() {
    let mut session = social().await;
    for suffix in ["", " OFFSET 1 LIMIT 2"] {
        let expected = strings(
            &session
                .query(&format!(
                    "MATCH(n:Person|Document) ORDER BY n.name DESC{suffix} RETURN n.name AS name"
                ))
                .await
                .unwrap(),
        );
        for result in [
            "RETURN n.name AS name, n.id='doc' AS eq",
            "SELECT n.name AS name WHERE n.id='doc' OR TRUE",
        ] {
            let rows = strings(
                &session
                    .query(&format!(
                        "MATCH(n:Person|Document) ORDER BY n.name DESC{suffix} {result}"
                    ))
                    .await
                    .unwrap(),
            );
            assert_eq!(
                rows.iter().map(|r| vec![r[0].clone()]).collect::<Vec<_>>(),
                expected
            );
        }
    }
}

#[tokio::test]
async fn incompatible_property_pairs_cannot_compare_unified_strings() {
    let mut session = Database::new().session();
    session.run("CREATE GRAPH g ANY GRAPH; SESSION SET GRAPH g; INSERT (:I {v:1}), (:B {v:TRUE}), (:S {v:'1',w:'1'})").await.unwrap();
    for query in [
        "MATCH(n:I|B),(m:S) RETURN n.v=m.w AS eq",
        "MATCH(n:I|B),(m:S) WHERE n.v=m.v RETURN n.v AS value",
        "MATCH(n:I|B),(m:S) FILTER n.v=m.v RETURN n.v AS value",
        "MATCH(m:S),(n:I|B {v:m.w}) RETURN n.v AS value",
    ] {
        let error = session.query(query).await.unwrap_err();
        assert!(
            error.to_string().contains("incomparable types"),
            "{query}: {error}"
        );
    }
}

#[tokio::test]
async fn property_pair_joins_preserve_clause_order_and_materialized_reference_order() {
    let mut session = social().await;
    for query in [
        "MATCH(n:Person|Document) ORDER BY n.name DESC NULLS LAST LET eq=n.id=n.id RETURN n.name AS name,eq LIMIT 1",
        "MATCH(n:Person|Document) ORDER BY n.name DESC NULLS LAST FILTER n.id=n.id RETURN n.name AS name, TRUE AS eq LIMIT 1",
        "MATCH(n:Person|Document) ORDER BY n.name DESC NULLS LAST LET r=n RETURN n.name AS name,n.id=r.id AS eq LIMIT 1",
        "MATCH(n:Person|Document) ORDER BY n.name DESC NULLS LAST LET r=n SELECT n.name AS name,TRUE AS eq WHERE n.id=r.id LIMIT 1",
    ] {
        assert_eq!(strings(&session.query(query).await.unwrap()),vec![vec!["Bea".to_string(),"true".to_string()]],"{query}");
    }
    let result = session.query("MATCH(n:Person|Document) ORDER BY n.name DESC NULLS LAST LET eq=n.id=n.id OFFSET 1 LIMIT 1 RETURN n.name AS name").await.unwrap();
    assert_eq!(strings(&result), vec![vec!["Ada".to_string()]]);
}
