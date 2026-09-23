use datafusion::common::ScalarValue;
use graphfusion::{Database, QueryResult, Session};

async fn graph() -> Session {
    let db = Database::new();
    let mut s = db.session();
    s.execute("CREATE GRAPH g ANY; SESSION SET GRAPH g")
        .unwrap();
    s.run("INSERT (a:N {name: 'A'}), (b:N {name: 'B'}), (c:N {name: 'C'}), (d:N {name: 'D'}), (a)-[:E]->(b), (a)-[:E]->(c), (b)-[:E]->(d), (c)-[:E]->(d), (d)-[:E]->(a), (a)-[:Loop]->(a)").await.unwrap();
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
                    .collect::<Vec<_>>()
            })
        })
        .collect()
}

#[tokio::test]
async fn recursive_edges_and_zero_repetitions_execute_in_datafusion() {
    let mut s = graph().await;
    let result = s.query("MATCH p = (a {name:'A'})-[:E]->{0,2}(b) RETURN b.name AS name, PATH_LENGTH(p) AS hops ORDER BY hops, name").await.unwrap();
    assert_eq!(
        rows(&result),
        vec![
            vec!["A", "0"],
            vec!["B", "1"],
            vec!["C", "1"],
            vec!["D", "2"],
            vec!["D", "2"]
        ]
    );
    assert!(
        result.physical_plan.contains("RecursiveQueryExec"),
        "{}",
        result.physical_plan
    );
    assert!(
        result.physical_plan.contains("WorkTableExec"),
        "{}",
        result.physical_plan
    );
    let result = s
        .query("MATCH p = (a {name:'A'})-[:E]->{2}(a) RETURN PATH_LENGTH(p) AS hops")
        .await
        .unwrap();
    assert_eq!(result.row_count(), 0);
}

#[tokio::test]
async fn fixed_path_modes_and_values() {
    let mut s = graph().await;
    for (mode, n) in [("WALK", 1), ("TRAIL", 0), ("SIMPLE", 0), ("ACYCLIC", 0)] {
        let q = format!("MATCH REPEATABLE ELEMENTS p = {mode} (a {{name:'A'}})-[:Loop]->(a)-[:Loop]->(a) RETURN PATH_LENGTH(p) AS hops");
        assert_eq!(s.query(&q).await.unwrap().row_count(), n, "{q}");
    }
    for (mode, n) in [("WALK", 1), ("TRAIL", 1), ("SIMPLE", 1), ("ACYCLIC", 0)] {
        let q = format!("MATCH REPEATABLE ELEMENTS p = {mode} (a {{name:'A'}})-[:Loop]->(a) RETURN p, PATH_LENGTH(p) AS hops");
        assert_eq!(s.query(&q).await.unwrap().row_count(), n, "{q}");
    }
    let result = s
        .query("MATCH p = (a {name:'A'}) RETURN PATH_LENGTH(p) AS hops")
        .await
        .unwrap();
    assert_eq!(rows(&result), vec![vec!["0"]]);
}

#[tokio::test]
async fn shortest_ties_and_partitioned_selection() {
    let mut s = graph().await;
    let result = s.query("MATCH p = ALL SHORTEST (a {name:'A'})-[:E]->{1,4}(b) RETURN b.name AS name, PATH_LENGTH(p) AS hops ORDER BY name, hops").await.unwrap();
    assert_eq!(
        rows(&result),
        vec![
            vec!["A", "3"],
            vec!["A", "3"],
            vec!["B", "1"],
            vec!["C", "1"],
            vec!["D", "2"],
            vec!["D", "2"]
        ]
    );
    let result = s.query("MATCH p = ANY SHORTEST (a {name:'A'})-[:E]->{1,4}(b) RETURN b.name AS name, PATH_LENGTH(p) AS hops ORDER BY name").await.unwrap();
    assert_eq!(
        rows(&result),
        vec![
            vec!["A", "3"],
            vec!["B", "1"],
            vec!["C", "1"],
            vec!["D", "2"]
        ]
    );
    let result = s.query("FOR x IN [1,1] MATCH p = ANY SHORTEST (a {name:'A'})-[:E]->{1,2}(b {name:'D'}) RETURN x, PATH_LENGTH(p) AS hops").await.unwrap();
    assert_eq!(rows(&result), vec![vec!["1", "2"], vec!["1", "2"]]);
}

