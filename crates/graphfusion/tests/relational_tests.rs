use datafusion::{arrow::datatypes::DataType, common::ScalarValue};
use graphfusion::{Database, QueryResult, Session, StatementOutput, Value};

fn rows(result: &QueryResult) -> Vec<Vec<String>> {
    result
        .batches
        .iter()
        .flat_map(|batch| {
            (0..batch.num_rows()).map(move |i| {
                batch
                    .columns()
                    .iter()
                    .map(|c| {
                        let v = ScalarValue::try_from_array(c, i).unwrap();
                        if v.is_null() {
                            "NULL".into()
                        } else {
                            v.to_string()
                        }
                    })
                    .collect()
            })
        })
        .collect()
}
async fn query(s: &mut Session, sql: &str) -> QueryResult {
    s.query(sql).await.unwrap_or_else(|e| panic!("{sql}\n{e}"))
}
async fn fixture() -> Session {
    let mut s = Database::new().session();
    s.run("CREATE GRAPH g ANY GRAPH; SESSION SET GRAPH g; INSERT (a:N {v: 1, k: 'a'})-[:E]->(b:N {v: 2, k: 'a'}), (:N {v: 3, k: 'b'})").await.unwrap();
    s
}
async fn check(s: &mut Session, sql: &str, expected: &[&[&str]]) {
    assert_eq!(rows(&query(s, sql).await), expected, "{sql}");
}

#[tokio::test]
async fn optional_preserves_duplicates_and_null_element_predicates() {
    let mut s = fixture().await;
    check(&mut s, "FOR x IN [1, 1] MATCH (a:N) OPTIONAL MATCH (a)-[e:E]->(b) RETURN a.v AS a, b.v AS b, COUNT(e) AS c GROUP BY a, b ORDER BY a",
        &[&["1", "2", "2"], &["2", "NULL", "0"], &["3", "NULL", "0"]]).await;
    check(&mut s, "MATCH (a:N {v: 3}) OPTIONAL MATCH (a)-[e:E]->(b) RETURN b IS NULL AS missing, ELEMENT_ID(b) AS id, b IS LABELED N AS label, PROPERTY_EXISTS(b, v) AS property, SAME(a, b) AS same, e IS DIRECTED AS directed",
        &[&["true", "NULL", "NULL", "NULL", "NULL", "NULL"]]).await;
    check(
        &mut s,
        "MATCH (a:N {v: 3}) OPTIONAL MATCH (a)-[]->(b) MATCH (b) RETURN COUNT(*) AS n",
        &[&["0"]],
    )
    .await;
}

#[tokio::test]
async fn optional_filters_and_blocks_pad_the_whole_new_binding() {
    let mut s = fixture().await;
    check(&mut s, "MATCH (a:N) OPTIONAL MATCH (a)-[e:E]->(b) WHERE b.v > 9 RETURN a.v AS a, ELEMENT_ID(e) AS e, b.v AS b ORDER BY a",
        &[&["1", "NULL", "NULL"], &["2", "NULL", "NULL"], &["3", "NULL", "NULL"]]).await;
    check(&mut s, "MATCH (a:N) OPTIONAL { MATCH (a)-[e:E]->(b) MATCH (b)-[]->(c) } RETURN a.v AS a, b.v AS b, c.v AS c ORDER BY a",
        &[&["1", "NULL", "NULL"], &["2", "NULL", "NULL"], &["3", "NULL", "NULL"]]).await;
    check(
        &mut s,
        "OPTIONAL MATCH (n:Absent) RETURN COUNT(*) AS rows, COUNT(n) AS nodes",
        &[&["1", "0"]],
    )
    .await;
    check(
        &mut s,
        "MATCH (n:Absent) OPTIONAL MATCH (m) RETURN COUNT(*) AS n",
        &[&["0"]],
    )
    .await;
    check(
        &mut s,
        "MATCH (n:N) OPTIONAL MATCH (n) WHERE n.v > 1 RETURN n.v AS v ORDER BY v",
        &[&["1"], &["2"], &["3"]],
    )
    .await;
}

#[tokio::test]
async fn disconnected_patterns_keep_bag_cardinality() {
    let mut s = fixture().await;
    check(
        &mut s,
        "MATCH (a:N), (b:N) RETURN COUNT(*) AS c, COUNT(DISTINCT a) AS nodes",
        &[&["9", "3"]],
    )
    .await;
    check(&mut s, "MATCH (a:N) OPTIONAL MATCH (b:Absent), (c:N) RETURN COUNT(*) AS c, COUNT(b) AS b, COUNT(c) AS c2", &[&["3", "0", "0"]]).await;
}

#[tokio::test]
async fn for_lists_indices_nulls_and_parameters() {
    let mut s = Database::new().session();
    check(
        &mut s,
        "FOR a IN [9, 9] FOR x IN [2, NULL, 2] WITH ORDINALITY i RETURN a, x, i ORDER BY i",
        &[
            &["9", "2", "1"],
            &["9", "2", "1"],
            &["9", "NULL", "2"],
            &["9", "NULL", "2"],
            &["9", "2", "3"],
            &["9", "2", "3"],
        ],
    )
    .await;
    check(
        &mut s,
        "FOR x IN [8, 6] WITH OFFSET i RETURN x, i ORDER BY i",
        &[&["8", "0"], &["6", "1"]],
    )
    .await;
    for source in ["[]", "NULL"] {
        check(
            &mut s,
            &format!("FOR x IN {source} RETURN COUNT(*) AS c"),
            &[&["0"]],
        )
        .await;
    }
    check(
        &mut s,
        "FOR xs IN [[1, 2], [3, 4]] FOR x IN xs RETURN x ORDER BY x",
        &[&["1"], &["2"], &["3"], &["4"]],
    )
    .await;
    s.set_parameter(
        "values",
        Value::List(vec![Value::Integer(4), Value::Null, Value::Integer(4)]),
    )
    .unwrap();
    check(
        &mut s,
        "FOR x IN $values RETURN COUNT(*) AS n, COUNT(x) AS nonnull, COUNT(DISTINCT x) AS d",
        &[&["3", "2", "1"]],
    )
    .await;
    for source in ["1", "[1, '2']", "[TRUE, 1]"] {
        assert!(
            s.query(&format!("FOR x IN {source} RETURN x"))
                .await
                .is_err(),
            "{source}"
        );
    }
    s.set_parameter(
        "values",
        Value::List(vec![Value::Integer(4), Value::String("4".into())]),
    )
    .unwrap();
    assert!(s.query("FOR x IN $values RETURN x").await.is_err());
}

#[tokio::test]
async fn byte_lists_keep_null_elements() {
    let mut s = Database::new().session();
    let result = query(
        &mut s,
        "FOR x IN [X'0A', NULL, X'0B'] WITH ORDINALITY i RETURN i, x ORDER BY i",
    )
    .await;
    assert_eq!(result.schema.field(1).data_type(), &DataType::Binary);
    assert_eq!(
        rows(&result),
        vec![
            vec!["1".to_string(), "0A".to_string()],
            vec!["2".to_string(), "NULL".to_string()],
            vec!["3".to_string(), "0B".to_string()],
        ]
    );
    check(
        &mut s,
        "FOR x IN [NULL, X'FF'] RETURN x",
        &[&["NULL"], &["FF"]],
    )
    .await;
    check(
        &mut s,
        "FOR xs IN [[X'0A', NULL], [X'0B']] FOR x IN xs RETURN x",
        &[&["0A"], &["NULL"], &["0B"]],
    )
    .await;
    s.execute("SESSION SET VALUE $xs = [X'0A', NULL]; SESSION SET VALUE $typed LIST<BYTES> = [NULL, X'FF']").unwrap();
    check(&mut s, "FOR x IN $xs RETURN x", &[&["0A"], &["NULL"]]).await;
    check(&mut s, "FOR x IN $typed RETURN x", &[&["NULL"], &["FF"]]).await;
    assert!(s.query("RETURN [X'0A', 'text'] AS bad").await.is_err());
}

