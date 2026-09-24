use datafusion::common::ScalarValue;
use graphfusion::{Database, QueryResult, Session};
async fn graph() -> Session {
    let db = Database::new();
    let mut s = db.session();
    s.execute("CREATE GRAPH g ANY; SESSION SET GRAPH g")
        .unwrap();
    s.run("INSERT (a:N {name:'A'}),(b:N {name:'B'}),(c:N {name:'C'}),(d:N {name:'D'}),(a)-[:E]->(b),(a)-[:E]->(c),(b)-[:E]->(d),(c)-[:E]->(d),(d)-[:E]->(a),(a)-[:Loop]->(a)").await.unwrap();
    s
}
fn rows(result: &QueryResult) -> Vec<Vec<String>> {
    result
        .batches
        .iter()
        .flat_map(|b| {
            (0..b.num_rows()).map(|r| {
                (0..b.num_columns())
                    .map(|c| {
                        ScalarValue::try_from_array(b.column(c), r)
                            .unwrap()
                            .to_string()
                    })
                    .collect()
            })
        })
        .collect()
}
async fn query(s: &mut Session, q: &str) -> QueryResult {
    s.query(q).await.unwrap_or_else(|e| panic!("{q}\n{e}"))
}
#[tokio::test]
async fn parentheses_concatenation_subpaths_and_prefilters() {
    let mut s = graph().await;
    let q="MATCH p = ALL SHORTEST ((a {name:'A'})-[:E]->(b)-[:E]->(d {name:'D'}) WHERE b.name='C') RETURN b.name AS via, PATH_LENGTH(p) AS hops";
    assert_eq!(rows(&query(&mut s, q).await), vec![vec!["C", "2"]]);
    let q="MATCH p = (sub = TRAIL (a {name:'A'})-[:E]->(b) WHERE b.name='B')-[:E]->(d) RETURN b.name AS via, d.name AS dest, PATH_LENGTH(p) AS total, PATH_LENGTH(sub) AS part";
    assert_eq!(
        rows(&query(&mut s, q).await),
        vec![vec!["B", "D", "2", "1"]]
    );
    let q="MATCH p = (a {name:'A'})(x)-[:E]->(b)(y) RETURN SAME(a,x) AS same_start, SAME(b,y) AS same_end";
    assert_eq!(rows(&query(&mut s, q).await), vec![vec!["true", "true"]; 2]);
}
#[tokio::test]
async fn quantified_groups_join_endpoints_and_preserve_zero_lengths() {
    let mut s = graph().await;
    let q="MATCH p = (a {name:'A'})((x)-[es:E]->(y)){0,2}(b) RETURN b.name AS name, PATH_LENGTH(p) AS hops ORDER BY hops, name";
    let r = query(&mut s, q).await;
    assert_eq!(
        rows(&r),
        vec![
            vec!["A", "0"],
            vec!["B", "1"],
            vec!["C", "1"],
            vec!["D", "2"],
            vec!["D", "2"]
        ]
    );
    assert!(r.physical_plan.contains("RecursiveQueryExec"));
    let q="MATCH p = (a {name:'A'})((x)-[:E]->(y) WHERE x.name <> 'B'){1,2}(b) RETURN b.name AS name, PATH_LENGTH(p) AS hops ORDER BY hops, name";
    assert_eq!(
        rows(&query(&mut s, q).await),
        vec![vec!["B", "1"], vec!["C", "1"], vec!["D", "2"]]
    );
    let q =
        "MATCH p = (a {name:'A'})((x)-[es:E]->(y)){0,1}(b) FOR e IN es RETURN ELEMENT_ID(e) AS id";
    assert_eq!(query(&mut s, q).await.row_count(), 2);
}
#[tokio::test]
async fn composed_paths_resolve_forward_predicates_before_materialization() {
    let mut s = graph().await;
    let q = "MATCH p = (a {name:'A'})((x WHERE x.name < y.name)-[:E]->(y)){1,2}(b) RETURN b.name AS name, PATH_LENGTH(p) AS hops ORDER BY hops, name";
    assert_eq!(
        rows(&query(&mut s, q).await),
        vec![
            vec!["B", "1"],
            vec!["C", "1"],
            vec!["D", "2"],
            vec!["D", "2"]
        ]
    );
    let q = "MATCH p = (a WHERE a.name < b.name)-[:E]->(b) | (a WHERE a.name < b.name)-[:E]->(b) RETURN a.name AS a, b.name AS b ORDER BY a, b";
    assert_eq!(
        rows(&query(&mut s, q).await),
        vec![
            vec!["A", "B"],
            vec!["A", "C"],
            vec!["B", "D"],
            vec!["C", "D"]
        ]
    );
    let q = "MATCH p = (a {name:'A'})(-[e:E WHERE a.name < b.name]->(b))?(c) RETURN b.name AS optional_name, c.name AS name ORDER BY name";
    assert_eq!(
        rows(&query(&mut s, q).await),
        vec![vec!["NULL", "A"], vec!["B", "B"], vec!["C", "C"]]
    );
}
#[tokio::test]
async fn whole_path_modes_cross_group_boundaries() {
    let mut s = graph().await;
    for (mode, n) in [("WALK", 1), ("TRAIL", 0), ("SIMPLE", 0), ("ACYCLIC", 0)] {
        let q=format!("MATCH REPEATABLE ELEMENTS p = {mode} (a {{name:'A'}})((x)-[:Loop]->(y)){{2}}(b) RETURN PATH_LENGTH(p) AS hops");
        assert_eq!(query(&mut s, &q).await.row_count(), n, "{q}");
    }
    for (mode, maximum) in [("TRAIL", "5"), ("SIMPLE", "3"), ("ACYCLIC", "2")] {
        let q=format!("MATCH REPEATABLE ELEMENTS p = {mode} (a {{name:'A'}})((x)-[:E]->(y))+(b) RETURN MAX(PATH_LENGTH(p)) AS longest");
        assert_eq!(rows(&query(&mut s, &q).await), vec![vec![maximum]], "{q}");
    }
    let q = "MATCH REPEATABLE ELEMENTS p = ACYCLIC ((a {name:'A'})-[:Loop]->{8,}(b)) RETURN PATH_LENGTH(p) AS hops";
    assert_eq!(query(&mut s, q).await.row_count(), 0);
}
#[tokio::test]
async fn multipath_uniqueness_does_not_silently_change_selection() {
    let mut s = graph().await;
    for q in [
        "MATCH p = ANY SHORTEST (a)-[:E]->{1,3}(b), (a)-[:E]->(c) RETURN p",
        "MATCH DIFFERENT EDGES (a)-[:E]->(c), p = ALL SHORTEST (a)-[:E]->{1,3}(b) RETURN p",
    ] {
        assert!(s
            .query(q)
            .await
            .unwrap_err()
            .to_string()
            .contains("selective paths in a multi-path DIFFERENT EDGES MATCH"));
    }
    let q = "MATCH (a {name:'A'})-[e:E]->(b), (a)-[f:E]->(c) RETURN b.name AS b, c.name AS c ORDER BY b, c";
    assert_eq!(
        rows(&query(&mut s, q).await),
        vec![vec!["B", "C"], vec!["C", "B"]]
    );
    let q = "MATCH REPEATABLE ELEMENTS p = ALL SHORTEST (a {name:'A'})-[:E]->{1,3}(b {name:'D'}), q = ANY SHORTEST (a)-[:E]->{1,3}(b) RETURN PATH_LENGTH(p) AS first, PATH_LENGTH(q) AS second";
    assert_eq!(rows(&query(&mut s, q).await), vec![vec!["2", "2"]; 2]);
}
#[tokio::test]
async fn questioned_paths_expose_conditional_singletons() {
    let mut s = graph().await;
    let q="MATCH p = (a {name:'A'})(-[e:E]->(b))?(c) RETURN b.name AS optional_name, c.name AS endpoint, e IS NULL AS absent, PATH_LENGTH(p) AS hops ORDER BY endpoint";
    assert_eq!(
        rows(&query(&mut s, q).await),
        vec![
            vec!["NULL", "A", "true", "0"],
            vec!["B", "B", "false", "1"],
            vec!["C", "C", "false", "1"]
        ]
    );
    assert!(s
        .query("MATCH (a {name:'A'})(->(b))?(b) RETURN b.name AS name")
        .await
        .unwrap_err()
        .to_string()
        .contains("conditional"));
}
#[tokio::test]
async fn union_and_multiset_alternation_preserve_the_right_bags() {
    let mut s = graph().await;
    for (operator, count) in [("|", 2), ("|+|", 4)] {
        let q=format!("MATCH p = (a {{name:'A'}})-[:E]->(b) {operator} (a {{name:'A'}})-[:E]->(b) RETURN PATH_LENGTH(p) AS hops");
        assert_eq!(query(&mut s, &q).await.row_count(), count, "{q}");
        let q=format!("FOR x IN [1,1] MATCH p = (a {{name:'A'}})-[:E]->(b) {operator} (a {{name:'A'}})-[:E]->(b) RETURN x, PATH_LENGTH(p) AS hops");
        assert_eq!(query(&mut s, &q).await.row_count(), 2 * count, "{q}");
    }
    let q="MATCH p = ((a {name:'A'})-[:E]->(b)) | ((a {name:'A'})-[:Loop]->(b)) RETURN b.name AS name ORDER BY name";
    assert_eq!(
        rows(&query(&mut s, q).await),
        vec![vec!["A"], vec!["B"], vec!["C"]]
    );
}
#[tokio::test]
async fn alternatives_expose_nulls_and_reject_incompatible_bindings() {
    let mut s = graph().await;
    let q="MATCH (x {name:'A'})-[:E]->(y) | (x {name:'A'})-[:E]->(z) RETURN y.name AS left_name, z.name AS right_name ORDER BY left_name NULLS FIRST, right_name NULLS FIRST";
    assert_eq!(
        rows(&query(&mut s, q).await),
        vec![
            vec!["NULL", "B"],
            vec!["NULL", "C"],
            vec!["B", "NULL"],
            vec!["C", "NULL"]
        ]
    );
    assert!(s
        .query("MATCH (x)->(y) | (x)->(z), (y)->(w) RETURN w.name AS name")
        .await
        .unwrap_err()
        .to_string()
        .contains("conditional"));
    assert!(s
        .query("MATCH (x)->() | ()-[x]->() RETURN ELEMENT_ID(x) AS id")
        .await
        .is_err());
}
#[tokio::test]
async fn alternation_and_nested_quantifiers_compose_inside_recursion() {
    let mut s = graph().await;
    for (op, count) in [("|", "4"), ("|+|", "12")] {
        let q=format!("MATCH p = (a {{name:'A'}})((x)-[:E]->(y) {op} (x)-[:E]->(y)){{1,2}}(b) RETURN COUNT(*) AS paths");
        assert_eq!(rows(&query(&mut s, &q).await), vec![vec![count]], "{q}");
    }
    let q="MATCH p = (a {name:'A'})((x)-[es:E]->{1,2}(y)){1,2}(b) RETURN MAX(PATH_LENGTH(p)) AS longest";
    assert_eq!(rows(&query(&mut s, q).await), vec![vec!["4"]]);
    let q="MATCH p = (a {name:'A'})((x)-[:E]->(y)(-[e:E]->(z))?){1}(b) FOR n IN z RETURN ELEMENT_ID(n) AS id ORDER BY id NULLS FIRST";
    let r = query(&mut s, q).await;
    assert_eq!(r.row_count(), 4);
    assert_eq!(rows(&r).iter().filter(|r| r[0] == "NULL").count(), 2);
}
#[tokio::test]
async fn group_scope_limits_and_runtime_errors_are_atomic() {
    let mut s = graph().await;
    for q in [
        "MATCH (a) ((a)->(b)){1,2} RETURN a",
        "MATCH ((a)){1} RETURN a",
        "MATCH REPEATABLE ELEMENTS ((a)->(b))+ RETURN a",
    ] {
        assert!(s.query(q).await.is_err(), "{q}");
    }
    let error = s
        .run("INSERT (:Ghost) MATCH ((a)-[:E]->(b) WHERE 1 / 0 = 1) RETURN a.name AS name")
        .await
        .unwrap_err();
    assert!(error.to_string().contains("zero"), "{error}");
    assert_eq!(
        rows(&query(&mut s, "MATCH (n:Ghost) RETURN COUNT(*) AS n").await),
        vec![vec!["0"]]
    );
    let output=s.run("MATCH (a {name:'A'})-[:E]->(y) | (a {name:'A'})-[:E]->(z) SET y.flag=TRUE RETURN y.name AS y, z.name AS z").await.unwrap();
    let graphfusion::StatementOutput::Query(result) = &output[0] else {
        panic!("query");
    };
    assert_eq!(result.row_count(), 4);
    assert_eq!(result.affected_elements, 2);
    assert_eq!(
        rows(
            &query(
                &mut s,
                "MATCH (n WHERE n.flag=TRUE) RETURN n.name AS name ORDER BY name"
            )
            .await
        ),
        vec![vec!["B"], vec!["C"]]
    );
}