#[tokio::test]
async fn inline_path_predicates_bind_forward_references_before_selection() {
    let mut s = graph().await;
    s.run("MATCH (a {name:'A'}), (d {name:'D'}) INSERT (a)-[:E]->(d)")
        .await
        .unwrap();
    // The direct A->D path fails the intermediate-node predicate. Selection
    // must retain A->B->D, including one result for each duplicate input row.
    let result = s.query("FOR x IN [1,1] MATCH p = ANY SHORTEST (a WHERE a.name = 'A' AND a.name < m.name)-[:E]->{0,2}(m {name:'B'})-[e:E WHERE m.name < b.name]->(b {name:'D'}) RETURN x, PATH_LENGTH(p) AS hops").await.unwrap();
    assert_eq!(rows(&result), vec![vec!["1", "2"], vec!["1", "2"]]);

    // Unselected patterns retain MATCH-wide forward references across paths.
    let result = s.query("MATCH p = (a WHERE a.name = b.name)-[:E]->{0,1}(c), (b {name:'A'}) RETURN c.name AS name ORDER BY name").await.unwrap();
    assert_eq!(
        rows(&result),
        vec![vec!["A"], vec!["B"], vec!["C"], vec!["D"]]
    );
}

#[tokio::test]
async fn finite_unbounded_modes_and_explicit_walk_bounds() {
    let mut s = graph().await;
    for (mode, max) in [("TRAIL", 5), ("SIMPLE", 3), ("ACYCLIC", 2)] {
        let q = format!("MATCH REPEATABLE ELEMENTS p = {mode} (a {{name:'A'}})-[:E]->+() RETURN MAX(PATH_LENGTH(p)) AS longest");
        assert_eq!(
            rows(&s.query(&q).await.unwrap()),
            vec![vec![max.to_string()]],
            "{q}"
        );
    }
    assert!(s
        .query("MATCH REPEATABLE ELEMENTS WALK ()-[:E]->+() RETURN 1 AS value")
        .await
        .unwrap_err()
        .to_string()
        .contains("unbounded"));
    assert!(s
        .query("MATCH REPEATABLE ELEMENTS WALK ()-[:E]->{257}() RETURN 1 AS value")
        .await
        .unwrap_err()
        .to_string()
        .contains("256-hop"));
    let result = s.query("MATCH REPEATABLE ELEMENTS p = WALK (a {name:'A'})-[:Loop]->{0,3}(a) RETURN PATH_LENGTH(p) AS hops ORDER BY hops").await.unwrap();
    assert_eq!(
        rows(&result),
        vec![vec!["0"], vec!["1"], vec!["2"], vec!["3"]]
    );
}

#[tokio::test]
async fn optional_paths_and_multiple_segments() {
    let mut s = graph().await;
    let result = s.query("MATCH (a {name:'A'}) OPTIONAL MATCH p = (a)-[:Missing]->+(b) RETURN PATH_LENGTH(p) AS hops, p IS NULL AS absent").await.unwrap();
    assert_eq!(rows(&result), vec![vec!["NULL", "true"]]);
    let result = s.query("MATCH REPEATABLE ELEMENTS p = TRAIL (a {name:'A'})-[:E]->{1,2}(b)-[:E]->{1,2}(c) RETURN b.name AS b, c.name AS c, PATH_LENGTH(p) AS hops ORDER BY b, c, hops").await.unwrap();
    assert_eq!(result.row_count(), 8, "{:?}", rows(&result));
    let result = s.query("MATCH p = (a {name:'A'})-[:E]->{0,1}() RETURN PATH_LENGTH(p) AS hops UNION ALL MATCH p = (a {name:'B'})-[:E]->{0,1}() RETURN PATH_LENGTH(p) AS hops").await.unwrap();
    assert_eq!(result.row_count(), 5); // independent recursive work tables in sibling branches
    for (mode, count) in [("DIFFERENT EDGES", 0), ("REPEATABLE ELEMENTS", 1)] {
        let q = format!("MATCH {mode} p = (a {{name:'A'}})-[:Loop]->{{1}}(a), q = (a)-[:Loop]->{{1}}(a) RETURN PATH_LENGTH(p) AS first, PATH_LENGTH(q) AS second");
        assert_eq!(s.query(&q).await.unwrap().row_count(), count);
    }
}