#[tokio::test]
async fn nested_lists_unify_empty_null_and_numeric_elements() {
    let mut s = Database::new().session();
    for source in [
        "[[1], [], [NULL], [2.5]]",
        "[[], [1], [2.5], [NULL]]",
        "[NULL, [1], [2.5], [], [NULL]]",
    ] {
        let result = query(
            &mut s,
            &format!("FOR xs IN {source} FOR x IN xs RETURN x ORDER BY x"),
        )
        .await;
        assert_eq!(result.schema.field(0).data_type(), &DataType::Float64);
        assert_eq!(rows(&result), vec![vec!["1"], vec!["2.5"], vec!["NULL"]]);
    }
    check(
        &mut s,
        "FOR xs IN [[[1]], [[]], [[NULL]]] FOR ys IN xs FOR x IN ys RETURN x ORDER BY x",
        &[&["1"], &["NULL"]],
    )
    .await;
    s.set_parameter(
        "nested",
        Value::List(vec![
            Value::List(vec![Value::Integer(1)]),
            Value::List(vec![]),
            Value::List(vec![Value::Null]),
            Value::List(vec![Value::Float(2.5)]),
        ]),
    )
    .unwrap();
    check(
        &mut s,
        "FOR xs IN $nested FOR x IN xs RETURN x ORDER BY x",
        &[&["1"], &["2.5"], &["NULL"]],
    )
    .await;
    s.execute("SESSION SET VALUE $typed LIST<LIST<FLOAT>> = [[1], [], [NULL], [2.5]]")
        .unwrap();
    check(
        &mut s,
        "FOR xs IN $typed FOR x IN xs RETURN x ORDER BY x",
        &[&["1"], &["2.5"], &["NULL"]],
    )
    .await;
    for source in ["[[1], [], ['2']]", "[[TRUE], [NULL], [1]]", "[[1], [] , 2]"] {
        assert!(s.query(&format!("RETURN {source} AS xs")).await.is_err());
    }
}

#[tokio::test]
async fn integer_avg_divides_the_exact_sum() {
    let mut session = Database::new().session();
    let averaged = query(
        &mut session,
        "FOR x IN [9007199254740993, 1] RETURN AVG(x) AS a",
    )
    .await;
    assert_eq!(
        ScalarValue::try_from_array(averaged.batches[0].column(0), 0).unwrap(),
        ScalarValue::Float64(Some(4503599627370497.0))
    );
    check(
        &mut session,
        "FOR x IN [1, 2, NULL] RETURN AVG(x) AS a",
        &[&["1.5"]],
    )
    .await;
    check(
        &mut session,
        "FOR x IN [1, 1, 2] RETURN AVG(DISTINCT x) AS a",
        &[&["1.5"]],
    )
    .await;
    check(
        &mut session,
        "FOR x IN [NULL, NULL] RETURN AVG(x) AS a",
        &[&["NULL"]],
    )
    .await;
    let thirds = query(&mut session, "FOR x IN [1, 1, 2] RETURN AVG(x) AS a").await;
    assert_eq!(
        ScalarValue::try_from_array(thirds.batches[0].column(0), 0).unwrap(),
        ScalarValue::Float64(Some(4.0 / 3.0))
    );
    check(
        &mut session,
        "FOR x IN [9007199254740993, 1, 10, 20]
         RETURN x < 100 AS small, AVG(x) AS average
         GROUP BY small ORDER BY small",
        &[
            &["false", "9007199254740992"],
            &["true", "10.333333333333334"],
        ],
    )
    .await;

    session
        .execute("CREATE GRAPH g ANY GRAPH; SESSION SET GRAPH g")
        .unwrap();
    session
        .run("INSERT (:N {v: 9007199254740993}), (:N {v: 1})")
        .await
        .unwrap();
    let stored = query(&mut session, "MATCH (n) RETURN AVG(n.v) AS a").await;
    assert_eq!(
        ScalarValue::try_from_array(stored.batches[0].column(0), 0).unwrap(),
        ScalarValue::Float64(Some(4503599627370497.0))
    );
}

#[tokio::test]
async fn aggregate_grouping_aliases_having_and_order() {
    let mut s = fixture().await;
    check(&mut s, "MATCH (n:N) RETURN n.k AS k, COUNT(*) AS c, SUM(n.v) AS sum, AVG(n.v) AS avg, MIN(n.v) AS min, MAX(n.v) AS max ORDER BY k",
        &[&["a", "2", "3", "1.5", "1", "2"], &["b", "1", "3", "3", "3", "3"]]).await;
    check(
        &mut s,
        "SELECT n.k AS k, COUNT(*) AS c FROM g MATCH (n:N) GROUP BY k HAVING c > 1 ORDER BY c DESC",
        &[&["a", "2"]],
    )
    .await;
    check(
        &mut s,
        "MATCH (n:N) RETURN n.k AS k GROUP BY k ORDER BY SUM(n.v) DESC, k",
        &[&["a"], &["b"]],
    )
    .await;
    check(
        &mut s,
        "MATCH (n:N) RETURN n.v AS v, COUNT(*) AS c GROUP BY n ORDER BY v",
        &[&["1", "1"], &["2", "1"], &["3", "1"]],
    )
    .await;
    check(
        &mut s,
        "FOR x IN [NULL, 1, NULL, 1, 2] RETURN x, COUNT(*) AS c ORDER BY x NULLS FIRST",
        &[&["NULL", "2"], &["1", "2"], &["2", "1"]],
    )
    .await;
    check(
        &mut s,
        "FOR x IN [1, 1, 2] RETURN SUM(DISTINCT x) AS s, COUNT(DISTINCT x) AS c",
        &[&["3", "2"]],
    )
    .await;
}

#[tokio::test]
async fn empty_groups_and_collect_have_consistent_null_behavior() {
    let mut s = fixture().await;
    check(&mut s, "MATCH (n:Absent) RETURN COUNT(*) AS c, SUM(n.v) AS s, AVG(n.v) AS a, MIN(n.v) AS mn, MAX(n.v) AS mx, COLLECT_LIST(n.v) AS xs",
        &[&["0", "NULL", "NULL", "NULL", "NULL", "[]"]]).await;
    check(
        &mut s,
        "MATCH (n:Absent) RETURN 7 AS v, COUNT(*) AS c",
        &[&["7", "0"]],
    )
    .await;
    check(
        &mut s,
        "MATCH (n:Absent) RETURN n.k AS k, COUNT(*) AS c",
        &[],
    )
    .await;
    // A property no fragment stores is a per-row null, not a query constant.
    check(
        &mut s,
        "MATCH (n:N) RETURN n.missing AS m, COUNT(*) AS c",
        &[&["NULL", "3"]],
    )
    .await;
    check(
        &mut s,
        "MATCH (n:N) WHERE n.v = -1 RETURN n.missing AS m, COUNT(*) AS c",
        &[],
    )
    .await;
    check(
        &mut s,
        "MATCH (n:Absent) RETURN n.missing AS m, COUNT(*) AS c",
        &[],
    )
    .await;
    check(
        &mut s,
        "MATCH (n:Absent) RETURN NULL AS m, COUNT(*) AS c",
        &[&["NULL", "0"]],
    )
    .await;
    check(
        &mut s,
        "MATCH (n:N) RETURN COALESCE(n.missing, 0) AS m, COUNT(*) AS c",
        &[&["0", "3"]],
    )
    .await;
    check(
        &mut s,
        "MATCH (n:Absent) RETURN COALESCE(n.missing, 0) AS m, COUNT(*) AS c",
        &[],
    )
    .await;
    check(
        &mut s,
        "SELECT n.missing AS m, COUNT(*) AS c FROM g MATCH (n:N) WHERE n.v = -1",
        &[],
    )
    .await;
    check(
        &mut s,
        "MATCH ()-[e:E]->() RETURN e.missing AS m, COUNT(*) AS c",
        &[&["NULL", "1"]],
    )
    .await;
    check(
        &mut s,
        "MATCH (n:Absent) RETURN n.missing AS m, COUNT(*) AS c GROUP BY ()",
        &[&["NULL", "0"]],
    )
    .await;
    check(
        &mut s,
        "MATCH (n:Absent) RETURN 7 AS v GROUP BY ()",
        &[&["7"]],
    )
    .await;
    check(
        &mut s,
        "FOR x IN [NULL, NULL] RETURN COUNT(x) AS c, COLLECT_LIST(x) AS xs",
        &[&["0", "[]"]],
    )
    .await;
    let collected = rows(
        &query(
            &mut s,
            "FOR x IN [2, NULL, 1, 2] RETURN COLLECT_LIST(DISTINCT x) AS xs",
        )
        .await,
    );
    assert!(collected == [vec!["[1, 2]"]] || collected == [vec!["[2, 1]"]]);
}

