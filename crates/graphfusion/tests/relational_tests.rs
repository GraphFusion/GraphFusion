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
    assert!(s
        .run("OPTIONAL MATCH (b:Absent) INSERT (:N {v: 9})-[:E]->(b)")
        .await
        .is_err());
    check(&mut s, "MATCH (n:N) RETURN COUNT(*) AS c", &[&["3"]]).await;
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
    for sql in [
        "RETURN PERCENTILE_DISC(1, -0.1) AS d",
        "RETURN PERCENTILE_CONT(1, '0.5') AS d",
        "RETURN PERCENTILE_DISC('text', 0.5) AS d",
    ] {
        assert!(s.query(sql).await.is_err(), "{sql}");
    }
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