#[tokio::test]
async fn group_variables_and_path_elements_are_ordered_references() {
    let mut s = graph().await;
    let nulls = s.query("LET e = NULL RETURN PATH_LENGTH(NULL) AS hops, ELEMENTS(NULL) AS elements, ELEMENT_ID(e) AS id").await.unwrap();
    assert_eq!(nulls.row_count(), 1);
    for batch in &nulls.batches {
        for column in batch.columns() {
            assert_eq!(column.null_count(), batch.num_rows());
        }
    }
    let result = s.query("MATCH p = (a {name:'A'})-[es:E]->{0,2}(b) FOR e IN es RETURN b.name AS name, ELEMENT_ID(e) AS id, PATH_LENGTH(p) AS hops ORDER BY name, id").await.unwrap();
    assert_eq!(result.row_count(), 6);
    assert!(rows(&result).iter().all(|r| r[1].contains(":e:")));
    let result = s
        .query(
            "MATCH p = (a {name:'A'})-[es:Loop]->{0,1}(a) FOR e IN es RETURN ELEMENT_ID(e) AS id",
        )
        .await
        .unwrap();
    assert_eq!(result.row_count(), 1); // zero repetitions contribute an empty list
    let result = s.query("MATCH p = (a {name:'A'})-[:Loop]->(a) FOR x IN ELEMENTS(p) WITH ORDINALITY i RETURN ELEMENT_ID(x) AS id, i ORDER BY i").await.unwrap();
    let values = rows(&result);
    assert_eq!(values.len(), 3);
    assert_eq!(values[0][0], values[2][0]);
    assert!(values[0][0].contains(":n:"));
    assert!(values[1][0].contains(":e:"));
    assert_eq!(
        values.iter().map(|r| r[1].as_str()).collect::<Vec<_>>(),
        vec!["1", "2", "3"]
    );
    let result = s
        .query("MATCH p = (a {name:'A'})-[es:E WHERE a.name = 'A']->{1,2}(b) RETURN COUNT(es) AS n")
        .await
        .unwrap();
    assert_eq!(rows(&result), vec![vec!["4"]]);
    let result = s.query("MATCH (a {name:'A'}) OPTIONAL MATCH p = (a)-[:Missing]->(b) FOR e IN ELEMENTS(p) RETURN ELEMENT_ID(e) AS id").await.unwrap();
    assert_eq!(result.row_count(), 0);
    let result = s.query("MATCH p = (a {name:'A'})-[:E]->(b)-[es:E]->{1,2}(c) FOR e IN es RETURN ELEMENT_ID(e) AS id").await.unwrap();
    assert_eq!(result.row_count(), 6); // group excludes the fixed prefix edge
}

#[tokio::test]
async fn counted_selection_parameters_and_post_filters() {
    use graphfusion::Value;
    let mut s = graph().await;
    s.set_parameter("k", Value::Integer(3)).unwrap();
    let result = s.query("MATCH REPEATABLE ELEMENTS p = SHORTEST $k (a {name:'A'})-[:E]->{1,5}(b {name:'D'}) RETURN PATH_LENGTH(p) AS hops ORDER BY hops").await.unwrap();
    assert_eq!(rows(&result), vec![vec!["2"], vec!["2"], vec!["5"]]);
    let result = s.query("MATCH REPEATABLE ELEMENTS p = SHORTEST 2 GROUPS (a {name:'A'})-[:E]->{1,5}(b {name:'D'}) RETURN PATH_LENGTH(p) AS hops ORDER BY hops").await.unwrap();
    assert_eq!(
        rows(&result),
        vec![
            vec!["2"],
            vec!["2"],
            vec!["5"],
            vec!["5"],
            vec!["5"],
            vec!["5"]
        ]
    );
    // Graph-pattern WHERE is a post-selection filter, not a new shortest search.
    let result = s.query("MATCH REPEATABLE ELEMENTS p = ALL SHORTEST (a {name:'A'})-[:E]->{1,5}(b {name:'D'}) WHERE PATH_LENGTH(p) > 2 RETURN PATH_LENGTH(p) AS hops").await.unwrap();
    assert_eq!(result.row_count(), 0);
    s.set_parameter("k", Value::Integer(0)).unwrap();
    assert!(s.query("MATCH ANY $k () RETURN 1 AS n").await.is_err());
    s.set_parameter("k", Value::Float(1.0)).unwrap();
    assert!(s.query("MATCH ANY $k () RETURN 1 AS n").await.is_err());
    assert!(s.query("MATCH p = (p) RETURN p").await.is_err());
    assert!(s
        .query("MATCH (e) MATCH ()-[e]->{1,2}() RETURN e")
        .await
        .is_err());
    assert!(s.query("RETURN PATH_LENGTH([1,2]) AS n").await.is_err());
}