#[tokio::test]
async fn ungrouped_references_are_invalid_queries() {
    let mut s = fixture().await;
    for sql in [
        "FOR n IN [1, 2, 3] RETURN COLLECT_LIST(n) AS xs ORDER BY n DESC",
        "MATCH (n:N) RETURN COUNT(*) AS c ORDER BY n.k",
        "SELECT COUNT(*) AS c FROM g MATCH (n:N) HAVING n.v > 0",
        "FOR x IN [1, 2] RETURN x, COUNT(*) AS c GROUP BY ()",
    ] {
        let error = s.query(sql).await.unwrap_err().to_string();
        assert!(
            error.contains("invalid query: expression is not grouped or aggregated"),
            "{sql}: {error}"
        );
        assert!(!error.contains("__gf_"), "{sql}: {error}");
    }
    check(
        &mut s,
        "FOR n IN [1, 2, 2] RETURN n AS n, COUNT(*) AS c ORDER BY n",
        &[&["1", "1"], &["2", "2"]],
    )
    .await;
    check(
        &mut s,
        "FOR n IN [1, 2, 3] RETURN COLLECT_LIST(n) AS xs ORDER BY COUNT(*)",
        &[&["[1, 2, 3]"]],
    )
    .await;
}

#[tokio::test]
async fn aggregate_errors_are_checked_and_never_publish_writes() {
    let mut s = fixture().await;
    for sql in [
        "RETURN SUM(COUNT(*)) AS x",
        "MATCH (n) WHERE COUNT(*) > 1 RETURN n.v AS v",
        "FOR x IN [1, 2] RETURN x, COUNT(*) AS c GROUP BY ()",
        "FOR x IN [1, 2] RETURN COUNT(*) AS c GROUP BY c",
        "RETURN SUM('text') AS x",
        "FOR x IN [9223372036854775807, 1] RETURN SUM(x) AS x",
    ] {
        assert!(s.query(sql).await.is_err(), "{sql}");
    }
    assert!(s
        .run("INSERT (:N {v: 9}) FOR x IN [9223372036854775807, 1] RETURN SUM(x) AS x")
        .await
        .is_err());
    check(&mut s, "MATCH (n:N) RETURN COUNT(*) AS c", &[&["3"]]).await;
}

#[tokio::test]
async fn statistical_aggregates_use_datafusion_and_percentile_is_fractional() {
    let mut s = Database::new().session();
    check(
        &mut s,
        "FOR x IN [1, 3] RETURN STDDEV_POP(x) AS sd, PERCENTILE_CONT(x, 0.25) AS p",
        &[&["1", "1.5"]],
    )
    .await;
    assert!(s
        .query("FOR x IN [1, 2] RETURN PERCENTILE_CONT(x, x) AS p")
        .await
        .is_err());
    assert!(s
        .query("RETURN PERCENTILE_CONT(1, 2.0) AS p")
        .await
        .is_err());
}

#[tokio::test]
async fn integer_stddev_uses_exact_moments() {
    let mut s = Database::new().session();
    let result = query(
        &mut s,
        "FOR x IN [9007199254740993, NULL, 1] RETURN STDDEV_POP(x) AS p, STDDEV_SAMP(x) AS s",
    )
    .await;
    assert_eq!(result.schema.field(0).data_type(), &DataType::Float64);
    assert_eq!(
        float_at(&result, 0, 0).to_bits(),
        4503599627370496.0f64.to_bits()
    );
    assert_eq!(float_at(&result, 0, 1).to_bits(), 0x4336a09e667f3bcd);
    check(
        &mut s,
        "FOR x IN [1, 2, 3, 4] RETURN STDDEV_POP(x) AS p, STDDEV_SAMP(x) AS s",
        &[&["1.118033988749895", "1.2909944487358056"]],
    )
    .await;
    check(
        &mut s,
        "FOR x IN [10, 20, 30] RETURN STDDEV_POP(x) AS p, STDDEV_SAMP(x) AS s",
        &[&["8.16496580927726", "10"]],
    )
    .await;
    check(
        &mut s,
        "FOR x IN [1, NULL] RETURN STDDEV_POP(x) AS p, STDDEV_SAMP(x) AS s",
        &[&["0", "NULL"]],
    )
    .await;
    check(
        &mut s,
        "FOR x IN [NULL, NULL] RETURN STDDEV_POP(x) AS p, STDDEV_SAMP(x) AS s",
        &[&["NULL", "NULL"]],
    )
    .await;
    check(
        &mut s,
        "FOR x IN [-2, 2] RETURN STDDEV_POP(x) AS p, STDDEV_SAMP(x) AS s",
        &[&["2", "2.8284271247461903"]],
    )
    .await;
    check(
        &mut s,
        "FOR x IN [1.0, 3.0] RETURN STDDEV_POP(x) AS p",
        &[&["1"]],
    )
    .await;
    let distinct = query(
        &mut s,
        "FOR x IN [9007199254740993, 9007199254740993, 1] RETURN STDDEV_POP(DISTINCT x) AS d, STDDEV_POP(x) AS a",
    )
    .await;
    assert_eq!(
        float_at(&distinct, 0, 0).to_bits(),
        4503599627370496.0f64.to_bits()
    );
    assert_eq!(float_at(&distinct, 0, 1).to_bits(), 0x432e2b7dddfefa66);
    let grouped = query(
        &mut s,
        "FOR x IN [9007199254740993, 1, 4, 4] RETURN x = 4 AS four, STDDEV_POP(x) AS p GROUP BY four ORDER BY four",
    )
    .await;
    assert_eq!(scalar_at(&grouped, 0, 0), ScalarValue::Boolean(Some(false)));
    assert_eq!(
        float_at(&grouped, 0, 1).to_bits(),
        4503599627370496.0f64.to_bits()
    );
    assert_eq!(scalar_at(&grouped, 1, 0), ScalarValue::Boolean(Some(true)));
    assert_eq!(float_at(&grouped, 1, 1), 0.0);
    s.run("CREATE GRAPH g ANY GRAPH; SESSION SET GRAPH g; INSERT (:N {v: 9007199254740993}), (:N {v: 1})")
        .await
        .unwrap();
    let graph = query(&mut s, "MATCH (n) RETURN STDDEV_POP(n.v) AS p").await;
    assert_eq!(
        float_at(&graph, 0, 0).to_bits(),
        4503599627370496.0f64.to_bits()
    );
}

fn scalar_at(result: &QueryResult, row: usize, column: usize) -> ScalarValue {
    let mut seen = 0;
    for batch in &result.batches {
        if row < seen + batch.num_rows() {
            return ScalarValue::try_from_array(batch.column(column), row - seen).unwrap();
        }
        seen += batch.num_rows();
    }
    panic!("missing row {row}");
}

