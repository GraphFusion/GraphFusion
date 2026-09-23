use datafusion::{arrow::datatypes::DataType, common::ScalarValue};
use graphfusion::{Database, Error, QueryResult, Value};

#[tokio::test]
async fn mixed_program_outputs_schema_context_and_atomic_continuations() {
    let db = Database::new();
    let mut session = db.session();
    let outputs = session
        .run("CREATE GRAPH g ANY GRAPH; SESSION SET GRAPH g; RETURN 42 AS result")
        .await
        .unwrap();
    assert_eq!(outputs.len(), 3);
    assert!(
        matches!(&outputs[2], graphfusion::StatementOutput::Query(result) if result.row_count() == 1)
    );
    let outputs = session
        .run("AT SCHEMA main USE GRAPH g MATCH (n) RETURN ELEMENT_ID(n) AS id")
        .await
        .unwrap();
    assert!(
        matches!(&outputs[0], graphfusion::StatementOutput::Query(result) if result.row_count() == 0)
    );
    assert!(session
        .run("CREATE GRAPH rolled_back ANY GRAPH NEXT RETURN 1 AS n")
        .await
        .is_err());
    assert!(session
        .query("USE GRAPH rolled_back MATCH (n) RETURN ELEMENT_ID(n) AS id")
        .await
        .is_err());
    assert!(session.run("SESSION CLOSE; RETURN 1 AS n").await.is_err());
    assert!(matches!(
        session.run("RETURN 1 AS n").await,
        Err(Error::SessionClosed)
    ));
}

fn row(result: &QueryResult) -> Vec<ScalarValue> {
    assert_eq!(result.row_count(), 1);
    let batch = result.batches.iter().find(|b| b.num_rows() > 0).unwrap();
    batch
        .columns()
        .iter()
        .map(|column| ScalarValue::try_from_array(column, 0).unwrap())
        .collect()
}

#[tokio::test]
async fn scalar_query_runs_through_datafusion_with_arrow_schema() {
    let mut session = Database::new().session();
    let result = session
        .query("RETURN 1 + 2 * 3 AS answer, 'graph' || 'fusion' AS name")
        .await
        .unwrap();
    assert_eq!(
        row(&result),
        vec![
            ScalarValue::Int64(Some(7)),
            ScalarValue::Utf8(Some("graphfusion".into()))
        ]
    );
    assert_eq!(result.schema.field(0).name(), "answer");
    assert_eq!(result.schema.field(0).data_type(), &DataType::Int64);
    assert!(
        result.logical_plan.contains("Projection"),
        "{}",
        result.logical_plan
    );
    assert!(
        result.physical_plan.contains("ProjectionExec"),
        "{}",
        result.physical_plan
    );
    assert_eq!(result.commit_seq, 0);
}

#[tokio::test]
async fn parameters_bind_values_without_query_text_substitution() {
    let db = Database::new();
    let mut first = db.session();
    let mut other = db.session();
    let payload = "'; CREATE GRAPH injected ANY GRAPH; --";
    first
        .set_parameter("text", Value::String(payload.into()))
        .unwrap();
    first.execute("SESSION SET VALUE $n INTEGER = 6").unwrap();
    let result = first
        .query("RETURN $text AS text, $n * 7 AS answer")
        .await
        .unwrap();
    assert_eq!(
        row(&result),
        vec![
            ScalarValue::Utf8(Some(payload.into())),
            ScalarValue::Int64(Some(42))
        ]
    );
    assert!(matches!(
        other.query("RETURN $n AS n").await,
        Err(Error::NotFound(_))
    ));
    assert_eq!(
        db.with_catalog(|catalog| catalog.entries().count())
            .unwrap(),
        2
    );
}

#[tokio::test]
async fn three_valued_logic_and_null_predicates() {
    let mut session = Database::new().session();
    let result = session.query("RETURN UNKNOWN AS u, FALSE AND UNKNOWN AS f, TRUE OR UNKNOWN AS t, TRUE XOR UNKNOWN AS x, UNKNOWN IS UNKNOWN AS iu, UNKNOWN IS NOT TRUE AS nt, NULL IS NULL AS n, COALESCE(NULL, 9) AS c, NULLIF(1, 1) AS ni").await.unwrap();
    assert_eq!(
        row(&result),
        vec![
            ScalarValue::Boolean(None),
            ScalarValue::Boolean(Some(false)),
            ScalarValue::Boolean(Some(true)),
            ScalarValue::Boolean(None),
            ScalarValue::Boolean(Some(true)),
            ScalarValue::Boolean(Some(true)),
            ScalarValue::Boolean(Some(true)),
            ScalarValue::Int64(Some(9)),
            ScalarValue::Int64(None),
        ]
    );
    assert_eq!(result.schema.field(0).data_type(), &DataType::Boolean);
    let nulls = session
        .query("RETURN NULL + 1 AS n, 'a' || NULL AS s")
        .await
        .unwrap();
    assert_eq!(
        row(&nulls),
        vec![ScalarValue::Int64(None), ScalarValue::Utf8(None)]
    );
    let filtered = session.query("FILTER UNKNOWN RETURN 1 AS n").await.unwrap();
    assert_eq!(filtered.row_count(), 0);
    assert_eq!(filtered.schema.field(0).data_type(), &DataType::Int64);
}

