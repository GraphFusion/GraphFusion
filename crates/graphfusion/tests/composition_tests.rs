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