fn float_at(result: &QueryResult, row: usize, column: usize) -> f64 {
    match scalar_at(result, row, column) {
        ScalarValue::Float64(Some(value)) => value,
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn distinct_standard_deviation_ignores_repeated_numbers() {
    let mut s = Database::new().session();
    check(
        &mut s,
        "FOR x IN [1, 2, 2, 1] RETURN STDDEV_POP(DISTINCT x) AS p, STDDEV_SAMP(DISTINCT x) AS s, STDDEV_POP(ALL x) AS all_p",
        &[&["0.5", "0.7071067811865476", "0.5"]],
    )
    .await;
    check(
        &mut s,
        "FOR x IN [10, 30, 20, 10, NULL] RETURN STDDEV_POP(DISTINCT x) AS p, STDDEV_SAMP(DISTINCT x) AS s",
        &[&["8.16496580927726", "10"]],
    )
    .await;
    check(
        &mut s,
        "FOR x IN [4, 4, NULL] RETURN STDDEV_POP(DISTINCT x) AS p, STDDEV_SAMP(DISTINCT x) AS s",
        &[&["0", "NULL"]],
    )
    .await;
    check(
        &mut s,
        "FOR x IN [NULL, NULL] RETURN STDDEV_POP(DISTINCT x) AS p, STDDEV_SAMP(DISTINCT x) AS s",
        &[&["NULL", "NULL"]],
    )
    .await;
    s.run("CREATE GRAPH g ANY GRAPH; SESSION SET GRAPH g; INSERT (:N {k:'a', v:1}), (:N {k:'a', v:1}), (:N {k:'a', v:3}), (:N {k:'b', v:10}), (:N {k:'b', v:20}), (:N {k:'b', v:30}), (:N {k:'b', v:10})").await.unwrap();
    check(
        &mut s,
        "MATCH (n) RETURN n.k AS k, STDDEV_POP(DISTINCT n.v) AS p GROUP BY k ORDER BY k",
        &[&["a", "1"], &["b", "8.16496580927726"]],
    )
    .await;
}

#[tokio::test]
async fn composite_all_counts_each_null_and_duplicate_occurrence() {
    let mut s = Database::new().session();
    for (op, expected) in [
        (
            "UNION ALL",
            vec!["NULL", "NULL", "NULL", "1", "1", "1", "1", "1", "2", "3"],
        ),
        ("UNION", vec!["NULL", "1", "2", "3"]),
        ("INTERSECT ALL", vec!["NULL", "1", "1"]),
        ("INTERSECT", vec!["NULL", "1"]),
        ("EXCEPT ALL", vec!["NULL", "2"]),
        ("EXCEPT", vec!["2"]),
    ] {
        let sql = format!(
            "FOR x IN [NULL, NULL, 1, 1, 2] RETURN x {op} FOR x IN [NULL, 1, 1, 1, 3] RETURN x"
        );
        let result = query(&mut s, &sql).await;
        let mut actual: Vec<_> = rows(&result).into_iter().map(|r| r[0].clone()).collect();
        actual.sort();
        let mut expected = expected;
        expected.sort();
        assert_eq!(actual, expected, "{sql}");
        if op.ends_with(" ALL") && !op.starts_with("UNION") {
            assert!(result.logical_plan.contains("row_number"));
        }
    }
    check(
        &mut s,
        "FOR x IN [1, 1, 1] RETURN x EXCEPT ALL RETURN 1 AS x EXCEPT ALL RETURN 1 AS x",
        &[&["1"]],
    )
    .await;
}

#[tokio::test]
async fn composite_schema_names_types_and_scopes_are_checked() {
    let mut s = Database::new().session();
    let result = query(
        &mut s,
        "RETURN 1 AS x UNION ALL RETURN 1.5 AS x UNION ALL RETURN NULL AS x",
    )
    .await;
    assert_eq!(result.schema.field(0).data_type(), &DataType::Float64);
    assert_eq!(result.row_count(), 3);
    for sql in [
        "RETURN 1 AS a UNION RETURN 1 AS b",
        "RETURN 1 AS a UNION RETURN 1 AS a, 2 AS b",
        "RETURN 1 AS a UNION RETURN '1' AS a",
        "LET x = 1 RETURN x UNION RETURN x",
        "RETURN 1 AS x OTHERWISE RETURN 1 AS y",
        "RETURN 1 AS x OTHERWISE RETURN missing AS x",
    ] {
        assert!(s.query(sql).await.is_err(), "{sql}");
    }
}

#[tokio::test]
async fn composite_lists_widen_numbers_and_accept_empty_lists() {
    let mut s = Database::new().session();
    let result = query(
        &mut s,
        "RETURN [1, 2] AS x UNION ALL RETURN [1.0, 2.0] AS x UNION ALL RETURN [] AS x",
    )
    .await;
    let DataType::List(field) = result.schema.field(0).data_type() else {
        panic!("list column");
    };
    assert_eq!(field.data_type(), &DataType::Float64);
    let mut actual: Vec<_> = rows(&result).into_iter().map(|r| r[0].clone()).collect();
    actual.sort();
    assert_eq!(actual, ["[1.0, 2.0]", "[1.0, 2.0]", "[]"]);
    check(
        &mut s,
        "RETURN [1] AS x UNION RETURN [1.0] AS x",
        &[&["[1.0]"]],
    )
    .await;
    check(&mut s, "RETURN [1] AS x EXCEPT RETURN [1.0] AS x", &[]).await;
    check(&mut s, "RETURN [1] AS x EXCEPT RETURN [] AS x", &[&["[1]"]]).await;
    check(
        &mut s,
        "RETURN [1] AS x INTERSECT RETURN [1.0] AS x",
        &[&["[1.0]"]],
    )
    .await;
    check(
        &mut s,
        "FOR x IN [[1], [1]] RETURN x INTERSECT ALL FOR x IN [[1.0]] RETURN x",
        &[&["[1.0]"]],
    )
    .await;
    check(
        &mut s,
        "RETURN [[1], []] AS x UNION RETURN [[1.0], []] AS x",
        &[&["[[1.0], []]"]],
    )
    .await;
    check(
        &mut s,
        "RETURN [1] AS x OTHERWISE RETURN [1.0] AS x",
        &[&["[1.0]"]],
    )
    .await;
    check(
        &mut s,
        "FILTER FALSE RETURN [] AS x OTHERWISE RETURN [2] AS x",
        &[&["[2]"]],
    )
    .await;
    let result = query(
        &mut s,
        "RETURN 1 AS n, [1] AS xs UNION ALL RETURN 1.5 AS n, [] AS xs",
    )
    .await;
    assert_eq!(result.schema.field(0).data_type(), &DataType::Float64);
    let DataType::List(field) = result.schema.field(1).data_type() else {
        panic!("list column");
    };
    assert_eq!(field.data_type(), &DataType::Int64);
    let mut actual: Vec<_> = rows(&result)
        .into_iter()
        .map(|row| (row[0].clone(), row[1].clone()))
        .collect();
    actual.sort();
    assert_eq!(
        actual,
        vec![("1".into(), "[1]".into()), ("1.5".into(), "[]".into())]
    );
    for sql in [
        "RETURN [1] AS x UNION RETURN ['a'] AS x",
        "RETURN [TRUE] AS x UNION ALL RETURN [1] AS x",
        "RETURN [[1]] AS x OTHERWISE RETURN [['a']] AS x",
    ] {
        assert!(s.query(sql).await.is_err(), "{sql}");
    }
}

#[tokio::test]
async fn otherwise_uses_first_nonempty_branch_and_skips_cold_runtime_errors() {
    let mut s = fixture().await;
    check(
        &mut s,
        "FILTER FALSE RETURN 1 AS x OTHERWISE RETURN NULL AS x OTHERWISE RETURN 2 AS x",
        &[&["NULL"]],
    )
    .await;
    check(
        &mut s,
        "RETURN 1 AS x OTHERWISE RETURN 1 / 0 AS x",
        &[&["1"]],
    )
    .await;
    check(
        &mut s,
        "RETURN 1 AS x OTHERWISE LET bad = 1 / 0 OPTIONAL MATCH (n:Absent) RETURN bad AS x",
        &[&["1"]],
    )
    .await;
    check(
        &mut s,
        "FILTER FALSE RETURN 1 AS x OTHERWISE FILTER FALSE RETURN 2 AS x",
        &[],
    )
    .await;
    assert!(s
        .query("FILTER FALSE RETURN 1 AS x OTHERWISE RETURN 1 / 0 AS x")
        .await
        .is_err());
    assert!(s
        .run("RETURN 1 AS x OTHERWISE INSERT (:N {v: 99}) RETURN 2 AS x")
        .await
        .is_err());
    check(&mut s, "MATCH (n:N) RETURN COUNT(*) AS c", &[&["3"]]).await;
}

#[tokio::test]
async fn otherwise_keeps_the_graph_of_the_branch_that_produced_rows() {
    let mut s = Database::new().session();
    s.run("CREATE GRAPH g1 ANY; CREATE GRAPH g2 ANY; SESSION SET GRAPH g1; INSERT (:N {name:'one'}); SESSION SET GRAPH g2; INSERT (:N {name:'two'})").await.unwrap();
    check(
        &mut s,
        "USE g1 MATCH (n) WHERE n.name = 'zzz' RETURN n.name AS name OTHERWISE USE g2 MATCH (n) RETURN n.name AS name NEXT MATCH (m) RETURN name AS prev, m.name AS m",
        &[&["two", "two"]],
    )
    .await;
    check(
        &mut s,
        "USE g1 MATCH (n) RETURN n.name AS name OTHERWISE USE g2 MATCH (n) RETURN n.name AS name NEXT MATCH (m) RETURN name AS prev, m.name AS m",
        &[&["one", "one"]],
    )
    .await;
    check(
        &mut s,
        "USE g1 MATCH (n) WHERE n.name = 'zzz' RETURN 'fallback' AS name OTHERWISE USE g2 RETURN 'from2' AS name NEXT MATCH (m) RETURN name AS prev, m.name AS m",
        &[&["from2", "two"]],
    )
    .await;
}

#[tokio::test]
async fn select_from_graphs_and_derived_composites() {
    let mut s = fixture().await;
    check(
        &mut s,
        "SELECT n.v AS v FROM g MATCH (n:N) WHERE n.v > 1 ORDER BY v",
        &[&["2"], &["3"]],
    )
    .await;
    check(&mut s, "SELECT x, COUNT(*) AS c FROM { RETURN 1 AS x UNION ALL RETURN 1 AS x UNION ALL RETURN 2 AS x } GROUP BY x HAVING c > 1", &[&["1", "2"]]).await;
    check(
        &mut s,
        "SELECT * FROM g { MATCH (n:N) RETURN n.v AS v } WHERE v > 1 ORDER BY v DESC",
        &[&["3"], &["2"]],
    )
    .await;
    check(
        &mut s,
        "SELECT COUNT(*) AS c FROM g MATCH (a:N) WHERE TRUE, g MATCH (b:N)",
        &[&["9"]],
    )
    .await;
    assert!(s
        .query("SELECT x FROM { INSERT (:N) RETURN 1 AS x }")
        .await
        .is_err());
}

#[tokio::test]
async fn optional_mutation_null_targets_are_noops_without_losing_rows() {
    let mut s = fixture().await;
    let outputs = s.run("MATCH (a:N) OPTIONAL MATCH (a)-[e:E]->(b) SET b.v = 20 RETURN a.v AS a, b.v AS b ORDER BY a").await.unwrap();
    let StatementOutput::Query(result) = &outputs[0] else {
        panic!()
    };
    assert_eq!(result.affected_elements, 1);
    assert_eq!(
        rows(result),
        &[vec!["1", "20"], vec!["3", "NULL"], vec!["20", "NULL"]]
    );
    let outputs = s
        .run("MATCH (a:N) OPTIONAL MATCH (a)-[e:E]->(b) DELETE e RETURN a.v AS a ORDER BY a")
        .await
        .unwrap();
    let StatementOutput::Query(result) = &outputs[0] else {
        panic!()
    };
    assert_eq!(result.affected_elements, 1);
    assert_eq!(rows(result), &[vec!["1"], vec!["3"], vec!["20"]]);
    let outputs = s.run("MATCH (a:N) OPTIONAL MATCH (a)-[e:E]->(b) SET e.v = 1 REMOVE e.v DELETE e RETURN a.v AS a ORDER BY a").await.unwrap();
    let StatementOutput::Query(result) = &outputs[0] else {
        panic!()
    };
    assert_eq!(result.affected_elements, 0);
    assert_eq!(rows(result), &[vec!["1"], vec!["3"], vec!["20"]]);
    let err = s
        .run("OPTIONAL MATCH (b:Absent) INSERT (:N {v: 9})-[:E]->(b)")
        .await
        .unwrap_err();
    assert!(err.to_string().contains("null INSERT endpoint"), "{err}");
    check(&mut s, "MATCH (n:N) RETURN COUNT(*) AS c", &[&["3"]]).await;
    let mut t = fixture().await;
    let err = t
        .run("MATCH (a:N) OPTIONAL MATCH (a)-[:E]->(b) INSERT (a)-[:F]->(b)")
        .await
        .unwrap_err();
    assert!(err.to_string().contains("null INSERT endpoint"), "{err}");
    check(&mut t, "MATCH ()-[e:F]->() RETURN COUNT(*) AS c", &[&["0"]]).await;
    let err = t
        .run("OPTIONAL MATCH (a:Absent) INSERT (a)-[:E]->(:N {v: 8})")
        .await
        .unwrap_err();
    assert!(err.to_string().contains("null INSERT endpoint"), "{err}");
    check(&mut t, "MATCH (n:N {v: 8}) RETURN COUNT(*) AS c", &[&["0"]]).await;
    let err = t
        .run("MATCH (a:N {v: 1}) OPTIONAL MATCH (a)-[:Missing]->(b) INSERT (a)-[:G]->(:N {v: 9})-[:G]->(b)")
        .await
        .unwrap_err();
    assert!(err.to_string().contains("null INSERT endpoint"), "{err}");
    check(&mut t, "MATCH (n:N {v: 9}) RETURN COUNT(*) AS c", &[&["0"]]).await;
    check(&mut t, "MATCH ()-[e:G]->() RETURN COUNT(*) AS c", &[&["0"]]).await;
    t.run("MATCH (a:N {v: 1})-[:E]->(b) INSERT (a)-[:F]->(b)")
        .await
        .unwrap();
    check(&mut t, "MATCH ()-[e:F]->() RETURN COUNT(*) AS c", &[&["1"]]).await;
}

#[tokio::test]
async fn for_insert_and_aggregate_return_publish_once() {
    let mut s = fixture().await;
    let outputs = s
        .run("FOR x IN [10, 10, 20] INSERT (n:M {v: x}) RETURN COUNT(n) AS n, SUM(n.v) AS sum")
        .await
        .unwrap();
    let StatementOutput::Query(result) = &outputs[0] else {
        panic!()
    };
    assert_eq!(rows(result), &[vec!["3", "40"]]);
    assert_eq!(result.affected_elements, 3);
    check(&mut s, "MATCH (n:M) RETURN COUNT(*) AS c", &[&["3"]]).await;
}

#[tokio::test]
async fn nested_optional_and_nested_query_primaries_preserve_results() {
    let mut s = fixture().await;
    check(&mut s, "MATCH (a:N) OPTIONAL { MATCH (a)-[]->(b) OPTIONAL MATCH (b)-[]->(c) } RETURN a.v AS a, b.v AS b, c.v AS c ORDER BY a", &[&["1", "2", "NULL"], &["2", "NULL", "NULL"], &["3", "NULL", "NULL"]]).await;
    check(
        &mut s,
        "{ RETURN 1 AS x UNION RETURN 2 AS x } INTERSECT RETURN 2 AS x",
        &[&["2"]],
    )
    .await;
    check(&mut s, "{ RETURN 1 AS x } FINISH", &[]).await;
    check(&mut s, "{ { RETURN 4 AS x } }", &[&["4"]]).await;
}

#[tokio::test]
async fn declared_lists_distinct_ordering_and_float_aggregate_errors() {
    let mut s = Database::new().session();
    s.execute("SESSION SET VALUE $xs LIST<FLOAT> = [1, NULL, 2]; SESSION SET VALUE $empty LIST<INTEGER> = []; SESSION SET VALUE $missing LIST<INTEGER> = NULL").unwrap();
    let result = query(&mut s, "FOR x IN $xs RETURN x ORDER BY x").await;
    assert_eq!(result.schema.field(0).data_type(), &DataType::Float64);
    assert_eq!(rows(&result), &[vec!["1"], vec!["2"], vec!["NULL"]]);
    check(&mut s, "FOR x IN $empty RETURN COUNT(x) AS c", &[&["0"]]).await;
    check(&mut s, "FOR x IN $missing RETURN COUNT(x) AS c", &[&["0"]]).await;
    check(
        &mut s,
        "FOR x IN [1, 1, 2] RETURN DISTINCT x, COUNT(*) AS c ORDER BY COUNT(*) DESC, x",
        &[&["1", "2"], &["2", "1"]],
    )
    .await;
    assert!(s
        .query("FOR x IN [1.0e308, 1.0e308] RETURN SUM(x) AS s")
        .await
        .is_err());
    assert!(s
        .query("FOR x IN [1, 2] RETURN DISTINCT 1 AS c ORDER BY x")
        .await
        .is_err());
}

#[tokio::test]
async fn percentile_nulls_extremes_distinct_and_discrete_exact_values() {
    let mut s = Database::new().session();
    check(&mut s, "FOR x IN [NULL, 1, 1, 3] RETURN PERCENTILE_DISC(x, 0.5) AS d, PERCENTILE_DISC(x, 0) AS lo, PERCENTILE_DISC(x, 1) AS hi, PERCENTILE_CONT(DISTINCT x, 0.5) AS dc, PERCENTILE_DISC(DISTINCT x, 0.5) AS dd", &[&["1", "1", "3", "2", "1"]]).await;
    check(
        &mut s,
        "FOR x IN [NULL, NULL] RETURN PERCENTILE_CONT(x, 0.5) AS c, PERCENTILE_DISC(x, 0.5) AS d",
        &[&["NULL", "NULL"]],
    )
    .await;
    check(
        &mut s,
        "FOR x IN [] RETURN PERCENTILE_CONT(x, 0.5) AS c, PERCENTILE_DISC(x, 0.5) AS d",
        &[&["NULL", "NULL"]],
    )
    .await;
    check(
        &mut s,
        "FOR x IN [1, 2] RETURN PERCENTILE_CONT(x, NULL) AS c, PERCENTILE_DISC(x, NULL) AS d",
        &[&["NULL", "NULL"]],
    )
    .await;
    check(
        &mut s,
        "FOR x IN [9007199254740993, 9007199254740994] RETURN PERCENTILE_DISC(x, 0) AS d",
        &[&["9007199254740993"]],
    )
    .await;
    // 0.28 * 25 is 7.000000000000001 in f64; ceil used to select the 8th value.
    let values = (1..=25)
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    check(
        &mut s,
        &format!("FOR x IN [{values}] RETURN PERCENTILE_DISC(x, 0.28) AS d, PERCENTILE_DISC(x, 0.29) AS next"),
        &[&["7", "8"]],
    )
    .await;
    // Division yields an approximate Float64 fraction, whose non-integer
    // rank retains its interpolation weight.
    check(
        &mut s,
        "FOR x IN [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23] RETURN PERCENTILE_CONT(x, 15.0 / 22.0) AS c",
        &[&["15.999999999999998"]],
    )
    .await;
    check(
        &mut s,
        "FOR x IN [1, 2, 3, 4] RETURN PERCENTILE_CONT(x, 0.25) AS c, PERCENTILE_DISC(x, 0.5) AS d",
        &[&["1.75", "2"]],
    )
    .await;
    for sql in [
        "RETURN PERCENTILE_DISC(1, -0.1) AS d",
        "RETURN PERCENTILE_CONT(1, '0.5') AS d",
        "RETURN PERCENTILE_DISC('text', 0.5) AS d",
    ] {
        assert!(s.query(sql).await.is_err(), "{sql}");
    }
}

#[tokio::test]
async fn integer_percentile_interpolates_exact_endpoints() {
    let mut s = Database::new().session();
    let result = query(
        &mut s,
        "FOR x IN [9007199254740993, NULL, 1] RETURN PERCENTILE_CONT(x, 0.5) AS c, PERCENTILE_DISC(x, 0.5) AS d, PERCENTILE_CONT(x, 0) AS lo, PERCENTILE_CONT(x, 1) AS hi",
    )
    .await;
    assert_eq!(result.schema.field(0).data_type(), &DataType::Float64);
    assert_eq!(result.schema.field(1).data_type(), &DataType::Int64);
    assert_eq!(
        scalar(&result, 0, 0),
        ScalarValue::Float64(Some(4503599627370497.0))
    );
    assert_eq!(scalar(&result, 0, 1), ScalarValue::Int64(Some(1)));
    assert_eq!(scalar(&result, 0, 2), ScalarValue::Float64(Some(1.0)));
    assert_eq!(
        scalar(&result, 0, 3),
        ScalarValue::Float64(Some(9007199254740992.0))
    );
    let result = query(
        &mut s,
        "FOR x IN [9007199254740993, 9007199254740993, 1] RETURN PERCENTILE_CONT(DISTINCT x, 0.5) AS distinct, PERCENTILE_CONT(x, 0.5) AS repeated",
    )
    .await;
    assert_eq!(
        scalar(&result, 0, 0),
        ScalarValue::Float64(Some(4503599627370497.0))
    );
    assert_eq!(
        scalar(&result, 0, 1),
        ScalarValue::Float64(Some(9007199254740992.0))
    );
    check(
        &mut s,
        "FOR x IN [1, 2, 3, 4] RETURN PERCENTILE_CONT(x, 0.5) AS mid, PERCENTILE_CONT(x, 0.25) AS lower",
        &[&["2.5", "1.75"]],
    )
    .await;
    check(
        &mut s,
        "FOR x IN [1.0, 2.0, 3.0, 4.0] RETURN PERCENTILE_CONT(x, 0.5) AS mid",
        &[&["2.5"]],
    )
    .await;
    check(
        &mut s,
        "FOR x IN [-9007199254740993, -1] RETURN PERCENTILE_CONT(x, 0.5) AS c",
        &[&["-4503599627370497"]],
    )
    .await;
    let grouped = query(
        &mut s,
        "FOR x IN [9007199254740993, 1, 10, 20] RETURN x < 100 AS small, PERCENTILE_CONT(x, 0.5) AS p GROUP BY small ORDER BY small",
    )
    .await;
    assert_eq!(scalar(&grouped, 0, 0), ScalarValue::Boolean(Some(false)));
    assert_eq!(
        scalar(&grouped, 0, 1),
        ScalarValue::Float64(Some(9007199254740992.0))
    );
    assert_eq!(scalar(&grouped, 1, 0), ScalarValue::Boolean(Some(true)));
    assert_eq!(scalar(&grouped, 1, 1), ScalarValue::Float64(Some(10.0)));
    s.run("CREATE GRAPH g ANY GRAPH; SESSION SET GRAPH g; INSERT (:N {v: 9007199254740993}), (:N {v: 1})")
        .await
        .unwrap();
    let graph = query(&mut s, "MATCH (n) RETURN PERCENTILE_CONT(n.v, 0.5) AS c").await;
    assert_eq!(
        scalar(&graph, 0, 0),
        ScalarValue::Float64(Some(4503599627370497.0))
    );
}

fn scalar(result: &QueryResult, row: usize, column: usize) -> ScalarValue {
    let mut seen = 0;
    for batch in &result.batches {
        if row < seen + batch.num_rows() {
            return ScalarValue::try_from_array(batch.column(column), row - seen).unwrap();
        }
        seen += batch.num_rows();
    }
    panic!("missing row {row}");
}

#[tokio::test]
async fn composite_null_only_rows_strings_and_graph_focus() {
    let mut s = fixture().await;
    check(
        &mut s,
        "FOR x IN [NULL, NULL] RETURN x EXCEPT ALL RETURN NULL AS x",
        &[&["NULL"]],
    )
    .await;
    check(
        &mut s,
        "FOR x IN ['a', 'a', 'b'] RETURN x INTERSECT ALL RETURN 'a' AS x",
        &[&["a"]],
    )
    .await;
    check(
        &mut s,
        "RETURN NULL AS x INTERSECT RETURN NULL AS x",
        &[&["NULL"]],
    )
    .await;
    s.run("CREATE GRAPH h ANY GRAPH; USE GRAPH h INSERT (:N {v: 9})")
        .await
        .unwrap();
    let result = query(
        &mut s,
        "USE GRAPH h MATCH (n) RETURN n.v AS v UNION ALL MATCH (n) RETURN n.v AS v",
    )
    .await;
    let mut values = rows(&result);
    values.sort();
    assert_eq!(values, &[vec!["1"], vec!["2"], vec!["3"], vec!["9"]]);
    check(&mut s, "MATCH (n) RETURN COUNT(*) AS c", &[&["3"]]).await;
}

#[tokio::test]
async fn list_comparisons_widen_numbers_and_accept_empty_lists() {
    let mut s = Database::new().session();
    check(
        &mut s,
        "RETURN [1, 2] = [1.0, 2.0] AS same, [1] < [1.5] AS less, \
         [3] >= [2.5] AS greater, [1] <> [1.0] AS different, \
         [1] = [] AS empty_eq, [] < [1] AS empty_lt, [] = [] AS both_empty, \
         [[1], []] = [[1.0], []] AS nested",
        &[&[
            "true", "true", "true", "false", "false", "true", "true", "true",
        ]],
    )
    .await;
    check(
        &mut s,
        "RETURN NULLIF([1, 2], [1.0, 2.0]) AS removed, NULLIF([1], []) AS kept, \
         COALESCE([1], [2.0]) AS first",
        &[&["NULL", "[1]", "[1.0]"]],
    )
    .await;
    check(
        &mut s,
        "FOR x IN [[1], [1.0], [], [2]] FILTER x = [1] OR x < [] RETURN x ORDER BY x",
        &[&["[1.0]"], &["[1.0]"]],
    )
    .await;
    let kept = query(&mut s, "RETURN NULLIF([1, NULL], [1.0, NULL]) AS kept").await;
    let DataType::List(item) = kept.schema.field(0).data_type() else {
        panic!("expected a list");
    };
    assert_eq!(item.data_type(), &DataType::Float64);
    let lists = kept.batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<datafusion::arrow::array::ListArray>()
        .unwrap();
    let values = lists.value(0);
    let values = values
        .as_any()
        .downcast_ref::<datafusion::arrow::array::Float64Array>()
        .unwrap();
    assert_eq!(
        values,
        &datafusion::arrow::array::Float64Array::from(vec![Some(1.0), None])
    );

    for query in ["RETURN [1] = ['a'] AS bad", "RETURN [TRUE] < [1] AS bad"] {
        assert!(s.query(query).await.is_err(), "{query}");
    }
}

#[tokio::test]
async fn list_comparisons_keep_null_elements_unknown() {
    let mut s = fixture().await;
    check(
        &mut s,
        "RETURN ([1, NULL] = [1, NULL]) IS NULL AS same,
                ([1, NULL] = [1, 2]) IS NULL AS maybe,
                ([1, NULL] <> [1, 2]) IS NULL AS maybe_not,
                [1, 3, NULL] = [1, 2, NULL] AS earlier,
                [1] = [1, NULL] AS length,
                [1] < [1, NULL] AS prefix,
                ([1, NULL] < [1, 2]) IS NULL AS order_unknown,
                [1, 2] < [1, 3] AS order,
                [1, 2] = [1, 2] AS equal,
                [1, 2] <= [1, 2] AS le,
                [1, 3] <= [1, 2] AS not_le,
                ([1, NULL] <= [1, 2]) IS NULL AS le_unknown,
                [] = [] AS empty,
                [] < [NULL] AS empty_prefix,
                ([NULL] = [NULL]) IS NULL AS nulls,
                ([[1, NULL]] = [[1, NULL]]) IS NULL AS nested,
                ([true, NULL] = [true, false]) IS NULL AS flags,
                (['a', NULL] = ['a', NULL]) IS NULL AS names,
                [2, NULL] > [1, 9] AS decided,
                true OR [1, NULL] = [1, NULL] AS either,
                NULLIF([1, 2], [1, 2]) IS NULL AS removed,
                NULLIF([1, NULL], [1, NULL]) IS NULL AS kept,
                [X'01', X'02'] < [X'01', X'03'] AS bytes,
                [X'01'] = [X'01'] AS same_bytes",
        &[&[
            "true", "true", "true", "false", "false", "true", "true", "true", "true", "true",
            "false", "true", "true", "true", "true", "true", "true", "true", "true", "true",
            "true", "false", "true", "true",
        ]],
    )
    .await;
    check(
        &mut s,
        "FOR x IN [[1, NULL], [1, 2], [1, 3]] FILTER x = [1, 2] OR x = [1, NULL] RETURN x",
        &[&["[1, 2]"]],
    )
    .await;
    check(
        &mut s,
        "FOR x IN NULLIF([1, NULL], [1, NULL]) WITH ORDINALITY i RETURN i, x ORDER BY i",
        &[&["1", "1"], &["2", "NULL"]],
    )
    .await;
    check(
        &mut s,
        "MATCH (a:N {v: 1}), (b:N {v: 2})
         RETURN [a] = [a] AS same, [a] = [b] AS different,
                ([a, NULL] = [a, NULL]) IS NULL AS unknown",
        &[&["true", "false", "true"]],
    )
    .await;
}

#[tokio::test]
async fn output_aliases_shadow_elements_in_group_having_and_order_contexts() {
    let mut s = fixture().await;
    s.run("INSERT (:N {v: 4})").await.unwrap();
    check(&mut s, "SELECT n.k AS n, COUNT(*) AS c FROM g MATCH (n:N) GROUP BY n HAVING COUNT(n) > 0 ORDER BY n", &[&["a", "2"], &["b", "1"]]).await;
    check(
        &mut s,
        "MATCH (n:N) RETURN n.k AS n, COUNT(*) AS c ORDER BY n IS NULL DESC, n",
        &[&["NULL", "1"], &["a", "2"], &["b", "1"]],
    )
    .await;
}

#[tokio::test]
async fn later_order_by_sorts_only_the_earlier_page() {
    let mut s = fixture().await;
    // An earlier ORDER BY fixes the page. A later ORDER BY sorts that page only.
    check(
        &mut s,
        "FOR n IN [10, 20, 30] ORDER BY n ASC OFFSET 1 ORDER BY n DESC RETURN n",
        &[&["30"], &["20"]],
    )
    .await;
    check(
        &mut s,
        "FOR n IN [10, 20, 30] ORDER BY n ASC OFFSET 1 RETURN n ORDER BY n DESC",
        &[&["30"], &["20"]],
    )
    .await;
    // List unnest order is the page when no earlier ORDER BY is present.
    check(
        &mut s,
        "FOR n IN [1, 2, 3, 4] LIMIT 2 ORDER BY n DESC RETURN n",
        &[&["2"], &["1"]],
    )
    .await;
    check(
        &mut s,
        "FOR n IN [1, 2, 3, 4] LIMIT 2 RETURN n ORDER BY n DESC",
        &[&["2"], &["1"]],
    )
    .await;
    check(
        &mut s,
        "FOR n IN [1, 2, 3, 4] ORDER BY n ASC LIMIT 2 ORDER BY n DESC RETURN n",
        &[&["2"], &["1"]],
    )
    .await;
    check(
        &mut s,
        "FOR n IN [1, 2, 3, 4] ORDER BY n ASC LIMIT 2 RETURN n ORDER BY n DESC",
        &[&["2"], &["1"]],
    )
    .await;
    check(
        &mut s,
        "FOR n IN [1, 2, 3, 4, 5] LIMIT 3 ORDER BY n DESC LIMIT 2 RETURN n",
        &[&["3"], &["2"]],
    )
    .await;
    check(
        &mut s,
        "FOR n IN [1, 2, 3, 4, 5] ORDER BY n DESC OFFSET 1 LIMIT 2 RETURN n",
        &[&["4"], &["3"]],
    )
    .await;
    check(
        &mut s,
        "FOR n IN [1, 2, 3, 4, 5] OFFSET 1 LIMIT 2 RETURN n",
        &[&["2"], &["3"]],
    )
    .await;
}

#[tokio::test]
async fn otherwise_unset_focus_does_not_inherit_an_empty_branch_graph() {
    let mut s = Database::new().session();
    s.run("CREATE GRAPH g1 ANY; USE GRAPH g1 INSERT (:N {name:'one'}); SESSION RESET GRAPH")
        .await
        .unwrap();
    assert!(s.query("MATCH (n) RETURN COUNT(*) AS c").await.is_err());
    let error = s.query("USE g1 FILTER FALSE RETURN 0 AS v OTHERWISE RETURN 1 AS v NEXT MATCH (m) RETURN m.name AS name")
        .await.expect_err("empty branch graph leaked into the continuation");
    assert!(
        error.to_string().contains("current graph is unset"),
        "{error}"
    );
}

#[tokio::test]
async fn trailing_page_applies_to_the_composite_result() {
    let mut s = Database::new().session();
    check(
        &mut s,
        "FOR n IN [3, 1, 2] RETURN n UNION ALL RETURN 0 AS n ORDER BY n",
        &[&["0"], &["1"], &["2"], &["3"]],
    )
    .await;
    check(
        &mut s,
        "RETURN 3 AS n UNION RETURN 1 AS n UNION RETURN 2 AS n ORDER BY n DESC",
        &[&["3"], &["2"], &["1"]],
    )
    .await;
    check(
        &mut s,
        "FOR n IN [1, 2, 3] RETURN n UNION ALL RETURN 4 AS n ORDER BY n OFFSET 1 LIMIT 2",
        &[&["2"], &["3"]],
    )
    .await;
    check(
        &mut s,
        "FOR n IN [1, 2, 3] RETURN n EXCEPT RETURN 2 AS n ORDER BY n DESC",
        &[&["3"], &["1"]],
    )
    .await;
    check(
        &mut s,
        "FOR n IN [1, 2, 3] RETURN n INTERSECT FOR n IN [3, 1] RETURN n ORDER BY n",
        &[&["1"], &["3"]],
    )
    .await;
    check(
        &mut s,
        "RETURN NULL AS n UNION ALL RETURN 1 AS n ORDER BY n DESC",
        &[&["NULL"], &["1"]],
    )
    .await;
    let limited = query(
        &mut s,
        "FOR n IN [1, 2, 3, 4] RETURN n UNION ALL RETURN 5 AS n LIMIT 2",
    )
    .await;
    assert_eq!(limited.row_count(), 2);
    check(
        &mut s,
        "FOR n IN [3, 1] RETURN n OTHERWISE RETURN 0 AS n ORDER BY n",
        &[&["1"], &["3"]],
    )
    .await;
    check(
        &mut s,
        "FOR n IN [] RETURN n OTHERWISE FOR n IN [2, 1] RETURN n ORDER BY n DESC",
        &[&["2"], &["1"]],
    )
    .await;
    // A page before the conjunction stays on that branch. UNION ALL does not
    // promise the concatenation order, so compare the bag.
    let branch = query(
        &mut s,
        "FOR n IN [3, 1, 2] RETURN n ORDER BY n DESC LIMIT 1 UNION ALL RETURN 0 AS n",
    )
    .await;
    let mut values = rows(&branch);
    values.sort();
    assert_eq!(values, vec![vec!["0"], vec!["3"]]);
}

#[tokio::test]
async fn composite_order_preserves_an_earlier_linear_branch_page() {
    let mut s = fixture().await;
    check(
        &mut s,
        "RETURN 0 AS n UNION ALL FOR n IN [1,2,3,4] LIMIT 2 RETURN n ORDER BY n DESC",
        &[&["2"], &["1"], &["0"]],
    )
    .await;
}

#[tokio::test]
async fn exact_integer_avg_is_reusable_in_distinct_ordering() {
    let mut session = fixture().await;
    for query in [
        "FOR x IN [9007199254740993,9007199254740995] RETURN DISTINCT AVG(x) AS a ORDER BY AVG(x)",
        "FOR x IN [9007199254740993,9007199254740993,9007199254740995] RETURN DISTINCT AVG(DISTINCT x) AS a ORDER BY AVG(DISTINCT x)",
    ] {
        check(&mut session,query,&[&["9007199254740994"]]).await;
    }
}

#[tokio::test]
async fn exact_integer_standard_deviation_rounds_midpoints_and_reuses_aggregates() {
    let mut session = fixture().await;
    check(
        &mut session,
        "FOR x IN [0,18014398509481990] RETURN STDDEV_POP(x) AS value",
        &[&["9007199254740996"]],
    )
    .await;
    check(&mut session,"FOR x IN [0,18014398509481990] RETURN DISTINCT STDDEV_POP(x) AS value ORDER BY STDDEV_POP(x)",&[&["9007199254740996"]]).await;
    check(&mut session,"FOR x IN [0,0,18014398509481990] RETURN DISTINCT STDDEV_POP(DISTINCT x) AS value ORDER BY STDDEV_POP(DISTINCT x)",&[&["9007199254740996"]]).await;
}

#[tokio::test]
async fn tiny_positive_percentiles_keep_their_weight_and_repeated_ordering() {
    let mut session = fixture().await;
    for query in [
        "FOR x IN [0,1000000000000000000] RETURN PERCENTILE_CONT(x,1e-16) AS value",
        "FOR x IN [0.0,1e18] RETURN PERCENTILE_CONT(x,1e-16) AS value",
    ] {
        check(&mut session, query, &[&["100"]]).await;
    }
    for query in [
        "FOR x IN [0,1,2] RETURN DISTINCT PERCENTILE_CONT(x,0.5) AS p ORDER BY PERCENTILE_CONT(x,0.5)",
        "FOR x IN [0,1,2] RETURN DISTINCT PERCENTILE_DISC(x,0.5) AS p ORDER BY PERCENTILE_DISC(x,0.5)",
    ] { check(&mut session,query,&[&["1"]]).await; }
}

#[tokio::test]
async fn percentile_rank_keeps_a_legitimate_offset_above_an_integer() {
    let mut session = fixture().await;
    let result = query(
        &mut session,
        "FOR x IN [0,0,1000000000000000000] RETURN PERCENTILE_CONT(x,0.5000000000000001) AS value",
    )
    .await;
    let value = result.batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<datafusion::arrow::array::Float64Array>()
        .unwrap()
        .value(0);
    assert_eq!(value, 1e18 * f64::EPSILON);
    let values = (1..=25)
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join(",");
    check(
        &mut session,
        &format!("FOR x IN [{values}] RETURN PERCENTILE_DISC(x,0.2800000000000001) AS value"),
        &[&["8"]],
    )
    .await;
}

#[tokio::test]
async fn distinct_deviation_keeps_exact_integers_and_repeated_float_expressions() {
    let mut session = fixture().await;
    for query in [
        "FOR x IN [9007199254740992,9007199254740993] RETURN STDDEV_SAMP(DISTINCT x) AS value",
        "FOR x IN [9007199254740992,9007199254740992,9007199254740993] RETURN DISTINCT STDDEV_SAMP(DISTINCT x) AS value ORDER BY STDDEV_SAMP(DISTINCT x)",
    ] { check(&mut session,query,&[&["0.7071067811865476"]]).await; }
    for query in [
        "FOR x IN [1e16,1e16+2.0,1e16] RETURN DISTINCT STDDEV_POP(DISTINCT x) AS value ORDER BY STDDEV_POP(DISTINCT x)",
        "FOR x IN [0.0,2.0,0.0] RETURN DISTINCT STDDEV_POP(DISTINCT x) AS value ORDER BY STDDEV_POP(DISTINCT x)",
    ] { check(&mut session,query,&[&["1"]]).await; }
    let result = query(
        &mut session,
        "FOR x IN [-1e308,1e308] RETURN STDDEV_POP(DISTINCT x) AS value",
    )
    .await;
    assert_eq!(
        result.batches[0]
            .column(0)
            .as_any()
            .downcast_ref::<datafusion::arrow::array::Float64Array>()
            .unwrap()
            .value(0),
        1e308
    );
}