#[tokio::test]
async fn untyped_null_concatenation_infers_string_type() {
    let mut session = Database::new().session();
    session.set_parameter("a", Value::Null).unwrap();
    session.set_parameter("b", Value::Null).unwrap();
    let result = session
        .query(
            "LET n = NULL RETURN NULL || NULL AS literal_null, \
             $a || $b AS parameter_null, n || n AS bound_null, \
             (NULL || NULL) || 'x' AS nested_null, \
             COALESCE($a || $b, 'fallback') AS fallback",
        )
        .await
        .unwrap();
    assert_eq!(
        row(&result),
        vec![
            ScalarValue::Utf8(None),
            ScalarValue::Utf8(None),
            ScalarValue::Utf8(None),
            ScalarValue::Utf8(None),
            ScalarValue::Utf8(Some("fallback".into())),
        ]
    );
}

#[tokio::test]
async fn nullif_preserves_untyped_nulls_in_outer_expressions() {
    let mut session = Database::new().session();
    session.set_parameter("a", Value::Null).unwrap();
    session.set_parameter("b", Value::Null).unwrap();
    let result = session
        .query(
            "LET n = NULLIF($a, $b) \
             RETURN n, COALESCE(NULLIF(NULL, NULL), 1) AS i, \
             COALESCE(n, TRUE) AS b, COALESCE(n, 'fallback') AS s, \
             NOT n AS unknown, n + 1 AS number_null",
        )
        .await
        .unwrap();
    assert_eq!(
        row(&result),
        vec![
            ScalarValue::Null,
            ScalarValue::Int64(Some(1)),
            ScalarValue::Boolean(Some(true)),
            ScalarValue::Utf8(Some("fallback".into())),
            ScalarValue::Boolean(None),
            ScalarValue::Int64(None),
        ]
    );

    session
        .execute("SESSION SET VALUE $n INTEGER = NULL")
        .unwrap();
    let typed = session
        .query("RETURN NULLIF($n, $n) AS n, NULLIF(1, 2) AS value")
        .await
        .unwrap();
    assert_eq!(
        row(&typed),
        vec![ScalarValue::Int64(None), ScalarValue::Int64(Some(1))]
    );
    assert!(matches!(
        session
            .query("RETURN COALESCE(NULLIF($n, $n), TRUE) AS value")
            .await,
        Err(Error::InvalidQuery(_))
    ));
}

#[tokio::test]
async fn typed_parameter_nulls_and_numeric_initializers_keep_their_type_family() {
    let mut session = Database::new().session();
    session.execute("SESSION SET VALUE $flag BOOLEAN = UNKNOWN; SESSION SET VALUE $n INTEGER = NULL; SESSION SET VALUE $f FLOAT = 1").unwrap();
    let result = session
        .query("RETURN $flag AS flag, $n AS n, $f AS f")
        .await
        .unwrap();
    assert_eq!(
        row(&result),
        vec![
            ScalarValue::Boolean(None),
            ScalarValue::Int64(None),
            ScalarValue::Float64(Some(1.0))
        ]
    );
    assert_eq!(result.schema.field(0).data_type(), &DataType::Boolean);
    assert_eq!(result.schema.field(1).data_type(), &DataType::Int64);
    assert_eq!(result.schema.field(2).data_type(), &DataType::Float64);
    assert!(matches!(
        session.query("RETURN $flag + 1 AS n").await,
        Err(Error::InvalidQuery(_))
    ));
}

#[tokio::test]
async fn let_scope_delimited_identifiers_filter_and_result_aliases() {
    let mut session = Database::new().session();
    let result = session.query("LET `a.b` = 6 LET y = `a.b` + 1 FILTER y > 5 RETURN y AS result ORDER BY result DESC NULLS FIRST LIMIT 1").await.unwrap();
    assert_eq!(row(&result), vec![ScalarValue::Int64(Some(7))]);
    assert!(matches!(
        session.query("LET x = 1 LET x = 2 RETURN x").await,
        Err(Error::InvalidQuery(_))
    ));
    assert!(matches!(
        session.query("RETURN y").await,
        Err(Error::InvalidQuery(_))
    ));
    let wildcard = session.query("LET a = 1, b = 2 RETURN *").await.unwrap();
    assert_eq!(
        row(&wildcard),
        vec![ScalarValue::Int64(Some(1)), ScalarValue::Int64(Some(2))]
    );
    assert!(session.query("RETURN *").await.is_err());
}

