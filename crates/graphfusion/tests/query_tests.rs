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
async fn decimal_parameters_validate_scale_and_read_as_float() {
    let mut session = Database::new().session();
    session
        .execute(
            "SESSION SET VALUE $price DECIMAL(4, 1) = 1.5; \
             SESSION SET VALUE $whole DECIMAL(4) = 10; \
             SESSION SET VALUE $open DECIMAL = 1.25; \
             SESSION SET VALUE $neg DEC(2, 1) = -1.5; \
             SESSION SET VALUE $list LIST<DECIMAL(4, 1)> = [1, NULL, 1.5]",
        )
        .unwrap();
    let result = session
        .query(
            "RETURN $price AS price, $price + 1 AS sum, $whole AS whole, \
             $open AS open, $neg AS neg",
        )
        .await
        .unwrap();
    assert_eq!(
        row(&result),
        vec![
            ScalarValue::Float64(Some(1.5)),
            ScalarValue::Float64(Some(2.5)),
            ScalarValue::Float64(Some(10.0)),
            ScalarValue::Float64(Some(1.25)),
            ScalarValue::Float64(Some(-1.5)),
        ]
    );
    assert_eq!(result.schema.field(0).data_type(), &DataType::Float64);
    let listed = session
        .query("FOR x IN $list WITH ORDINALITY i RETURN i AS i, x AS x ORDER BY i")
        .await
        .unwrap();
    assert_eq!(listed.schema.field(1).data_type(), &DataType::Float64);
    assert_eq!(
        listed
            .batches
            .iter()
            .flat_map(|batch| (0..batch.num_rows())
                .map(|row| { ScalarValue::try_from_array(batch.column(1), row).unwrap() }))
            .collect::<Vec<_>>(),
        vec![
            ScalarValue::Float64(Some(1.0)),
            ScalarValue::Float64(None),
            ScalarValue::Float64(Some(1.5)),
        ]
    );
    for statement in [
        "SESSION SET VALUE $bad DECIMAL(4, 1) = 1.25",
        "SESSION SET VALUE $bad DECIMAL(2, 1) = 10",
        "SESSION SET VALUE $bad DECIMAL(4) = 1.5",
        "SESSION SET VALUE $bad DECIMAL(4, 2) = 100",
        "SESSION SET VALUE $bad DECIMAL(19) = 9007199254740993",
        "SESSION SET VALUE $bad DECIMAL = 'no'",
    ] {
        assert!(
            matches!(session.execute(statement), Err(Error::InvalidDefinition(_))),
            "{statement}"
        );
    }
    session
        .execute("SESSION SET VALUE $exact DECIMAL(16, 0) = 9007199254740992")
        .unwrap();
    let exact = session.query("RETURN $exact AS n").await.unwrap();
    assert_eq!(
        row(&exact),
        vec![ScalarValue::Float64(Some(9007199254740992.0))]
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
async fn coalesce_null_with_byte_string() {
    let mut session = Database::new().session();
    let result = session
        .query(
            "LET n = NULL \
             RETURN COALESCE(NULL, X'0A') AS missing, \
                    COALESCE(X'0A', NULL) AS present, \
                    COALESCE(NULL, NULL, X'0B') AS later, \
                    COALESCE(X'01', X'02') AS first, \
                    COALESCE(n, X'FF') AS bound",
        )
        .await
        .unwrap();
    assert_eq!(result.schema.field(0).data_type(), &DataType::Binary);
    assert_eq!(
        row(&result),
        vec![
            ScalarValue::Binary(Some(vec![0x0A])),
            ScalarValue::Binary(Some(vec![0x0A])),
            ScalarValue::Binary(Some(vec![0x0B])),
            ScalarValue::Binary(Some(vec![0x01])),
            ScalarValue::Binary(Some(vec![0xFF])),
        ]
    );
    session.set_parameter("b", Value::Null).unwrap();
    session
        .execute("SESSION SET VALUE $typed BYTES = NULL")
        .unwrap();
    let parameters = session
        .query("RETURN COALESCE($b, X'0A') AS untyped, COALESCE($typed, X'0A') AS typed")
        .await
        .unwrap();
    assert_eq!(
        row(&parameters),
        vec![
            ScalarValue::Binary(Some(vec![0x0A])),
            ScalarValue::Binary(Some(vec![0x0A])),
        ]
    );
    session
        .run("CREATE GRAPH g ANY GRAPH; SESSION SET GRAPH g; INSERT (:N {b: X'0A'}), (:N {})")
        .await
        .unwrap();
    let matched = session
        .query(
            "MATCH (n) WHERE COALESCE(NULL, n.b) = X'0A' \
             RETURN COALESCE(n.b, X'FF') AS b",
        )
        .await
        .unwrap();
    assert_eq!(row(&matched), vec![ScalarValue::Binary(Some(vec![0x0A]))]);
    assert!(matches!(
        session
            .query("RETURN COALESCE(NULL, X'0A', 'text') AS bad")
            .await,
        Err(Error::InvalidQuery(_))
    ));
    assert_eq!(
        row(&session
            .query("RETURN COALESCE(NULL, NULL) AS n, COALESCE(NULL, 1, 1.5) AS number")
            .await
            .unwrap()),
        vec![ScalarValue::Null, ScalarValue::Float64(Some(1.0))]
    );
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
async fn char_length_limits_the_session_value() {
    let mut session = Database::new().session();
    session
        .execute("SESSION SET VALUE $s CHAR(2) = 'ab'; SESSION SET VALUE $short CHAR(2) = 'a'; SESSION SET VALUE $mark CHAR(1) = 'é'")
        .unwrap();
    let result = session
        .query("RETURN $s AS s, $short AS short, $mark AS mark")
        .await
        .unwrap();
    assert_eq!(
        row(&result),
        vec![
            ScalarValue::Utf8(Some("ab".into())),
            ScalarValue::Utf8(Some("a".into())),
            ScalarValue::Utf8(Some("é".into())),
        ]
    );
    for declaration in [
        "SESSION SET VALUE $bad CHAR(2) = 'abc'",
        "SESSION SET VALUE $bad CHAR(2) = 'abcd'",
        "SESSION SET VALUE $bad CHAR(1) = 'éx'",
        "SESSION SET VALUE $bad CHAR(0) = ''",
        "SESSION SET VALUE $bad CHAR(2, 4) = 'ab'",
        "SESSION SET VALUE $items LIST<CHAR(2)> = ['ab', 'abcd']",
    ] {
        assert!(
            matches!(
                session.execute(declaration),
                Err(Error::InvalidDefinition(_))
            ),
            "{declaration}"
        );
    }
    session
        .execute("SESSION SET VALUE $items LIST<CHAR(2)> = ['ab', 'a']")
        .unwrap();
}

#[tokio::test]
async fn float_parameters_reject_integers_that_are_not_exact() {
    let mut session = Database::new().session();
    for statement in [
        "SESSION SET VALUE $f FLOAT = 9007199254740993",
        "SESSION SET VALUE $f DOUBLE = -9007199254740993",
        "SESSION SET VALUE $f REAL = 9223372036854775807",
        "SESSION SET VALUE $xs LIST<FLOAT> = [1, 9007199254740993]",
        "SESSION SET VALUE $nested LIST<LIST<FLOAT>> = [[9007199254740995]]",
    ] {
        assert!(
            matches!(session.execute(statement), Err(Error::InvalidDefinition(_))),
            "{statement}"
        );
    }
    assert!(matches!(
        session.query("RETURN $f AS f").await,
        Err(Error::NotFound(_))
    ));

    session
        .execute(
            "SESSION SET VALUE $exact FLOAT = 9007199254740992; \
             SESSION SET VALUE $next FLOAT = 9007199254740994; \
             SESSION SET VALUE $one FLOAT = 1; \
             SESSION SET VALUE $half FLOAT = 1.5; \
             SESSION SET VALUE $missing FLOAT = NULL; \
             SESSION SET VALUE $xs LIST<FLOAT> = [9007199254740992, NULL, 2]; \
             SESSION SET VALUE $big INTEGER = 9007199254740993",
        )
        .unwrap();
    let result = session
        .query(
            "RETURN $exact AS exact, $next AS next, $one AS one, $half AS half, \
             $missing AS missing, $big AS big",
        )
        .await
        .unwrap();
    assert_eq!(result.schema.field(0).data_type(), &DataType::Float64);
    assert_eq!(result.schema.field(4).data_type(), &DataType::Float64);
    assert_eq!(result.schema.field(5).data_type(), &DataType::Int64);
    assert_eq!(
        row(&result),
        vec![
            ScalarValue::Float64(Some(9007199254740992.0)),
            ScalarValue::Float64(Some(9007199254740994.0)),
            ScalarValue::Float64(Some(1.0)),
            ScalarValue::Float64(Some(1.5)),
            ScalarValue::Float64(None),
            ScalarValue::Int64(Some(9007199254740993)),
        ]
    );
    let list = session
        .query("FOR x IN $xs RETURN x ORDER BY x")
        .await
        .unwrap();
    assert_eq!(list.schema.field(0).data_type(), &DataType::Float64);
    assert_eq!(
        scalar_column(&list),
        vec![
            ScalarValue::Float64(Some(2.0)),
            ScalarValue::Float64(Some(9007199254740992.0)),
            ScalarValue::Float64(None),
        ]
    );
}

fn scalar_column(result: &QueryResult) -> Vec<ScalarValue> {
    result
        .batches
        .iter()
        .flat_map(|batch| {
            (0..batch.num_rows())
                .map(|row| ScalarValue::try_from_array(batch.column(0), row).unwrap())
        })
        .collect()
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
async fn insert_return_star_projects_declared_elements() {
    let mut session = Database::new().session();
    let outputs = session
        .run("CREATE GRAPH g ANY GRAPH; SESSION SET GRAPH g; INSERT (n:Seen {k: 1})-[e:E]->(m:Seen {k: 2}) RETURN *")
        .await
        .unwrap();
    let graphfusion::StatementOutput::Query(result) = outputs.last().unwrap() else {
        panic!("expected the insert result, got {outputs:?}");
    };
    assert_eq!(
        result
            .schema
            .fields()
            .iter()
            .map(|field| field.name().as_str())
            .collect::<Vec<_>>(),
        vec!["n", "e", "m"]
    );
    let values = row(result);
    let text = values
        .iter()
        .map(|value| value.to_string())
        .collect::<Vec<_>>();
    assert!(text[0].contains("__gql_element_kind:n"), "{text:?}");
    assert!(text[1].contains("__gql_element_kind:e"), "{text:?}");
    assert!(text[2].contains("__gql_element_kind:n"), "{text:?}");
    let stored = session
        .query("MATCH (n:Seen) RETURN n.k AS k ORDER BY k")
        .await
        .unwrap();
    let keys = stored
        .batches
        .iter()
        .flat_map(|batch| {
            (0..batch.num_rows()).map(move |i| {
                ScalarValue::try_from_array(batch.column(0), i)
                    .unwrap()
                    .to_string()
            })
        })
        .collect::<Vec<_>>();
    assert_eq!(keys, vec!["1".to_string(), "2".to_string()]);
    assert!(session.run("INSERT (:Seen) RETURN *").await.is_err());
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
    for query in ["RETURN 1e999 AS n", "RETURN -1e309 AS n"] {
        assert!(
            matches!(session.query(query).await, Err(Error::Parse(_))),
            "{query}"
        );
    }
    assert!(matches!(
        session.run("SESSION SET VALUE $x = 1e999").await,
        Err(Error::Parse(_))
    ));
    assert_eq!(
        row(&session.query("RETURN 1e308 AS n").await.unwrap()),
        vec![ScalarValue::Float64(Some(1e308))]
    );
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

#[tokio::test]
async fn integer_arithmetic_errors_name_overflow_and_division_by_zero() {
    let mut session = Database::new().session();
    let division = session.query("RETURN 1 / 0 AS q").await.unwrap_err();
    assert!(
        division.to_string().contains("division by zero"),
        "{division}"
    );
    assert!(!division.to_string().contains("Arrow"), "{division}");
    let zero = session.query("RETURN 0 / 0 AS q").await.unwrap_err();
    assert!(zero.to_string().contains("division by zero"), "{zero}");
    let missing = session.query("RETURN NULL / 0 AS q").await.unwrap();
    assert_eq!(row(&missing), vec![ScalarValue::Int64(None)]);

    let sum = session
        .query("RETURN 9223372036854775807 + 1 AS q")
        .await
        .unwrap_err();
    assert!(sum.to_string().contains("integer overflow"), "{sum}");
    assert!(!sum.to_string().contains("Arrow"), "{sum}");
    let product = session
        .query("RETURN 9223372036854775807 * 2 AS q")
        .await
        .unwrap_err();
    assert!(
        product.to_string().contains("integer overflow"),
        "{product}"
    );
    let negated = session
        .query("RETURN -(-9223372036854775808) AS q")
        .await
        .unwrap_err();
    assert!(
        negated.to_string().contains("integer overflow"),
        "{negated}"
    );
    let quotient = session
        .query("RETURN -9223372036854775808 / -1 AS q")
        .await
        .unwrap_err();
    assert!(
        quotient.to_string().contains("integer overflow"),
        "{quotient}"
    );

    session
        .execute("CREATE GRAPH g ANY GRAPH; SESSION SET GRAPH g")
        .unwrap();
    session
        .run("INSERT (:N {a: 4, b: 2}), (:N {a: 4, b: 0})")
        .await
        .unwrap();
    let stored = session
        .query("MATCH (n) RETURN n.a / n.b AS q")
        .await
        .unwrap_err();
    assert!(stored.to_string().contains("division by zero"), "{stored}");
    let finite = session.query("RETURN 1.0 / 0.0 AS q").await.unwrap_err();
    assert!(finite.to_string().contains("division by zero"), "{finite}");
}

fn table(result: &QueryResult) -> Vec<Vec<String>> {
    result
        .batches
        .iter()
        .flat_map(|batch| {
            (0..batch.num_rows()).map(|row| {
                (0..batch.num_columns())
                    .map(|column| {
                        ScalarValue::try_from_array(batch.column(column), row)
                            .unwrap()
                            .to_string()
                    })
                    .collect()
            })
        })
        .collect()
}

#[tokio::test]
async fn repeated_arithmetic_orders_by_the_same_result_expression() {
    let mut session = Database::new().session();
    let result = session
        .query("FOR n IN [1, 1, 2] RETURN DISTINCT n + 1 AS value ORDER BY n + 1 DESC")
        .await
        .unwrap();
    assert_eq!(table(&result), vec![vec!["3"], vec!["2"]]);
    let result = session
        .query("FOR n IN [2, 1] RETURN DISTINCT -n AS value ORDER BY -n")
        .await
        .unwrap();
    assert_eq!(table(&result), vec![vec!["-2"], vec!["-1"]]);
    let result = session
        .query("FOR n IN [1, 1, 2] RETURN n + 1 AS value, COUNT(*) AS rows ORDER BY n + 1 DESC")
        .await
        .unwrap();
    assert_eq!(table(&result), vec![vec!["3", "1"], vec!["2", "2"]]);
    let result = session
        .query("FOR n IN [1.25, 0.25] RETURN DISTINCT n * 2.0 AS value ORDER BY n * 2.0")
        .await
        .unwrap();
    assert_eq!(table(&result), vec![vec!["0.5"], vec!["2.5"]]);
    session
        .execute("CREATE GRAPH g ANY; SESSION SET GRAPH g")
        .unwrap();
    session
        .run("INSERT (:N {v: 4}), (:N {v: 1}), (:N {v: 4})")
        .await
        .unwrap();
    let result = session
        .query("MATCH (n:N) RETURN DISTINCT n.v / 2 AS value ORDER BY n.v / 2")
        .await
        .unwrap();
    assert_eq!(table(&result), vec![vec!["0"], vec!["2"]]);
}

#[tokio::test]
async fn repeated_float_aggregate_orders_by_the_same_expression() {
    let mut session = Database::new().session();
    let result = session
        .query("FOR n IN [1, 2, 1] RETURN DISTINCT n AS k, AVG(n) AS average ORDER BY AVG(n) DESC")
        .await
        .unwrap();
    assert_eq!(table(&result), vec![vec!["2", "2"], vec!["1", "1"]]);
    let result = session
        .query("FOR n IN [4, 4] RETURN DISTINCT AVG(n) AS average ORDER BY AVG(n)")
        .await
        .unwrap();
    assert_eq!(table(&result), vec![vec!["4"]]);
    let result = session
        .query(
            "FOR n IN [1.5, 2.5, 1.5] RETURN DISTINCT n AS k, SUM(n) AS total ORDER BY SUM(n) DESC",
        )
        .await
        .unwrap();
    assert_eq!(table(&result), vec![vec!["1.5", "3"], vec!["2.5", "2.5"]]);
    let result = session
        .query(
            "FOR n IN [1, 2, 1] RETURN DISTINCT n AS k, STDDEV_POP(n) AS deviation ORDER BY STDDEV_POP(n), n",
        )
        .await
        .unwrap();
    assert_eq!(table(&result), vec![vec!["1", "0"], vec!["2", "0"]]);
    let result = session
        .query("FOR n IN [1.5, 2.5] RETURN DISTINCT MIN(n) AS value ORDER BY MIN(n)")
        .await
        .unwrap();
    assert_eq!(table(&result), vec![vec!["1.5"]]);
    let result = session
        .query(
            "FOR n IN [10, 20, 30, 40] RETURN DISTINCT PERCENTILE_CONT(n, 0.5) AS value ORDER BY PERCENTILE_CONT(n, 0.5)",
        )
        .await
        .unwrap();
    assert_eq!(table(&result), vec![vec!["25"]]);
    let result = session
        .query(
            "FOR n IN [10, 20, 30, 40] RETURN DISTINCT PERCENTILE_DISC(n, 0.5) AS value ORDER BY PERCENTILE_DISC(n, 0.5)",
        )
        .await
        .unwrap();
    assert_eq!(table(&result), vec![vec!["20"]]);
    session
        .execute("CREATE GRAPH g ANY; SESSION SET GRAPH g")
        .unwrap();
    session
        .run("INSERT (:N {k: 1, v: 10}), (:N {k: 1, v: 30}), (:N {k: 2, v: 5})")
        .await
        .unwrap();
    let result = session
        .query("MATCH (n:N) RETURN DISTINCT n.k AS k, AVG(n.v) AS average ORDER BY AVG(n.v) DESC")
        .await
        .unwrap();
    assert_eq!(table(&result), vec![vec!["1", "20"], vec!["2", "5"]]);
}

#[tokio::test]
async fn repeated_reference_expression_orders_by_the_same_value() {
    let mut session = Database::new().session();
    session
        .execute("CREATE GRAPH g ANY; SESSION SET GRAPH g")
        .unwrap();
    session
        .run(
            "INSERT (:P {name:'Ann', age:30}), (:P {name:'Ann', age:30}), (:P {name:'Bea', age:20})",
        )
        .await
        .unwrap();
    session
        .run("MATCH (a {name:'Ann'}), (b {name:'Bea'}) INSERT (a)-[:E]->(b)")
        .await
        .unwrap();
    let result = session
        .query(
            "MATCH (p:P) LET person = p RETURN DISTINCT person.name AS name ORDER BY person.name",
        )
        .await
        .unwrap();
    assert_eq!(table(&result), vec![vec!["Ann"], vec!["Bea"]]);
    let result = session
        .query(
            "MATCH (p:P) LET person = p RETURN person.name AS name, COUNT(*) AS rows ORDER BY person.name DESC",
        )
        .await
        .unwrap();
    assert_eq!(table(&result), vec![vec!["Bea", "1"], vec!["Ann", "2"]]);
    let result = session
        .query(
            "MATCH (p:P) LET person = p RETURN DISTINCT person.age AS age ORDER BY person.age DESC",
        )
        .await
        .unwrap();
    assert_eq!(table(&result), vec![vec!["30"], vec!["20"]]);
    let result = session
        .query(
            "MATCH (p:P) LET person = p RETURN DISTINCT PROPERTY_EXISTS(person, name) AS present ORDER BY PROPERTY_EXISTS(person, name)",
        )
        .await
        .unwrap();
    assert_eq!(table(&result), vec![vec!["true"]]);
    let result = session
        .query(
            "MATCH (p:P) LET person = p RETURN DISTINCT person IS LABELED P AS labeled ORDER BY person IS LABELED P",
        )
        .await
        .unwrap();
    assert_eq!(table(&result), vec![vec!["true"]]);
    let result = session
        .query(
            "MATCH (a)-[e:E]->(b) LET edge = e RETURN DISTINCT edge IS DIRECTED AS directed ORDER BY edge IS DIRECTED",
        )
        .await
        .unwrap();
    assert_eq!(table(&result), vec![vec!["true"]]);
    let result = session
        .query(
            "MATCH p = (a {name:'Ann'})-[:E]->(b) FOR x IN ELEMENTS(p) FILTER x.name IS NOT NULL RETURN DISTINCT x.name AS name ORDER BY x.name",
        )
        .await
        .unwrap();
    assert_eq!(table(&result), vec![vec!["Ann"], vec!["Bea"]]);
}

#[tokio::test]
async fn repeated_element_value_orders_by_the_same_identity() {
    let mut session = Database::new().session();
    session
        .execute("CREATE GRAPH g ANY; SESSION SET GRAPH g")
        .unwrap();
    session
        .run("INSERT (:P {name:'Ann'}), (:P {name:'Ann'}), (:P {name:'Bea'})")
        .await
        .unwrap();
    let names = |result: QueryResult, column: usize| -> Vec<String> {
        table(&result)
            .into_iter()
            .map(|row| row[column].clone())
            .collect()
    };
    let result = session
        .query("MATCH (p:P) RETURN DISTINCT p AS person, p.name AS name ORDER BY p")
        .await
        .unwrap();
    assert_eq!(names(result, 1), ["Ann", "Ann", "Bea"]);
    let result = session
        .query("MATCH (p:P) RETURN p AS person, p.name AS name, COUNT(*) AS rows ORDER BY p DESC")
        .await
        .unwrap();
    assert_eq!(
        table(&result)
            .into_iter()
            .map(|row| (row[1].clone(), row[2].clone()))
            .collect::<Vec<_>>(),
        vec![
            ("Bea".into(), "1".into()),
            ("Ann".into(), "1".into()),
            ("Ann".into(), "1".into())
        ]
    );
    session
        .run("MATCH (a {name:'Ann'}), (b {name:'Bea'}) INSERT (a)-[:E]->(b)")
        .await
        .unwrap();
    let result = session
        .query("MATCH (a)-[e:E]->(b) RETURN DISTINCT e AS edge, b.name AS dest ORDER BY e")
        .await
        .unwrap();
    assert_eq!(names(result, 1), ["Bea", "Bea"]);
    let result = session
        .query("MATCH p = (a:P) RETURN DISTINCT a.name AS name, ELEMENTS(p) AS els ORDER BY ELEMENTS(p)")
        .await
        .unwrap();
    assert_eq!(names(result, 0), ["Ann", "Ann", "Bea"]);
}

#[tokio::test]
async fn insert_star_lists_reused_nodes_once_in_pattern_order() {
    for (query, expected) in [
        ("INSERT(n:N)-[e:E]->(n) RETURN *", vec!["n", "e"]),
        (
            "INSERT(a:N)-[e:E]->(b:N)-[f:F]->(a) RETURN *",
            vec!["a", "e", "b", "f"],
        ),
    ] {
        let mut s = Database::new().session();
        s.execute("CREATE GRAPH g ANY; SESSION SET GRAPH g")
            .unwrap();
        let outputs = s.run(query).await.unwrap();
        let graphfusion::StatementOutput::Query(r) = outputs.last().unwrap() else {
            panic!("expected query result")
        };
        assert_eq!(r.row_count(), 1);
        assert_eq!(
            r.schema
                .fields()
                .iter()
                .map(|f| f.name().as_str())
                .collect::<Vec<_>>(),
            expected
        );
    }
}

fn column(result: &QueryResult) -> Vec<String> {
    result
        .batches
        .iter()
        .flat_map(|batch| {
            (0..batch.num_rows()).map(move |i| {
                let value = ScalarValue::try_from_array(batch.column(0), i).unwrap();
                if value.is_null() {
                    "NULL".into()
                } else {
                    value.to_string()
                }
            })
        })
        .collect()
}

#[tokio::test]
async fn distinct_truth_predicates_accept_nullable_booleans() {
    let mut session = Database::new().session();
    session
        .run("CREATE GRAPH g ANY GRAPH; SESSION SET GRAPH g")
        .await
        .unwrap();
    let distinct = session
        .query(
            "FOR n IN [TRUE, FALSE, NULL, TRUE]
             RETURN DISTINCT n IS TRUE AS v ORDER BY v DESC",
        )
        .await
        .unwrap();
    assert_eq!(column(&distinct), ["true", "false"]);
    for (predicate, expected) in [
        ("n IS FALSE", ["true", "false"]),
        ("n IS NOT TRUE", ["true", "false"]),
        ("n IS NOT FALSE", ["true", "false"]),
        ("NOT n IS TRUE", ["true", "false"]),
    ] {
        let result = session
            .query(&format!(
                "FOR n IN [TRUE, FALSE, NULL, TRUE]
                 RETURN DISTINCT {predicate} AS v ORDER BY v DESC"
            ))
            .await
            .unwrap();
        assert_eq!(column(&result), expected, "{predicate}");
    }
    let grouped = session
        .query(
            "FOR n IN [TRUE, FALSE, NULL, TRUE]
             RETURN n IS TRUE AS v, COUNT(*) AS c GROUP BY v ORDER BY v",
        )
        .await
        .unwrap();
    assert_eq!(
        grouped
            .batches
            .iter()
            .flat_map(|batch| (0..batch.num_rows()).map(move |i| {
                (
                    ScalarValue::try_from_array(batch.column(0), i).unwrap(),
                    ScalarValue::try_from_array(batch.column(1), i).unwrap(),
                )
            }))
            .collect::<Vec<_>>(),
        vec![
            (
                ScalarValue::Boolean(Some(false)),
                ScalarValue::Int64(Some(2))
            ),
            (
                ScalarValue::Boolean(Some(true)),
                ScalarValue::Int64(Some(2))
            )
        ]
    );
    session
        .run("INSERT (:N {flag: TRUE}), (:N {flag: FALSE}), (:N)")
        .await
        .unwrap();
    assert_eq!(
        column(
            &session
                .query(
                    "MATCH (n) WHERE n.flag IS TRUE
                     RETURN n.flag AS flag"
                )
                .await
                .unwrap()
        ),
        ["true"]
    );
    assert_eq!(
        column(
            &session
                .query(
                    "MATCH (n)
                     RETURN DISTINCT n.flag IS NOT FALSE AS known
                     ORDER BY known"
                )
                .await
                .unwrap()
        ),
        ["false", "true"]
    );
    assert_eq!(
        row(&session
            .query(
                "RETURN UNKNOWN IS NOT FALSE AS not_false,
                            NULL IS TRUE AS is_true,
                            NOT NULL IS TRUE AS not_is_true"
            )
            .await
            .unwrap()),
        vec![
            ScalarValue::Boolean(Some(true)),
            ScalarValue::Boolean(Some(false)),
            ScalarValue::Boolean(Some(true))
        ]
    );
}

#[tokio::test]
async fn decimal_parameters_reject_small_excess_fractional_digits() {
    let mut s = Database::new().session();
    for (ty, value) in [
        ("DECIMAL(3,0)", "0.000000001"),
        ("DECIMAL(4,2)", "1.230000001"),
        ("DECIMAL(4,2)", "99.999999999"),
    ] {
        let q = format!("SESSION SET VALUE $bad {ty} = {value}");
        assert!(
            matches!(s.execute(&q), Err(Error::InvalidDefinition(_))),
            "{q}"
        );
    }
}

#[tokio::test]
async fn numeric_parameters_enforce_declared_precision_and_scale() {
    let mut s = Database::new().session();
    for q in [
        "SESSION SET VALUE $bad NUMERIC(2,1) = 12.3",
        "SESSION SET VALUE $bad NUMERIC(2,1) = 1.23",
        "SESSION SET VALUE $bad NUMERIC(0) = 0",
        "SESSION SET VALUE $bad NUMERIC(1,2) = 0",
    ] {
        assert!(s.execute(q).is_err(), "{q}");
    }
    s.execute("SESSION SET VALUE $a NUMERIC(2,1) = 0.1; SESSION SET VALUE $b DECIMAL(5,2) = -10.25; SESSION SET VALUE $z DECIMAL(2,2) = 0.0").unwrap();
    assert_eq!(
        row(&s.query("RETURN $a AS a,$b AS b,$z AS z").await.unwrap()),
        vec![
            ScalarValue::Float64(Some(0.1)),
            ScalarValue::Float64(Some(-10.25)),
            ScalarValue::Float64(Some(0.0))
        ]
    );
}
