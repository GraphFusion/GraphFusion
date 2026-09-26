use datafusion::common::ScalarValue;
use graphfusion::{Database, QueryResult, Session};

async fn graph() -> Session {
    let db = Database::new();
    let mut s = db.session();
    s.execute("CREATE GRAPH g ANY; SESSION SET GRAPH g")
        .unwrap();
    s.run("INSERT (a:N {name:'A',v:1}),(b:N {name:'B',v:2}),(c:N {name:'C',v:3}),(a)-[:E {name:'AB',w:4}]->(b),(b)-[:U {name:'BC',w:5}]-(c)").await.unwrap();
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
async fn check(s: &mut Session, q: &str, expected: &[&[&str]]) {
    let r = s.query(q).await.unwrap_or_else(|e| panic!("{q}\n{e}"));
    assert_eq!(rows(&r), expected, "{q}");
}

#[tokio::test]
async fn whole_elements_aliases_and_lists_preserve_identity_and_nulls() {
    let mut s = graph().await;
    check(&mut s, "MATCH (a {name:'A'}), (b {name:'B'}) LET x=a FOR r IN [x,NULL,b,x] WITH ORDINALITY i RETURN i,r.name AS name,PROPERTY_EXISTS(r,v) AS present,r IS LABELED N AS labeled,SAME(r,a) AS same,r IS NULL AS absent ORDER BY i",
        &[&["1","A","true","true","true","false"], &["2","NULL","NULL","NULL","NULL","true"], &["3","B","true","true","false","false"], &["4","A","true","true","true","false"]]).await;
    check(&mut s, "MATCH (a {name:'A'}) LET r=a RETURN r=a AS same,ALL_DIFFERENT(a,r) AS different,ELEMENT_ID(a)=ELEMENT_ID(r) AS ids,r.missing AS missing", &[&["true","false","true","NULL"]]).await;
    let r = s.query("MATCH (n) RETURN n").await.unwrap();
    assert_eq!(r.row_count(), 3);
    assert!(matches!(
        r.schema.field(0).data_type(),
        graphfusion::arrow::datatypes::DataType::Struct(_)
    ));
}

#[tokio::test]
async fn path_and_group_references_read_properties_and_labels() {
    let mut s = graph().await;
    check(&mut s, "MATCH p=(a {name:'A'})-[e:E]->(b) FOR r IN ELEMENTS(p) WITH ORDINALITY i RETURN i,r.name AS name,r IS LABELED N AS node,r IS LABELED E AS edge,PROPERTY_EXISTS(r,v) AS has_v ORDER BY i",
        &[&["1","A","true","false","true"], &["2","AB","false","true","false"], &["3","B","true","false","true"]]).await;
    check(&mut s, "MATCH (a {name:'A'})((x)-[edges:E]->(y)){1}(b) FOR e IN edges RETURN e.name AS name,e.w AS w,e IS DIRECTED AS directed,e IS LABELED E AS label", &[&["AB","4","true","true"]]).await;
    check(&mut s, "MATCH (b {name:'B'})-[e:U]-(c {name:'C'}) LET r=e RETURN r IS DIRECTED AS directed,r IS NOT DIRECTED AS undirected,r.name AS name", &[&["false","true","BC"]]).await;
}

#[tokio::test]
async fn references_rebind_nodes_edges_and_preserve_input_multiplicity() {
    let mut s = graph().await;
    check(&mut s, "MATCH (a {name:'A'}) FOR r IN [a,a,NULL] MATCH (r)-[:E]->(b) RETURN r.name AS origin,b.name AS destination", &[&["A","B"], &["A","B"]]).await;
    check(&mut s, "MATCH (a)-[e:E]->(b) LET r=e MATCH (x)-[r]->(y) RETURN x.name AS x,y.name AS y,r.name AS edge", &[&["A","B","AB"]]).await;
    check(
        &mut s,
        "MATCH (a {name:'A'}) LET r=a MATCH ()-[r]->() RETURN r",
        &[],
    )
    .await;
    check(&mut s, "LET r=NULL MATCH (r) RETURN r", &[]).await;
    assert!(s.query("LET r=1 MATCH (r) RETURN r").await.is_err());
}

#[tokio::test]
async fn optional_and_conditional_references_are_real_null_values() {
    let mut s = graph().await;
    check(&mut s, "MATCH (a {name:'C'}) OPTIONAL MATCH (a)-[e:E]->(b) LET r=b RETURN r IS NULL AS missing,PROPERTY_EXISTS(r,name) AS property,r IS LABELED N AS label,SAME(r,a) AS same,ELEMENT_ID(r) AS id", &[&["true","NULL","NULL","NULL","NULL"]]).await;
    check(&mut s, "MATCH (a {name:'A'})(-[e:E]->(b))? LET r=e RETURN r IS NULL AS absent,r.name AS name,r IS DIRECTED AS directed ORDER BY absent", &[&["false","AB","true"], &["true","NULL","NULL"]]).await;
    check(&mut s, "LET r=NULL RETURN r.name AS name,PROPERTY_EXISTS(r,name) AS present,r IS LABELED N AS label,r IS DIRECTED AS directed", &[&["NULL","NULL","NULL","NULL"]]).await;
}

#[tokio::test]
async fn optional_matches_preserve_incoming_reference_bindings() {
    let mut s = graph().await;
    check(&mut s, "MATCH (a {name:'C'}) LET r=a OPTIONAL MATCH (r)-[:E]->(b) RETURN r.name AS name,r IS NULL AS absent,SAME(r,a) AS same,b IS NULL AS missing", &[&["C","false","true","true"]]).await;
    check(&mut s, "MATCH (a {name:'C'}) FOR r IN [a,a,NULL] OPTIONAL MATCH (r)-[:E]->(b) RETURN r.name AS name,r IS NULL AS absent,b.name AS target ORDER BY name", &[&["C","false","NULL"], &["C","false","NULL"], &["NULL","true","NULL"]]).await;
    check(&mut s, "MATCH (a {name:'A'}) LET r=a OPTIONAL MATCH (r)-[:E]->(b) RETURN r.name AS name,b.name AS target", &[&["A","B"]]).await;
    check(
        &mut s,
        "MATCH (a {name:'C'}) LET r=a OPTIONAL MATCH (r)-[:E]->(b) MATCH (r) RETURN r.name AS name",
        &[&["C"]],
    )
    .await;
    check(&mut s, "MATCH (a {name:'C'}) LET r=a OPTIONAL { MATCH (r)-[:U]-(b) MATCH (b)-[:E]->(c) } RETURN r.name AS name,b IS NULL AS missing,c IS NULL AS absent", &[&["C","true","true"]]).await;
    check(&mut s, "MATCH ()-[e:E]->() LET r=e OPTIONAL MATCH (x {name:'C'})-[r]->(y) RETURN r.name AS name,r IS NULL AS absent,x IS NULL AS missing", &[&["AB","false","true"]]).await;
    check(&mut s, "MATCH ()-[e:E]->() LET r=e OPTIONAL MATCH (x {name:'C'})-[r]->(y) MATCH (origin)-[r]->(target) RETURN r.name AS name,origin.name AS origin,target.name AS target", &[&["AB","A","B"]]).await;
    check(
        &mut s,
        "LET r=NULL OPTIONAL MATCH (r)-[:E]->(b) RETURN r.name AS name,r IS NULL AS absent",
        &[&["NULL", "true"]],
    )
    .await;
}

#[tokio::test]
async fn output_reference_aliases_resolve_properties_before_input_names() {
    let mut s = graph().await;
    for query in [
        "MATCH (n) LET r=n RETURN r AS x,n.name AS name ORDER BY x.name DESC",
        "MATCH (n) RETURN n AS x,n.name AS name ORDER BY x.name DESC",
        "MATCH (n),(x {name:'A'}) LET r=n RETURN r AS x,n.name AS name ORDER BY x.name DESC",
        "MATCH (n),(a {name:'A'}) LET x=a,r=n RETURN r AS x,n.name AS name ORDER BY x.name DESC",
        "MATCH (n) LET r=n RETURN r AS n,n.name AS name ORDER BY n.name DESC",
        "MATCH (n) RETURN NULL AS n,n.name AS name ORDER BY n.name,name DESC",
        "MATCH (n) RETURN NULL AS n,n.name AS name ORDER BY ELEMENT_ID(n),name DESC",
        "MATCH (n) LET r=n RETURN NULL AS r,n.name AS name ORDER BY ELEMENT_ID(r),name DESC",
    ] {
        let result = s
            .query(query)
            .await
            .unwrap_or_else(|e| panic!("{query}\n{e}"));
        let names = rows(&result)
            .into_iter()
            .map(|row| row[1].clone())
            .collect::<Vec<_>>();
        assert_eq!(names, ["C", "B", "A"], "{query}");
    }
    assert!(s
        .query("MATCH (n) RETURN 1 AS n ORDER BY n.name")
        .await
        .is_err());
}

#[tokio::test]
async fn projected_reference_aliases_keep_grouping_lookup_columns() {
    let mut s = graph().await;
    for query in [
        "MATCH (n) LET r=n FOR duplicate IN [1,2] RETURN r,r.name AS name,COUNT(*) AS count GROUP BY r ORDER BY name",
        "MATCH (n) LET r=n FOR duplicate IN [1,2] RETURN r AS x,r.name AS name,COUNT(*) AS count GROUP BY x ORDER BY x.name",
        "MATCH (n) LET r=n FOR duplicate IN [1,2] RETURN r AS n,r.name AS name,COUNT(*) AS count GROUP BY n ORDER BY name",
        "MATCH (n) FOR duplicate IN [1,2] RETURN n AS x,n.name AS name,COUNT(*) AS count GROUP BY x ORDER BY name",
        "SELECT r AS x,r.name AS name,COUNT(*) AS count FROM { MATCH (n) FOR duplicate IN [1,2] RETURN n AS r } GROUP BY x HAVING x.v > 0 ORDER BY x.name",
    ] {
        let result = s.query(query).await.unwrap_or_else(|e| panic!("{query}\n{e}"));
        let values = rows(&result).into_iter().map(|row| {
            assert_ne!(row[0], "NULL", "{query}");
            row[1..].to_vec()
        }).collect::<Vec<_>>();
        assert_eq!(values, [["A", "2"], ["B", "2"], ["C", "2"]], "{query}");
    }
}

#[tokio::test]
async fn derived_queries_and_collected_elements_keep_reference_sources() {
    let mut s = graph().await;
    check(
        &mut s,
        "SELECT r.name AS name FROM g { MATCH (n) RETURN n AS r } ORDER BY name",
        &[&["A"], &["B"], &["C"]],
    )
    .await;
    check(
        &mut s,
        "{MATCH (n) RETURN COLLECT_LIST(n) AS nodes} FOR r IN nodes RETURN r.name AS name ORDER BY name",
        &[&["A"], &["B"], &["C"]],
    )
    .await;
    check(&mut s, "{MATCH (n) RETURN n AS r UNION ALL MATCH (n {name:'A'}) RETURN n AS r} RETURN r.name AS name ORDER BY name", &[&["A"], &["A"], &["B"], &["C"]]).await;
}

#[tokio::test]
async fn cross_graph_ids_never_alias_or_rebind() {
    let mut s = graph().await;
    s.run("CREATE GRAPH h ANY; USE GRAPH h INSERT (:N {name:'Other',v:7})")
        .await
        .unwrap();
    check(&mut s, "USE GRAPH g MATCH (a {name:'A'}) USE GRAPH h MATCH (b) FOR r IN [a,b] RETURN r.name AS name,SAME(a,r) AS same,ELEMENT_ID(a)=ELEMENT_ID(r) AS ids ORDER BY name", &[&["A","true","true"], &["Other","false","false"]]).await;
    check(
        &mut s,
        "USE GRAPH g MATCH (a {name:'A'}) LET r=a USE GRAPH h MATCH (r) RETURN r",
        &[],
    )
    .await;
}

#[tokio::test]
async fn reference_aliases_refresh_after_mutations_and_rollback_on_dangling_reads() {
    let mut s = graph().await;
    let r=s.run("MATCH (n {name:'A'}) LET r=n,old=r.v SET n.v=9, n.copy=r.v RETURN old,r.v AS new,r.copy AS copied").await.unwrap();
    let graphfusion::StatementOutput::Query(r) = r.last().unwrap() else {
        panic!("query result")
    };
    assert_eq!(rows(r), vec![vec!["1", "9", "9"]]);
    let r = s
        .run("MATCH (n {name:'A'}) LET r=n MATCH (r) SET r.v=11 RETURN n.v AS n,r.v AS r")
        .await
        .unwrap();
    let graphfusion::StatementOutput::Query(r) = r.last().unwrap() else {
        panic!("query result")
    };
    assert_eq!(rows(r), vec![vec!["11", "11"]]);
    assert!(s
        .run("MATCH (n {name:'C'}) LET r=n DETACH DELETE n RETURN r.name AS name")
        .await
        .is_err());
    check(&mut s, "MATCH (n {name:'C'}) RETURN n.v AS v", &[&["3"]]).await;
}

#[tokio::test]
async fn statically_typed_references_can_be_mutation_targets() {
    let mut s = graph().await;
    let outputs = s
        .run("MATCH (n {name:'A'}) FOR r IN [n,n,NULL] SET r.v=6 RETURN r.v AS v ORDER BY v")
        .await
        .unwrap();
    let graphfusion::StatementOutput::Query(r) = outputs.last().unwrap() else {
        panic!("query");
    };
    assert_eq!(rows(r), vec![vec!["6"], vec!["6"], vec!["NULL"]]);
    assert_eq!(r.affected_elements, 1);
    s.run("MATCH (n {name:'A'}) LET r=n INSERT (r)-[:Added]->(:N {name:'D',v:8})")
        .await
        .unwrap();
    check(
        &mut s,
        "MATCH (n {name:'A'})-[:Added]->(d) RETURN d.name AS name",
        &[&["D"]],
    )
    .await;
    s.run("MATCH ()-[es:E]->{1}() FOR r IN es SET r.w=12 REMOVE r.name")
        .await
        .unwrap();
    check(
        &mut s,
        "MATCH ()-[e:E]->() RETURN e.w AS w,PROPERTY_EXISTS(e,name) AS named",
        &[&["12", "false"]],
    )
    .await;
    assert!(s
        .run("MATCH (n {name:'C'}) LET r=n DETACH DELETE n SET r.v=99")
        .await
        .is_err());
    check(&mut s, "MATCH (n {name:'C'}) RETURN n.v AS v", &[&["3"]]).await;
    s.run("MATCH (n {name:'D'}) LET r=n DETACH DELETE r")
        .await
        .unwrap();
    check(&mut s, "MATCH (n {name:'D'}) RETURN n", &[]).await;
}

#[tokio::test]
async fn reference_domains_avoid_unrelated_property_types_and_group_by_identity() {
    let mut s = graph().await;
    s.run("MATCH ()-[e:E]->() SET e.v='edge value'")
        .await
        .unwrap();
    check(
        &mut s,
        "MATCH (n {name:'A'}) LET r=n RETURN r.v AS v",
        &[&["1"]],
    )
    .await;
    check(
        &mut s,
        "SELECT r.v AS v FROM g { MATCH (n {name:'A'}) RETURN n AS r }",
        &[&["1"]],
    )
    .await;
    check(
        &mut s,
        "{MATCH (n {name:'A'}) RETURN COLLECT_LIST(n) AS ns} FOR r IN ns RETURN r.v AS v",
        &[&["1"]],
    )
    .await;
    check(&mut s, "{MATCH (n {name:'A'}) RETURN n AS r UNION ALL MATCH (n {name:'B'}) RETURN n AS r} RETURN r.v AS v ORDER BY v", &[&["1"],&["2"]]).await;
    check(
        &mut s,
        "MATCH (a {name:'A'})((x)-[:E]->(y)){1}(b) FOR r IN x RETURN r.v AS v",
        &[&["1"]],
    )
    .await;
    check(
        &mut s,
        "MATCH (n) FOR r IN [n,n] RETURN r.name AS name,COUNT(*) AS count GROUP BY r ORDER BY name",
        &[&["A", "2"], &["B", "2"], &["C", "2"]],
    )
    .await;
    let error = s
        .query("MATCH p=()-[:E]->() FOR r IN ELEMENTS(p) RETURN r.v AS value")
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("incompatible types"), "{error}");
    assert!(s
        .query("MATCH (n) LET r=n RETURN r IS DIRECTED AS directed")
        .await
        .is_err());
    assert!(s.query("LET r=3 RETURN r.name AS name").await.is_err());
}

#[tokio::test]
async fn reference_lookups_use_the_explicit_transaction_snapshot() {
    let db = Database::new();
    let mut reader = db.session();
    reader
        .run("CREATE GRAPH g ANY; SESSION SET GRAPH g; INSERT (:N {name:'A',v:1})")
        .await
        .unwrap();
    let mut writer = db.session();
    writer.execute("SESSION SET GRAPH g").unwrap();
    reader.run("START TRANSACTION READ ONLY").await.unwrap();
    let q = "MATCH (n) LET r=n RETURN r.v AS v";
    check(&mut reader, q, &[&["1"]]).await;
    writer.run("MATCH (n) SET n.v=2").await.unwrap();
    check(&mut reader, q, &[&["1"]]).await;
    reader.run("ROLLBACK").await.unwrap();
    check(&mut reader, q, &[&["2"]]).await;
    reader.run("START TRANSACTION").await.unwrap();
    assert!(reader
        .run("MATCH (n) LET r=n SET r.v=9 RETURN 1/0 AS bad")
        .await
        .is_err());
    assert!(reader.run("COMMIT").await.is_err());
    reader.run("ROLLBACK").await.unwrap();
    check(&mut reader, q, &[&["2"]]).await;
}