#[tokio::test]
async fn select_distinct_and_pagination_preserve_empty_result_schema() {
    let mut session = Database::new().session();
    assert_eq!(
        row(&session.query("SELECT DISTINCT 5 AS n").await.unwrap()),
        vec![ScalarValue::Int64(Some(5))]
    );
    for query in [
        "RETURN 1 AS n LIMIT 0",
        "RETURN 1 AS n OFFSET 1",
        "FILTER FALSE RETURN 1 AS n",
    ] {
        let result = session.query(query).await.unwrap();
        assert_eq!(result.row_count(), 0, "{query}");
        assert_eq!(result.schema.field(0).name(), "n");
        assert_eq!(result.schema.field(0).data_type(), &DataType::Int64);
    }
    session.set_parameter("limit", Value::Integer(0)).unwrap();
    assert_eq!(
        session
            .query("RETURN 1 AS n LIMIT $limit")
            .await
            .unwrap()
            .row_count(),
        0
    );
    for bad in [Value::Integer(-1), Value::Float(1.0), Value::Null] {
        session.set_parameter("limit", bad).unwrap();
        assert!(matches!(
            session.query("RETURN 1 AS n LIMIT $limit").await,
            Err(Error::InvalidQuery(_))
        ));
    }
}

#[tokio::test]
async fn gql_type_checks_reject_implicit_string_and_boolean_numeric_coercion() {
    let mut session = Database::new().session();
    for query in [
        "RETURN '2' + 1 AS n",
        "RETURN TRUE + 1 AS n",
        "RETURN +'2' AS n",
        "RETURN NOT 1 AS n",
        "FILTER 1 RETURN 1 AS n",
        "RETURN '1' = 1 AS n",
        "RETURN COALESCE(1, '1') AS n",
        "RETURN 1 IS UNKNOWN AS n",
        "RETURN NULL || 1 AS n",
        "RETURN TRUE || NULL AS n",
    ] {
        assert!(
            matches!(session.query(query).await, Err(Error::InvalidQuery(_))),
            "{query}"
        );
    }
    assert!(session.query("RETURN 1 / 0 AS n").await.is_err());
    assert!(session
        .query("RETURN 9223372036854775807 + 1 AS n")
        .await
        .is_err());
    session
        .set_parameter("min", Value::Integer(i64::MIN))
        .unwrap();
    session
        .set_parameter("max", Value::Integer(i64::MAX))
        .unwrap();
    for query in [
        "RETURN -$min AS n",
        "RETURN $min - 1 AS n",
        "RETURN $max * 2 AS n",
        "RETURN $min / -1 AS n",
        "RETURN 1.0 / 0.0 AS n",
        "RETURN 1e308 * 1e308 AS n",
    ] {
        assert!(session.query(query).await.is_err(), "{query}");
    }
    assert_eq!(
        row(&session.query("RETURN $max - 1 AS n").await.unwrap()),
        vec![ScalarValue::Int64(Some(i64::MAX - 1))]
    );
}

#[tokio::test]
async fn unsupported_programs_never_publish_catalog_or_session_effects() {
    let db = Database::new();
    let mut session = db.session();
    let state = session.state().clone();
    for query in [
        "CREATE GRAPH hidden ANY GRAPH; RETURN 1 AS n",
        "SESSION SET VALUE $n INTEGER = 1; RETURN $n AS n",
        "START TRANSACTION; RETURN 1 AS n; COMMIT",
        "RETURN 1 AS n; RETURN 2 AS n",
        "INSERT (:Person)",
        "RETURN 1 AS n UNION RETURN 2 AS n",
        "RETURN COUNT(*) AS n",
    ] {
        assert!(
            matches!(
                session.query(query).await,
                Err(Error::UnsupportedFeature(_))
            ),
            "{query}"
        );
        assert_eq!(session.state(), &state);
        assert_eq!(
            db.with_catalog(|catalog| catalog.entries().count())
                .unwrap(),
            2
        );
    }
    db.checkpoint().unwrap(); // every failed query has released its snapshot lease
    session.execute("SESSION CLOSE").unwrap();
    assert!(matches!(
        session.query("RETURN 1 AS n").await,
        Err(Error::SessionClosed)
    ));
}