#[tokio::test]
async fn groups_match_flat_expansions_and_keep_ordered_bindings() {
    let mut s = graph().await;
    for mode in ["WALK", "TRAIL", "SIMPLE", "ACYCLIC"] {
        for (min, max) in [(0, 1), (0, 3), (1, 3), (2, 3)] {
            let grouped=format!("FOR input IN [1,1] MATCH REPEATABLE ELEMENTS p = {mode} (a {{name:'A'}})((x)-[:E]->(y)){{{min},{max}}}(b) RETURN input, p");
            let flat=format!("FOR input IN [1,1] MATCH REPEATABLE ELEMENTS p = {mode} (a {{name:'A'}})-[:E]->{{{min},{max}}}(b) RETURN input, p");
            let mut expected = rows(&query(&mut s, &flat).await);
            expected.sort();
            let mut actual = rows(&query(&mut s, &grouped).await);
            actual.sort();
            assert_eq!(actual, expected, "{grouped}");
        }
    }
    let base = "MATCH p = (a {name:'A'})((xs)-[es:E]->(ys) WHERE xs.name <> 'B'){2}(b {name:'D'})";
    let xs = query(
        &mut s,
        &format!("{base} FOR x IN xs WITH ORDINALITY i RETURN ELEMENT_ID(x) AS id ORDER BY i"),
    )
    .await;
    let ys = query(
        &mut s,
        &format!("{base} FOR y IN ys WITH ORDINALITY i RETURN ELEMENT_ID(y) AS id ORDER BY i"),
    )
    .await;
    let expected=query(&mut s,"MATCH (a {name:'A'})-[:E]->(c {name:'C'})-[:E]->(d {name:'D'}) RETURN ELEMENT_ID(a) AS a, ELEMENT_ID(c) AS c, ELEMENT_ID(d) AS d").await;
    let expected = rows(&expected);
    assert_eq!(
        rows(&xs),
        vec![vec![expected[0][0].clone()], vec![expected[0][1].clone()]]
    );
    assert_eq!(
        rows(&ys),
        vec![vec![expected[0][1].clone()], vec![expected[0][2].clone()]]
    );
    let base = "MATCH p = (a {name:'A'})((x)-[es:E]->{1,2}(y)){1,2}(b)";
    let lengths = query(&mut s, &format!("{base} RETURN SUM(PATH_LENGTH(p)) AS n")).await;
    let refs = query(
        &mut s,
        &format!("{base} FOR e IN es RETURN COUNT(ELEMENT_ID(e)) AS n"),
    )
    .await;
    assert_eq!(rows(&refs), rows(&lengths)); // nested group lists flatten in traversal order
}