#[tokio::test]
async fn resource_errors_roll_back_the_whole_statement() {
    use graphfusion::{QueryLimits, TransactionStatus};
    let mut s = graph().await;
    s.set_query_limits(QueryLimits {
        max_path_hops: 1,
        ..Default::default()
    })
    .unwrap();
    let error = s
        .run("INSERT (:New) MATCH REPEATABLE ELEMENTS ()-[:E]->{2}() RETURN 1 AS n")
        .await
        .unwrap_err();
    assert!(error.to_string().contains("1-hop"), "{error}");
    assert_eq!(
        s.query("MATCH (n:New) RETURN COUNT(*) AS n")
            .await
            .unwrap()
            .row_count(),
        1
    );
    assert_eq!(
        rows(&s.query("MATCH (n:New) RETURN COUNT(*) AS n").await.unwrap()),
        vec![vec!["0"]]
    );
    s.set_query_limits(QueryLimits {
        memory_limit_bytes: 1,
        ..Default::default()
    })
    .unwrap();
    s.execute("START TRANSACTION").unwrap();
    let error = s
        .query("MATCH p = ()-[:E]->{0,3}() RETURN p")
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("memory") || error.to_string().contains("Memory"),
        "{error}"
    );
    assert!(matches!(
        s.transaction_status(),
        TransactionStatus::Failed { .. }
    ));
    s.execute("ROLLBACK").unwrap();
    s.set_query_limits(QueryLimits::default()).unwrap();
    assert_eq!(
        s.query("MATCH (n) RETURN n.name AS name")
            .await
            .unwrap()
            .row_count(),
        4
    );
}

