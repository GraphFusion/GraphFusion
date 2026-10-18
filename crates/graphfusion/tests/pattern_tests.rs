use datafusion::{arrow::datatypes::DataType, common::ScalarValue};
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
fn scans(plan: &str) -> (usize, usize) {
    let mut nodes = 0;
    let mut edges = 0;
    let mut rest = plan;
    while let Some(index) = rest.find("TableScan: __gf_scan_") {
        let prelude = rest[..index].rsplit("Projection:").next().unwrap_or(rest);
        if prelude.contains("__gf_source") {
            edges += 1;
        } else if prelude.contains("__gf_id") {
            nodes += 1;
        }
        rest = &rest[index + "TableScan: __gf_scan_".len()..];
    }
    (nodes, edges)
}
#[tokio::test]
async fn quantified_anonymous_start_reads_edges() {
    let mut s = graph().await;
    s.run("INSERT (:N {name:'Z'})").await.unwrap();
    let pairs = [
        (
            "MATCH ()-[:E]->{1,2}(b) RETURN b.name AS name ORDER BY name",
            "MATCH (a)-[:E]->{1,2}(b) RETURN b.name AS name ORDER BY name",
        ),
        (
            "MATCH ()-[es:E]->{2}(b {name:'D'}) FOR e IN es RETURN ELEMENT_ID(e) AS id ORDER BY id",
            "MATCH (a)-[es:E]->{2}(b {name:'D'}) FOR e IN es RETURN ELEMENT_ID(e) AS id ORDER BY id",
        ),
        (
            "MATCH p = ()-[:E]->{1,3}() RETURN PATH_LENGTH(p) AS hops ORDER BY hops",
            "MATCH p = (a)-[:E]->{1,3}() RETURN PATH_LENGTH(p) AS hops ORDER BY hops",
        ),
        (
            "MATCH ()<-[:E]-{1,2}(b) RETURN b.name AS name ORDER BY name",
            "MATCH (a)<-[:E]-{1,2}(b) RETURN b.name AS name ORDER BY name",
        ),
        (
            "MATCH ()-[:Loop]->{1,2}() RETURN COUNT(*) AS n",
            "MATCH (a)-[:Loop]->{1,2}() RETURN COUNT(*) AS n",
        ),
        (
            "MATCH REPEATABLE ELEMENTS ()-[:Loop]->{1,2}() RETURN COUNT(*) AS n",
            "MATCH REPEATABLE ELEMENTS (a)-[:Loop]->{1,2}() RETURN COUNT(*) AS n",
        ),
        (
            "FOR x IN [1, 1] MATCH ()-[:E]->{1}() RETURN COUNT(*) AS n",
            "FOR x IN [1, 1] MATCH (a)-[:E]->{1}() RETURN COUNT(*) AS n",
        ),
        (
            "MATCH ()-[:E]->{0,1}(b) RETURN b.name AS name ORDER BY name",
            "MATCH (a)-[:E]->{0,1}(b) RETURN b.name AS name ORDER BY name",
        ),
        (
            "MATCH p = ANY SHORTEST ()-[:E]->*(b) RETURN b.name AS name, PATH_LENGTH(p) AS hops ORDER BY name, hops",
            "MATCH p = ANY SHORTEST (a)-[:E]->*(b) RETURN b.name AS name, PATH_LENGTH(p) AS hops ORDER BY name, hops",
        ),
        (
            "MATCH ()-[:E]->()-[:E]->{1,2}(b) RETURN b.name AS name ORDER BY name",
            "MATCH (a)-[:E]->()-[:E]->{1,2}(b) RETURN b.name AS name ORDER BY name",
        ),
    ];
    for (open, named) in pairs {
        assert_eq!(
            rows(&query(&mut s, open).await),
            rows(&query(&mut s, named).await),
            "{open}"
        );
    }
    s.run("MATCH (a {name:'A'}) INSERT (a)-[:W {w:1}]->(:N {name:'Y'}), (a)-[:W {w:9}]->(:N {name:'X'})").await.unwrap();
    let filtered = [
        "MATCH ()-[e:W WHERE e.w < 5]->{1,2}(b) RETURN b.name AS name ORDER BY name",
        "MATCH (a)-[e:W WHERE e.w < 5]->{1,2}(b) RETURN b.name AS name ORDER BY name",
    ];
    assert_eq!(
        rows(&query(&mut s, filtered[0]).await),
        rows(&query(&mut s, filtered[1]).await)
    );
    s.run("INSERT (:N {name:'P'})~[:U]~(:N {name:'Q'})")
        .await
        .unwrap();
    assert_eq!(
        rows(&query(&mut s, "MATCH ()~[:U]~{1}() RETURN COUNT(*) AS n").await),
        rows(&query(&mut s, "MATCH (a)~[:U]~{1}() RETURN COUNT(*) AS n").await)
    );

    let open = scans(
        &query(&mut s, "MATCH ()-[:E]->{1,2}() RETURN COUNT(*) AS n")
            .await
            .logical_plan,
    );
    let named = scans(
        &query(&mut s, "MATCH (a)-[:E]->{1,2}() RETURN COUNT(*) AS n")
            .await
            .logical_plan,
    );
    assert_eq!(
        named.0,
        open.0 + 1,
        "named start adds one node scan: open={open:?} named={named:?}"
    );
    let zero = scans(
        &query(&mut s, "MATCH ()-[:E]->{0,1}() RETURN COUNT(*) AS n")
            .await
            .logical_plan,
    );
    let zero_named = scans(
        &query(&mut s, "MATCH (a)-[:E]->{0,1}() RETURN COUNT(*) AS n")
            .await
            .logical_plan,
    );
    assert_eq!(
        zero.0, zero_named.0,
        "zero repetitions still anchor on nodes"
    );
    let labeled = scans(
        &query(&mut s, "MATCH (:N)-[:E]->{1,2}() RETURN COUNT(*) AS n")
            .await
            .logical_plan,
    );
    assert_eq!(labeled.0, open.0 + 1);
}
#[tokio::test]
async fn anonymous_path_starts_at_the_first_edge() {
    let mut s = graph().await;
    let q = "MATCH ()-[:E]->() RETURN COUNT(*) AS n";
    assert_eq!(rows(&query(&mut s, q).await), vec![vec!["5"]]);
    let q = "MATCH ()-[:E]->()-[:E]->() RETURN COUNT(*) AS n";
    assert_eq!(rows(&query(&mut s, q).await), vec![vec!["6"]]);
    let q = "MATCH ()<-[:E]-() RETURN COUNT(*) AS n";
    assert_eq!(rows(&query(&mut s, q).await), vec![vec!["5"]]);
    let q = "FOR x IN [1, 2] MATCH ()-[:E]->() RETURN COUNT(*) AS n";
    assert_eq!(rows(&query(&mut s, q).await), vec![vec!["10"]]);
    let open = scans(
        &query(&mut s, "MATCH ()-[e:E]->() RETURN e")
            .await
            .logical_plan,
    );
    let named = scans(
        &query(&mut s, "MATCH (a)-[e:E]->() RETURN e")
            .await
            .logical_plan,
    );
    assert_eq!(open.1, 1, "edge scans");
    assert_eq!(named, (open.0 + 1, 1), "named start adds one node scan");
    let end = scans(
        &query(&mut s, "MATCH ()-[e:E]->(b) RETURN b.name AS name")
            .await
            .logical_plan,
    );
    assert_eq!(end, named);
    let labeled = scans(
        &query(&mut s, "MATCH (:N)-[e:E]->() RETURN e")
            .await
            .logical_plan,
    );
    assert_eq!(labeled.0, open.0 + 1);
    assert_eq!(
        rows(&query(&mut s, "MATCH (:N)-[:E]->() RETURN COUNT(*) AS n").await),
        vec![vec!["5"]]
    );
}
#[tokio::test]
async fn anonymous_endpoints_reuse_the_edge_endpoint() {
    let mut s = graph().await;
    s.run("INSERT (:N {name:'A'})-[:E]->({name:'Z'})")
        .await
        .unwrap();
    // Five original :E edges, plus A -> Z.
    let q = "MATCH ()-[:E]->() RETURN COUNT(*) AS n";
    assert_eq!(rows(&query(&mut s, q).await), vec![vec!["6"]]);
    let plan = query(&mut s, "MATCH ()-[e:E]->() RETURN e")
        .await
        .logical_plan;
    let open = scans(&plan);
    assert_eq!(open, (0, 1), "{plan}");
    let named = query(&mut s, "MATCH ()-[e:E]->(b) RETURN e")
        .await
        .logical_plan;
    // Naming the endpoint still reads the node tables.
    let named_scans = scans(&named);
    assert!(named_scans.0 > 0 && named_scans.1 == 1, "{named}");
    // A label or property map still has to read the endpoint node.
    let q = "MATCH ()-[:E]->(:N) RETURN COUNT(*) AS n";
    assert_eq!(rows(&query(&mut s, q).await), vec![vec!["5"]]);
    let q = "MATCH (a)-[:E]->({name:'Z'}) RETURN a.name AS name";
    assert_eq!(rows(&query(&mut s, q).await), vec![vec!["A"]]);
    let q = "MATCH ()-[:E]->()-[:E]->() RETURN COUNT(*) AS n";
    assert_eq!(rows(&query(&mut s, q).await), vec![vec!["6"]]);
    let plan = query(&mut s, q).await.logical_plan;
    assert_eq!(scans(&plan), (open.0, 2), "{plan}");
    let q = "MATCH ()-[:E]->({name:'B'})-[:E]->() RETURN COUNT(*) AS n";
    assert_eq!(rows(&query(&mut s, q).await), vec![vec!["1"]]);
    let plan = query(&mut s, q).await.logical_plan;
    assert_eq!(scans(&plan), (named_scans.0, 2), "{plan}");
    let q = "MATCH (a {name:'A'})(()-[:E]->()) RETURN COUNT(*) AS n";
    assert_eq!(rows(&query(&mut s, q).await), vec![vec!["3"]]);
    let plan = query(&mut s, q).await.logical_plan;
    assert_eq!(scans(&plan), (named_scans.0, 1), "{plan}");
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
    let q = "MATCH p = ((a)-[e:E]->(b))?(c) RETURN COUNT(*) AS n";
    // Four zero-hop anchors plus one row for each :E edge.
    assert_eq!(rows(&query(&mut s, q).await), vec![vec!["9"]]);
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
async fn alternation_unifies_divergent_property_schemas() {
    let mut s = Database::new().session();
    s.run("CREATE GRAPH g ANY; SESSION SET GRAPH g; INSERT (:A {name:'a', v:1}), (:B {name:'b', v:1.5})").await.unwrap();
    let widened = query(
        &mut s,
        "MATCH (n:A) | (n:B) RETURN n.name AS name, n.v AS v, n IS LABELED A AS a, n IS LABELED B AS b ORDER BY name",
    )
    .await;
    assert_eq!(
        widened.schema.field_with_name("v").unwrap().data_type(),
        &DataType::Float64
    );
    assert_eq!(
        rows(&widened),
        vec![
            vec!["a", "1", "true", "false"],
            vec!["b", "1.5", "false", "true"]
        ]
    );
    let reversed = query(
        &mut s,
        "MATCH (n:B) | (n:A) WHERE n.v > 1.2 RETURN n.name AS name",
    )
    .await;
    assert_eq!(
        reversed.schema.field_with_name("name").unwrap().data_type(),
        &DataType::Utf8
    );
    assert_eq!(rows(&reversed), vec![vec!["b"]]);

    let mut s = Database::new().session();
    s.run("CREATE GRAPH g ANY; SESSION SET GRAPH g; INSERT (:A {name:'a', v:1}), (:B {name:'b'})")
        .await
        .unwrap();
    // The extra property exists on only one branch. Projecting the shared name
    // used to fail while resolving the other branch's missing column.
    assert_eq!(
        rows(
            &query(
                &mut s,
                "MATCH (n:A) | (n:B) RETURN n.name AS name, n.v AS v ORDER BY name"
            )
            .await
        ),
        vec![vec!["a", "1"], vec!["b", "NULL"]]
    );

    let mut s = Database::new().session();
    s.run("CREATE GRAPH g ANY; SESSION SET GRAPH g; INSERT (:A {v:1}), (:B {v:true})")
        .await
        .unwrap();
    let alternation = query(&mut s, "MATCH (n:A) | (n:B) RETURN n.v AS v ORDER BY v").await;
    let disjunction = query(&mut s, "MATCH (n:A|B) RETURN n.v AS v ORDER BY v").await;
    assert_eq!(
        alternation.schema.field_with_name("v").unwrap().data_type(),
        disjunction.schema.field_with_name("v").unwrap().data_type()
    );
    assert_eq!(rows(&alternation), rows(&disjunction));

    let mut s = Database::new().session();
    s.run("CREATE GRAPH g ANY; SESSION SET GRAPH g; INSERT (:A {name:'a'}), (:B {name:'b'})")
        .await
        .unwrap();
    s.run("MATCH (n:A) | (n:B) SET n.flag = true RETURN n.name AS name")
        .await
        .unwrap();
    assert_eq!(
        rows(
            &query(
                &mut s,
                "MATCH (n:A|B) RETURN n.name AS name, n.flag AS flag ORDER BY name"
            )
            .await
        ),
        vec![vec!["a", "true"], vec!["b", "true"]]
    );

    let mut s = Database::new().session();
    s.run("CREATE GRAPH g ANY; SESSION SET GRAPH g; INSERT (:N)-[:E {v:1}]->(:N), (:N)-[:F {v:1.5, w:true}]->(:N)").await.unwrap();
    let edges = query(
        &mut s,
        "MATCH ()-[e:E]->() | ()-[e:F]->() RETURN e.v AS v, e.w AS w ORDER BY v",
    )
    .await;
    assert_eq!(
        edges.schema.field_with_name("v").unwrap().data_type(),
        &DataType::Float64
    );
    assert_eq!(
        edges.schema.field_with_name("w").unwrap().data_type(),
        &DataType::Boolean
    );
    assert_eq!(rows(&edges), vec![vec!["1", "NULL"], vec!["1.5", "true"]]);
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
async fn independent_repeated_group_reuses_one_step() {
    let mut s = graph().await;
    let shared = "MATCH p = (a)((x)-[:E]->(y)){1,2}(b) RETURN a.name AS a, b.name AS b, PATH_LENGTH(p) AS hops ORDER BY a, hops, b";
    let result = query(&mut s, shared).await;
    assert!(
        !result.logical_plan.contains("step_row"),
        "{}",
        result.logical_plan
    );
    let flat = "MATCH p = (a)-[:E]->{1,2}(b) RETURN a.name AS a, b.name AS b, PATH_LENGTH(p) AS hops ORDER BY a, hops, b";
    assert_eq!(rows(&result), rows(&query(&mut s, flat).await));
    let dependent =
        "MATCH (a)((x)-[:Loop]->(y WHERE x.name = a.name)){1}(b) RETURN a.name AS a, b.name AS b";
    let dependent_result = query(&mut s, dependent).await;
    assert!(
        dependent_result.logical_plan.contains("step_row"),
        "{}",
        dependent_result.logical_plan
    );
    assert_eq!(rows(&dependent_result), vec![vec!["A", "A"]]);
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
async fn selective_search_uses_the_node_bound_under_different_edges() {
    let mut s = graph().await;
    // Four nodes and six edges. Zero-or-more selection is complete by hop 3.
    let q =
        "MATCH p = ANY SHORTEST (a {name:'A'})-[:E]->*(d {name:'D'}) RETURN PATH_LENGTH(p) AS hops";
    let result = query(&mut s, q).await;
    assert_eq!(rows(&result), vec![vec!["2"]]);
    assert!(
        result.logical_plan.contains("< UInt64(3)"),
        "{}",
        result.logical_plan
    );
    assert!(
        !result.logical_plan.contains("< UInt64(6)"),
        "{}",
        result.logical_plan
    );
    let q = "MATCH p = ALL SHORTEST (a {name:'A'})-[:E]->+(d) RETURN d.name AS name, PATH_LENGTH(p) AS hops ORDER BY name, hops";
    let open = query(&mut s, q).await;
    assert_eq!(
        rows(&open),
        rows(&query(&mut s, &q.replace("->+", "->{1,8}")).await)
    );
    assert!(
        open.logical_plan.contains("< UInt64(4)"),
        "{}",
        open.logical_plan
    );

    let anonymous = "MATCH p = ALL SHORTEST ()-[:E]->+(d) RETURN d.name AS name, PATH_LENGTH(p) AS hops ORDER BY name, hops";
    let seeded = query(&mut s, anonymous).await;
    assert_eq!(
        rows(&seeded),
        rows(&query(&mut s, &anonymous.replace("->+", "->{1,8}")).await)
    );
    assert!(
        seeded.logical_plan.contains("< UInt64(4)"),
        "{}",
        seeded.logical_plan
    );

    // The unique A-to-D route has length 3, equal to the zero-or-more bound.
    s.execute("CREATE GRAPH line ANY; SESSION SET GRAPH line")
        .unwrap();
    s.run(
        "INSERT (a:N {name:'A'})-[:E]->(b:N {name:'B'})-[:E]->(c:N {name:'C'})-[:E]->(d:N {name:'D'}), \
         (b)-[:E]->(a), (c)-[:E]->(a), (c)-[:E]->(b), (d)-[:E]->(a), (d)-[:E]->(b)",
    )
    .await
    .unwrap();
    let q =
        "MATCH p = ANY SHORTEST (a {name:'A'})-[:E]->*(d {name:'D'}) RETURN PATH_LENGTH(p) AS hops";
    let result = query(&mut s, q).await;
    assert_eq!(rows(&result), vec![vec!["3"]]);
    assert!(
        result.logical_plan.contains("< UInt64(3)"),
        "{}",
        result.logical_plan
    );
    for selector in ["ALL SHORTEST", "SHORTEST 2 GROUPS"] {
        let q = format!(
            "MATCH p = {selector} (a {{name:'A'}})-[:E]->*(b) RETURN b.name AS name, PATH_LENGTH(p) AS hops ORDER BY name, hops"
        );
        assert_eq!(
            rows(&query(&mut s, &q).await),
            rows(&query(&mut s, &q.replace("-[:E]->*", "-[:E]->{0,8}")).await),
            "{q}"
        );
    }
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

#[tokio::test]
async fn shared_elements_accept_an_empty_alternative_in_either_order() {
    let mut s = Database::new().session();
    s.run("CREATE GRAPH g ANY; SESSION SET GRAPH g; INSERT (:A {v:1})")
        .await
        .unwrap();
    for q in [
        "MATCH (n:A) | (n:Missing) RETURN n.v AS v",
        "MATCH (n:Missing) | (n:A) RETURN n.v AS v",
    ] {
        let result = query(&mut s, q).await;
        assert_eq!(rows(&result), vec![vec!["1"]]);
        assert_eq!(result.schema.field(0).data_type(), &DataType::Int64);
    }
}