#[tokio::test]
async fn unbounded_homogeneous_selection_uses_complete_finite_bounds() {
    let mut s = graph().await;
    for selector in [
        "ALL SHORTEST",
        "ANY SHORTEST",
        "SHORTEST 3",
        "SHORTEST 2 GROUPS",
    ] {
        let q=format!("MATCH REPEATABLE ELEMENTS p = {selector} (a {{name:'A'}})-[:E]->+(b) RETURN b.name AS name, PATH_LENGTH(p) AS hops ORDER BY name, hops");
        let bounded = q.replace("->+", "->{1,12}");
        assert_eq!(
            rows(&query(&mut s, &q).await),
            rows(&query(&mut s, &bounded).await),
            "{q}"
        );
    }
    let q="MATCH REPEATABLE ELEMENTS p = ALL SHORTEST (a {name:'A'})-[:Loop]->{7,}(a) RETURN PATH_LENGTH(p) AS hops";
    assert_eq!(rows(&query(&mut s, q).await), vec![vec!["7"]]);
    let q="MATCH REPEATABLE ELEMENTS p = SHORTEST 3 GROUPS (a {name:'A'})-[:E]->*(a) RETURN PATH_LENGTH(p) AS hops ORDER BY hops";
    assert_eq!(
        rows(&query(&mut s, q).await),
        vec![
            vec!["0"],
            vec!["3"],
            vec!["3"],
            vec!["6"],
            vec!["6"],
            vec!["6"],
            vec!["6"]
        ]
    );
    // A group-list condition is history-dependent and cannot use the cycle-removal proof.
    let q="MATCH REPEATABLE ELEMENTS (a {name:'A'})-[previous:Loop]->{8}(a) MATCH REPEATABLE ELEMENTS p = ALL SHORTEST (a)-[es:Loop]->*(b WHERE es=previous) RETURN PATH_LENGTH(p) AS hops";
    assert!(s
        .query(q)
        .await
        .unwrap_err()
        .to_string()
        .contains("unbounded"));
    let q = q.replace("->*(b", "->{0,8}(b");
    assert_eq!(rows(&query(&mut s, &q).await), vec![vec!["8"]]);
}