// A small independent exhaustive oracle checks complete path bags, rather than
// just cardinality: parallel edges, direction, zero hops, isolated nodes, loops,
// and both kinds of repeated-element restriction all affect the identity list.
#[tokio::test]
async fn path_bags_match_exhaustive_walks_on_mixed_cyclic_graph() {
    use graphfusion::{
        arrow::{
            array::{Array, ArrayRef, ListArray, StructArray, UInt64Array},
            datatypes::{DataType, Field, Schema},
            record_batch::RecordBatch,
        },
        graph::{EdgeTable, GraphData, NodeTable, DESTINATION, ID, SOURCE},
    };
    use std::{
        collections::{BTreeMap, BTreeSet},
        sync::Arc,
    };
    type Bag = BTreeMap<(Vec<u64>, Vec<u64>), usize>;
    let db = Database::new();
    let mut s = db.session();
    s.execute("CREATE GRAPH g ANY; SESSION SET GRAPH g")
        .unwrap();
    let schema = Arc::new(Schema::new(vec![Field::new(ID, DataType::UInt64, false)]));
    let nodes = NodeTable::try_new(
        vec![],
        schema.clone(),
        vec![
            RecordBatch::try_new(schema, vec![Arc::new(UInt64Array::from(vec![0, 1, 2]))]).unwrap(),
        ],
    )
    .unwrap();
    let directed = vec![(10, 0, 1), (11, 0, 1), (12, 1, 0), (13, 1, 1)];
    let undirected = vec![(20, 0, 1), (21, 2, 2)];
    let edge_table = |directed: bool, rows: &[(u64, u64, u64)]| {
        let schema = Arc::new(Schema::new(vec![
            Field::new(ID, DataType::UInt64, false),
            Field::new(SOURCE, DataType::UInt64, false),
            Field::new(DESTINATION, DataType::UInt64, false),
        ]));
        let values: Vec<ArrayRef> = vec![
            Arc::new(UInt64Array::from(
                rows.iter().map(|r| r.0).collect::<Vec<_>>(),
            )),
            Arc::new(UInt64Array::from(
                rows.iter().map(|r| r.1).collect::<Vec<_>>(),
            )),
            Arc::new(UInt64Array::from(
                rows.iter().map(|r| r.2).collect::<Vec<_>>(),
            )),
        ];
        EdgeTable::try_new(
            vec![],
            directed,
            schema.clone(),
            vec![RecordBatch::try_new(schema, values).unwrap()],
        )
        .unwrap()
    };
    s.replace_graph_data(
        GraphData::try_new(
            vec![nodes],
            vec![edge_table(true, &directed), edge_table(false, &undirected)],
        )
        .unwrap(),
    )
    .unwrap();
    for (syntax, orient) in [("-[]->", 0), ("<-[]-", 1), ("-[]-", 2), ("~[]~", 3)] {
        let mut arcs = Vec::new();
        for &(id, a, b) in &directed {
            match orient {
                0 => arcs.push((id, a, b)),
                1 => arcs.push((id, b, a)),
                2 => {
                    arcs.push((id, a, b));
                    if a != b {
                        arcs.push((id, b, a));
                    }
                }
                _ => (),
            }
        }
        if orient >= 2 {
            for &(id, a, b) in &undirected {
                arcs.push((id, a, b));
                if a != b {
                    arcs.push((id, b, a));
                }
            }
        }
        let mut all: Vec<(Vec<u64>, Vec<u64>)> = (0..3).map(|n| (vec![n], vec![])).collect();
        let mut frontier = all.clone();
        for _ in 0..3 {
            let mut next = Vec::new();
            for (ns, es) in frontier {
                for &(id, a, b) in &arcs {
                    if ns.last() == Some(&a) {
                        let mut ns = ns.clone();
                        let mut es = es.clone();
                        ns.push(b);
                        es.push(id);
                        next.push((ns, es));
                    }
                }
            }
            all.extend(next.clone());
            frontier = next;
        }
        for mode in ["WALK", "TRAIL", "SIMPLE", "ACYCLIC"] {
            for different in [false, true] {
                let mut expected = Bag::new();
                for (ns, es) in &all {
                    let unique =
                        |ids: &[u64]| ids.iter().collect::<BTreeSet<_>>().len() == ids.len();
                    if (different || mode == "TRAIL") && !unique(es) {
                        continue;
                    }
                    if mode == "ACYCLIC" && !unique(ns) {
                        continue;
                    }
                    if mode == "SIMPLE"
                        && !unique(ns)
                        && !(ns.first() == ns.last() && unique(&ns[..ns.len() - 1]))
                    {
                        continue;
                    }
                    *expected.entry((ns.clone(), es.clone())).or_default() += 1;
                }
                let q = format!(
                    "MATCH {} p = {mode} (){syntax}{{0,3}}() RETURN p",
                    if different {
                        "DIFFERENT EDGES"
                    } else {
                        "REPEATABLE ELEMENTS"
                    }
                );
                let result = s.query(&q).await.unwrap();
                let mut actual = Bag::new();
                for b in &result.batches {
                    let paths = b.column(0).as_any().downcast_ref::<StructArray>().unwrap();
                    let ids = |name: &str, row: usize| {
                        let list = paths
                            .column_by_name(name)
                            .unwrap()
                            .as_any()
                            .downcast_ref::<ListArray>()
                            .unwrap()
                            .value(row);
                        list.as_any()
                            .downcast_ref::<UInt64Array>()
                            .unwrap()
                            .values()
                            .to_vec()
                    };
                    for row in 0..paths.len() {
                        *actual
                            .entry((ids("__gql_path_nodes", row), ids("__gql_path_edges", row)))
                            .or_default() += 1;
                    }
                }
                assert_eq!(actual, expected, "{q}");
            }
        }
    }
}
