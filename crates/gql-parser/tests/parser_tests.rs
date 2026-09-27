use graphfusion_gql_parser::{
    parse, ApproximateNumericTypeKind, BinaryOp, BindingTableExpression, BindingTableName,
    BindingVariableDefinition, BooleanTypeKind, ByteStringTypeKind, CharacterStringTypeKind,
    CreateGraphType, CreateGraphTypeSource, DateTimeFunctionKind, Direction, ExactNumericTypeKind,
    Expr, ForOrdinalityOrOffsetKind, GraphExpression, GraphTypeElement, GraphTypeReference,
    Identifier, LabelExpression, ListValueTypeName, Literal, MatchMode, NullOrdering,
    NumericFunctionKind, PathMode, PathOrPaths, PathPatternAlternation, PathPatternFactor,
    PathPatternPrefix, PathPatternQuantifier, PathSearchPrefix, ProcedureCall, ProcedureReference,
    QueryClause, QuerySetOperator, RemoveItem, ResultKind, SchemaReference, SessionResetTarget,
    SessionSetTarget, SetItem, SetQuantifier, SortDirection, SourceDestinationKind, Statement,
    StringLengthUnit, SubstringSide, TemporalTypeKind, TransactionAccessMode, TrimSpec, UnaryOp,
    UnsignedIntegerSpecification, ValueType, YieldItem,
};

fn ident(value: &str) -> Identifier {
    Identifier::new(value)
}

fn graph_name(parts: &[&str]) -> graphfusion_gql_parser::GraphName {
    graphfusion_gql_parser::GraphName {
        absolute: false,
        current_schema: false,
        home_schema: false,
        parent_levels: 0,
        parts: parts.iter().map(|part| ident(part)).collect(),
    }
}

fn absolute_graph_name(parts: &[&str]) -> graphfusion_gql_parser::GraphName {
    graphfusion_gql_parser::GraphName {
        absolute: true,
        current_schema: false,
        home_schema: false,
        parent_levels: 0,
        parts: parts.iter().map(|part| ident(part)).collect(),
    }
}

fn current_schema_graph_name(parts: &[&str]) -> graphfusion_gql_parser::GraphName {
    graphfusion_gql_parser::GraphName {
        absolute: false,
        current_schema: true,
        home_schema: false,
        parent_levels: 0,
        parts: parts.iter().map(|part| ident(part)).collect(),
    }
}

fn home_schema_graph_name(parts: &[&str]) -> graphfusion_gql_parser::GraphName {
    graphfusion_gql_parser::GraphName {
        absolute: false,
        current_schema: false,
        home_schema: true,
        parent_levels: 0,
        parts: parts.iter().map(|part| ident(part)).collect(),
    }
}

fn parent_graph_name(levels: usize, parts: &[&str]) -> graphfusion_gql_parser::GraphName {
    graphfusion_gql_parser::GraphName {
        absolute: false,
        current_schema: false,
        home_schema: false,
        parent_levels: levels,
        parts: parts.iter().map(|part| ident(part)).collect(),
    }
}

fn binding_table_name(parts: &[&str]) -> BindingTableName {
    BindingTableName {
        absolute: false,
        current_schema: false,
        home_schema: false,
        parent_levels: 0,
        parts: parts.iter().map(|part| ident(part)).collect(),
    }
}

fn absolute_binding_table_name(parts: &[&str]) -> BindingTableName {
    BindingTableName {
        absolute: true,
        current_schema: false,
        home_schema: false,
        parent_levels: 0,
        parts: parts.iter().map(|part| ident(part)).collect(),
    }
}

fn current_schema_binding_table_name(parts: &[&str]) -> BindingTableName {
    BindingTableName {
        absolute: false,
        current_schema: true,
        home_schema: false,
        parent_levels: 0,
        parts: parts.iter().map(|part| ident(part)).collect(),
    }
}

fn home_schema_binding_table_name(parts: &[&str]) -> BindingTableName {
    BindingTableName {
        absolute: false,
        current_schema: false,
        home_schema: true,
        parent_levels: 0,
        parts: parts.iter().map(|part| ident(part)).collect(),
    }
}

fn parent_binding_table_name(levels: usize, parts: &[&str]) -> BindingTableName {
    BindingTableName {
        absolute: false,
        current_schema: false,
        home_schema: false,
        parent_levels: levels,
        parts: parts.iter().map(|part| ident(part)).collect(),
    }
}

fn schema_name(parts: &[&str]) -> graphfusion_gql_parser::SchemaName {
    graphfusion_gql_parser::SchemaName {
        absolute: false,
        current_schema: false,
        home_schema: false,
        parent_levels: 0,
        parts: parts.iter().map(|part| ident(part)).collect(),
    }
}

fn graph_type_name(parts: &[&str]) -> graphfusion_gql_parser::GraphTypeName {
    graphfusion_gql_parser::GraphTypeName {
        absolute: false,
        current_schema: false,
        home_schema: false,
        parent_levels: 0,
        parts: parts.iter().map(|part| ident(part)).collect(),
    }
}

fn absolute_graph_type_name(parts: &[&str]) -> graphfusion_gql_parser::GraphTypeName {
    graphfusion_gql_parser::GraphTypeName {
        absolute: true,
        current_schema: false,
        home_schema: false,
        parent_levels: 0,
        parts: parts.iter().map(|part| ident(part)).collect(),
    }
}

fn current_schema_graph_type_name(parts: &[&str]) -> graphfusion_gql_parser::GraphTypeName {
    graphfusion_gql_parser::GraphTypeName {
        absolute: false,
        current_schema: true,
        home_schema: false,
        parent_levels: 0,
        parts: parts.iter().map(|part| ident(part)).collect(),
    }
}

fn home_schema_graph_type_name(parts: &[&str]) -> graphfusion_gql_parser::GraphTypeName {
    graphfusion_gql_parser::GraphTypeName {
        absolute: false,
        current_schema: false,
        home_schema: true,
        parent_levels: 0,
        parts: parts.iter().map(|part| ident(part)).collect(),
    }
}

fn parent_graph_type_name(levels: usize, parts: &[&str]) -> graphfusion_gql_parser::GraphTypeName {
    graphfusion_gql_parser::GraphTypeName {
        absolute: false,
        current_schema: false,
        home_schema: false,
        parent_levels: levels,
        parts: parts.iter().map(|part| ident(part)).collect(),
    }
}

fn graph_type_ref(parts: &[&str]) -> GraphTypeReference {
    GraphTypeReference::Name(graph_type_name(parts))
}

fn procedure_ref(parts: &[&str]) -> ProcedureReference {
    ProcedureReference::Name(graphfusion_gql_parser::ProcedureName {
        absolute: false,
        current_schema: false,
        home_schema: false,
        parent_levels: 0,
        parts: parts.iter().map(|part| ident(part)).collect(),
    })
}

fn absolute_procedure_ref(parts: &[&str]) -> ProcedureReference {
    ProcedureReference::Name(graphfusion_gql_parser::ProcedureName {
        absolute: true,
        current_schema: false,
        home_schema: false,
        parent_levels: 0,
        parts: parts.iter().map(|part| ident(part)).collect(),
    })
}

fn current_schema_procedure_ref(parts: &[&str]) -> ProcedureReference {
    ProcedureReference::Name(graphfusion_gql_parser::ProcedureName {
        absolute: false,
        current_schema: true,
        home_schema: false,
        parent_levels: 0,
        parts: parts.iter().map(|part| ident(part)).collect(),
    })
}

fn home_schema_procedure_ref(parts: &[&str]) -> ProcedureReference {
    ProcedureReference::Name(graphfusion_gql_parser::ProcedureName {
        absolute: false,
        current_schema: false,
        home_schema: true,
        parent_levels: 0,
        parts: parts.iter().map(|part| ident(part)).collect(),
    })
}

fn parent_procedure_ref(levels: usize, parts: &[&str]) -> ProcedureReference {
    ProcedureReference::Name(graphfusion_gql_parser::ProcedureName {
        absolute: false,
        current_schema: false,
        home_schema: false,
        parent_levels: levels,
        parts: parts.iter().map(|part| ident(part)).collect(),
    })
}

fn first_match(
    query: &graphfusion_gql_parser::QueryStatement,
) -> &graphfusion_gql_parser::MatchClause {
    let QueryClause::Match(match_clause) = &query.body.clauses[0] else {
        panic!("expected match clause");
    };
    match_clause
}

#[test]
fn parses_match_return_with_pattern_where_order_and_limit() {
    let program = parse(
        "MATCH (p:Person {name: 'Alice'})-[r:KNOWS]->(f:Person) \
         WHERE f.age >= 18 AND p.active = true \
         RETURN DISTINCT f.name AS friend, r.since AS since ORDER BY r.since DESC LIMIT 10;",
    )
    .unwrap();

    assert_eq!(program.statements.len(), 1);
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query statement");
    };

    assert_eq!(query.body.clauses.len(), 1);
    let match_clause = first_match(query);
    let pattern = &match_clause.patterns[0];
    assert_eq!(pattern.start.variable, Some(ident("p")));
    assert_eq!(pattern.start.labels, vec![ident("Person")]);
    assert_eq!(
        pattern.start.properties.as_ref().unwrap().entries[0].0,
        ident("name")
    );
    assert_eq!(
        pattern.start.properties.as_ref().unwrap().entries[0].1,
        Expr::Literal(Literal::String("Alice".to_owned()))
    );
    assert_eq!(pattern.chains.len(), 1);
    assert_eq!(pattern.chains[0].relationship.direction, Direction::Right);
    assert_eq!(pattern.chains[0].relationship.variable, Some(ident("r")));
    assert_eq!(pattern.chains[0].relationship.labels, vec![ident("KNOWS")]);
    assert!(matches!(
        match_clause.where_clause,
        Some(Expr::Binary {
            op: BinaryOp::And,
            ..
        })
    ));

    assert!(query.body.result_clause.distinct);
    assert_eq!(query.body.result_clause.items.len(), 2);
    assert_eq!(
        query.body.result_clause.items[0].alias,
        Some(ident("friend"))
    );
    assert_eq!(query.body.order_by[0].direction, Some(SortDirection::Desc));
    assert_eq!(
        query.body.limit,
        Some(UnsignedIntegerSpecification::Literal(10))
    );
}

#[test]
fn parses_delimited_identifiers() {
    let program = parse(
        "MATCH (\"select\":`Person Label` {\"display name\": 'Alice'}) \
         RETURN \"select\".\"display name\" AS `display alias`, \
                \"quoted \"\"name\"\"\" AS \"quoted alias\"",
    )
    .unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query statement");
    };

    let match_clause = first_match(query);
    let start = &match_clause.patterns[0].start;
    assert_eq!(start.variable, Some(ident("select")));
    assert_eq!(start.labels, vec![ident("Person Label")]);
    assert_eq!(
        start.properties.as_ref().unwrap().entries[0].0,
        ident("display name")
    );

    let Expr::Property { base, key } = &query.body.result_clause.items[0].expr else {
        panic!("expected property expression");
    };
    assert_eq!(base.as_ref(), &Expr::Identifier(ident("select")));
    assert_eq!(key, &ident("display name"));
    assert_eq!(
        query.body.result_clause.items[0].alias,
        Some(ident("display alias"))
    );

    assert_eq!(
        query.body.result_clause.items[1].expr,
        Expr::Literal(Literal::String("quoted \"name\"".to_owned()))
    );
    assert_eq!(
        query.body.result_clause.items[1].alias,
        Some(ident("quoted alias"))
    );
}

#[test]
fn parses_escaped_and_no_escape_delimited_identifiers() {
    let program = parse(
        r#"RETURN "line\nidentifier".field AS @`raw\nalias`, @`raw\nidentifier` AS "escaped\u0020alias""#,
    )
    .unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query statement");
    };
    let items = &query.body.result_clause.items;

    let Expr::Property { base, key } = &items[0].expr else {
        panic!("expected property expression");
    };
    assert_eq!(base.as_ref(), &Expr::Identifier(ident("line\nidentifier")));
    assert_eq!(key, &ident("field"));
    assert_eq!(items[0].alias, Some(ident(r"raw\nalias")));
    assert_eq!(items[1].expr, Expr::Identifier(ident(r"raw\nidentifier")));
    assert_eq!(items[1].alias, Some(ident("escaped alias")));
}

#[test]
fn parses_unicode_regular_identifiers() {
    let program = parse("MATCH (节点:用户 {名字: 'Alice'}) RETURN 节点.名字 AS 名字").unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query statement");
    };

    let match_clause = first_match(query);
    let start = &match_clause.patterns[0].start;
    assert_eq!(start.variable, Some(ident("节点")));
    assert_eq!(start.labels, vec![ident("用户")]);
    assert_eq!(
        start.properties.as_ref().unwrap().entries[0].0,
        ident("名字")
    );

    let Expr::Property { base, key } = &query.body.result_clause.items[0].expr else {
        panic!("expected property expression");
    };
    assert_eq!(base.as_ref(), &Expr::Identifier(ident("节点")));
    assert_eq!(key, &ident("名字"));
    assert_eq!(query.body.result_clause.items[0].alias, Some(ident("名字")));
}

#[test]
fn parses_match_graph_pattern_yield_clauses() {
    let program = parse(
        "MATCH (n)-[e]->(m) WHERE n.active = true YIELD n, e RETURN n; \
         MATCH path = (a)-[r]->(b) YIELD path, r RETURN path",
    )
    .unwrap();

    let Statement::Query(first) = &program.statements[0] else {
        panic!("expected query");
    };
    let yield_clause = first_match(first).yield_clause.as_ref().unwrap();
    assert_eq!(yield_clause.items.len(), 2);
    assert!(matches!(
        yield_clause.items[0],
        YieldItem::Item {
            ref name,
            alias: None
        } if name == &ident("n")
    ));

    let Statement::Query(second) = &program.statements[1] else {
        panic!("expected query");
    };
    let path_yield = first_match(second).yield_clause.as_ref().unwrap();
    assert_eq!(path_yield.items.len(), 2);
    assert!(matches!(
        path_yield.items[0],
        YieldItem::Item {
            ref name,
            alias: None
        } if name == &ident("path")
    ));
}

#[test]
fn parses_match_modes() {
    let program = parse(
        "MATCH REPEATABLE ELEMENT BINDINGS (a)-[:KNOWS]->(b) RETURN a; \
         MATCH REPEATABLE ELEMENTS (a)-[:KNOWS]->(b) RETURN a; \
         MATCH DIFFERENT EDGE BINDINGS (a)-[e]->(b) RETURN e; \
         MATCH DIFFERENT EDGES (a)-[e]->(b) RETURN e; \
         MATCH DIFFERENT RELATIONSHIP BINDINGS (a)-[r]->(b) RETURN r; \
         MATCH DIFFERENT RELATIONSHIPS (a)-[r]->(b) RETURN r",
    )
    .unwrap();

    let Statement::Query(first) = &program.statements[0] else {
        panic!("expected query");
    };
    assert_eq!(
        first_match(first).mode,
        Some(MatchMode::RepeatableElements { bindings: true })
    );

    let Statement::Query(second) = &program.statements[1] else {
        panic!("expected query");
    };
    assert_eq!(
        first_match(second).mode,
        Some(MatchMode::RepeatableElements { bindings: false })
    );

    let Statement::Query(third) = &program.statements[2] else {
        panic!("expected query");
    };
    assert_eq!(
        first_match(third).mode,
        Some(MatchMode::DifferentEdges { bindings: true })
    );

    let Statement::Query(fourth) = &program.statements[3] else {
        panic!("expected query");
    };
    assert_eq!(
        first_match(fourth).mode,
        Some(MatchMode::DifferentEdges { bindings: false })
    );

    let Statement::Query(fifth) = &program.statements[4] else {
        panic!("expected query");
    };
    assert_eq!(
        first_match(fifth).mode,
        Some(MatchMode::DifferentEdges { bindings: true })
    );

    let Statement::Query(sixth) = &program.statements[5] else {
        panic!("expected query");
    };
    assert_eq!(
        first_match(sixth).mode,
        Some(MatchMode::DifferentEdges { bindings: false })
    );
}

#[test]
fn parses_use_graph_optional_match_and_left_relationship() {
    let program = parse("USE GRAPH social OPTIONAL MATCH (a)<-[e:LIKES]-(b) RETURN a, b").unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query statement");
    };

    assert_eq!(
        query.use_graph.as_ref().unwrap(),
        &GraphExpression::Name(graph_name(&["social"]))
    );
    let match_clause = first_match(query);
    assert!(match_clause.optional);
    assert_eq!(
        match_clause.patterns[0].chains[0].relationship.direction,
        Direction::Left
    );
}

#[test]
fn parses_focused_linear_query_use_graph_clauses() {
    let program = parse(
        "USE GRAPH social MATCH (a) \
         USE GRAPH archive MATCH (b) \
         RETURN a, b \
         UNION USE GRAPH reports MATCH (r) RETURN r",
    )
    .unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query statement");
    };

    assert_eq!(
        query.use_graph,
        Some(GraphExpression::Name(graph_name(&["social"])))
    );
    assert!(matches!(query.body.clauses[0], QueryClause::Match(_)));
    assert_eq!(
        query.body.clauses[1],
        QueryClause::UseGraph(GraphExpression::Name(graph_name(&["archive"])))
    );
    assert!(matches!(query.body.clauses[2], QueryClause::Match(_)));

    assert_eq!(query.set_operations.len(), 1);
    assert_eq!(query.set_operations[0].operator, QuerySetOperator::Union);
    assert_eq!(
        query.set_operations[0].body.clauses[0],
        QueryClause::UseGraph(GraphExpression::Name(graph_name(&["reports"])))
    );
    assert!(matches!(
        query.set_operations[0].body.clauses[1],
        QueryClause::Match(_)
    ));
}

#[test]
fn parses_standard_use_graph_clause_without_graph_keyword() {
    let program = parse(
        "USE social MATCH (n) RETURN n; \
         RETURN 1 AS value UNION USE reports MATCH (r) RETURN r",
    )
    .unwrap();
    assert_eq!(program.statements.len(), 2);

    let Statement::Query(first) = &program.statements[0] else {
        panic!("expected first query statement");
    };
    assert_eq!(
        first.use_graph,
        Some(GraphExpression::Name(graph_name(&["social"])))
    );
    assert!(matches!(first.body.clauses[0], QueryClause::Match(_)));

    let Statement::Query(second) = &program.statements[1] else {
        panic!("expected second query statement");
    };
    assert_eq!(second.set_operations.len(), 1);
    assert_eq!(second.set_operations[0].operator, QuerySetOperator::Union);
    assert_eq!(
        second.set_operations[0].body.clauses[0],
        QueryClause::UseGraph(GraphExpression::Name(graph_name(&["reports"])))
    );
    assert!(matches!(
        second.set_operations[0].body.clauses[1],
        QueryClause::Match(_)
    ));
}

#[test]
fn parses_full_undirected_relationship_pattern() {
    let program = parse("MATCH (a)~[r:KNOWS {since: 2020}]~(b) RETURN r, a, b").unwrap();

    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query statement");
    };
    let relationship = &first_match(query).patterns[0].chains[0].relationship;
    assert_eq!(relationship.direction, Direction::Undirected);
    assert_eq!(relationship.variable, Some(ident("r")));
    assert_eq!(relationship.labels, vec![ident("KNOWS")]);
    assert_eq!(
        relationship
            .properties
            .as_ref()
            .expect("expected relationship properties")
            .entries
            .len(),
        1
    );
}

#[test]
fn parses_standard_relationship_direction_variants() {
    let program = parse(
        "MATCH (a)-(b), \
               (a)-[any]- (b), \
               (a)<->(b), \
               (a)<-[either]->(b), \
               (a)<~(b), \
               (a)<~[left_or_undirected]~(b), \
               (a)~>(b), \
               (a)~[undirected_or_right]~>(b) \
         RETURN a, b",
    )
    .unwrap();

    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query statement");
    };
    let patterns = &first_match(query).patterns;
    assert_eq!(patterns[0].chains[0].relationship.direction, Direction::Any);
    assert_eq!(patterns[1].chains[0].relationship.direction, Direction::Any);
    assert_eq!(
        patterns[2].chains[0].relationship.direction,
        Direction::LeftOrRight
    );
    assert_eq!(
        patterns[3].chains[0].relationship.direction,
        Direction::LeftOrRight
    );
    assert_eq!(
        patterns[4].chains[0].relationship.direction,
        Direction::LeftOrUndirected
    );
    assert_eq!(
        patterns[5].chains[0].relationship.direction,
        Direction::LeftOrUndirected
    );
    assert_eq!(
        patterns[6].chains[0].relationship.direction,
        Direction::UndirectedOrRight
    );
    assert_eq!(
        patterns[7].chains[0].relationship.direction,
        Direction::UndirectedOrRight
    );
}

#[test]
fn parses_implicit_node_patterns_for_edge_only_paths() {
    let program = parse(
        "MATCH -[r]->, \
               (a)-[s]->, \
               <-[t]-, \
               -[u]->-[v]->, \
               (-[q]-> WHERE true){1,2} \
         RETURN r, s, t, u, v, q",
    )
    .unwrap();

    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query statement");
    };
    let patterns = &first_match(query).patterns;

    assert_eq!(patterns[0].start.variable, None);
    assert_eq!(patterns[0].chains.len(), 1);
    assert_eq!(
        patterns[0].chains[0].relationship.variable,
        Some(ident("r"))
    );
    assert_eq!(patterns[0].chains[0].node.variable, None);

    assert_eq!(patterns[1].start.variable, Some(ident("a")));
    assert_eq!(patterns[1].chains.len(), 1);
    assert_eq!(
        patterns[1].chains[0].relationship.variable,
        Some(ident("s"))
    );
    assert_eq!(patterns[1].chains[0].node.variable, None);

    assert_eq!(
        patterns[2].chains[0].relationship.direction,
        Direction::Left
    );
    assert_eq!(
        patterns[2].chains[0].relationship.variable,
        Some(ident("t"))
    );

    assert_eq!(patterns[3].start.variable, None);
    assert_eq!(patterns[3].chains.len(), 2);
    assert_eq!(
        patterns[3].chains[0].relationship.variable,
        Some(ident("u"))
    );
    assert_eq!(patterns[3].chains[0].node.variable, None);
    assert_eq!(
        patterns[3].chains[1].relationship.variable,
        Some(ident("v"))
    );
    assert_eq!(patterns[3].chains[1].node.variable, None);

    let parenthesized = patterns[4]
        .parenthesized
        .as_ref()
        .expect("expected parenthesized edge-only path");
    assert_eq!(
        parenthesized.quantifier,
        Some(PathPatternQuantifier::Range {
            min: Some(1),
            max: Some(2)
        })
    );
    assert!(matches!(
        parenthesized.where_clause,
        Some(Expr::Literal(Literal::Boolean(true)))
    ));
    assert_eq!(
        parenthesized.pattern.chains[0].relationship.variable,
        Some(ident("q"))
    );
}

#[test]
fn parses_simplified_path_pattern_expressions() {
    let program = parse(
        "MATCH (a)-/RIGHT/->{1,2}(b), \
               (a)<-/LEFT/-(b), \
               (a)~/UNDIRECTED/~(b), \
               (a)<~/LEFT_OR_UNDIRECTED/~(b), \
               (a)~/UNDIRECTED_OR_RIGHT/~>(b), \
               (a)<-/LEFT_OR_RIGHT/->(b), \
               (a)-/ANY & !BLOCKED/-(b), \
               (a)-/<OVERRIDE_LEFT/->(b), \
               (a)-/OVERRIDE_RIGHT>/-(b), \
               (a)-/~OVERRIDE_UNDIRECTED_OR_RIGHT>/-(b), \
               (a)-/-OVERRIDE_ANY/->(b), \
               (a)-/FIRST? SECOND*/->(b), \
               (a)-/UNION_LEFT|UNION_RIGHT/->(b), \
               (a)-/MULTI_LEFT|+|MULTI_RIGHT/->(b) \
         RETURN a, b",
    )
    .unwrap();

    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query statement");
    };
    let patterns = &first_match(query).patterns;
    assert_eq!(
        patterns[0].chains[0].relationship.direction,
        Direction::Right
    );
    assert_eq!(
        patterns[0].chains[0].relationship.quantifier,
        Some(PathPatternQuantifier::Range {
            min: Some(1),
            max: Some(2)
        })
    );
    assert_eq!(
        patterns[1].chains[0].relationship.direction,
        Direction::Left
    );
    assert_eq!(
        patterns[2].chains[0].relationship.direction,
        Direction::Undirected
    );
    assert_eq!(
        patterns[3].chains[0].relationship.direction,
        Direction::LeftOrUndirected
    );
    assert_eq!(
        patterns[4].chains[0].relationship.direction,
        Direction::UndirectedOrRight
    );
    assert_eq!(
        patterns[5].chains[0].relationship.direction,
        Direction::LeftOrRight
    );
    assert_eq!(patterns[6].chains[0].relationship.direction, Direction::Any);
    assert!(matches!(
        patterns[6].chains[0].relationship.label_expression,
        Some(LabelExpression::And(_, _))
    ));
    assert_eq!(
        patterns[7].chains[0].relationship.direction,
        Direction::Left
    );
    assert_eq!(
        patterns[8].chains[0].relationship.direction,
        Direction::Right
    );
    assert_eq!(
        patterns[9].chains[0].relationship.direction,
        Direction::UndirectedOrRight
    );
    assert_eq!(
        patterns[10].chains[0].relationship.direction,
        Direction::Any
    );
    assert_eq!(patterns[11].factors.len(), 4);
    assert_eq!(patterns[11].chains.len(), 2);
    assert_eq!(
        patterns[11].chains[0].relationship.labels,
        vec![ident("FIRST")]
    );
    assert_eq!(
        patterns[11].chains[0].relationship.quantifier,
        Some(PathPatternQuantifier::Optional)
    );
    assert_eq!(
        patterns[11].chains[1].relationship.labels,
        vec![ident("SECOND")]
    );
    assert_eq!(
        patterns[11].chains[1].relationship.quantifier,
        Some(PathPatternQuantifier::ZeroOrMore)
    );
    assert_eq!(patterns[11].chains[1].node.variable, Some(ident("b")));

    let PathPatternFactor::Alternation {
        alternation,
        alternatives,
    } = &patterns[12].factors[1]
    else {
        panic!("expected simplified path union factor");
    };
    assert_eq!(*alternation, PathPatternAlternation::Union);
    assert_eq!(alternatives.len(), 2);
    let PathPatternFactor::Relationship(left) = &alternatives[0][0] else {
        panic!("expected relationship alternative");
    };
    assert_eq!(left.labels, vec![ident("UNION_LEFT")]);
    let PathPatternFactor::Relationship(right) = &alternatives[1][0] else {
        panic!("expected relationship alternative");
    };
    assert_eq!(right.labels, vec![ident("UNION_RIGHT")]);

    let PathPatternFactor::Alternation {
        alternation,
        alternatives,
    } = &patterns[13].factors[1]
    else {
        panic!("expected simplified multiset factor");
    };
    assert_eq!(*alternation, PathPatternAlternation::Multiset);
    assert_eq!(alternatives.len(), 2);
}

#[test]
fn parses_optional_match_statement_block() {
    let program = parse(
        "OPTIONAL { MATCH (a)-[:KNOWS]->(b) OPTIONAL MATCH (b)-[:LIKES]->(c) } RETURN a, b, c; \
         OPTIONAL (MATCH (a)-[:FOLLOWS]->(b)) RETURN a, b",
    )
    .unwrap();

    let Statement::Query(first) = &program.statements[0] else {
        panic!("expected query statement");
    };
    let QueryClause::OptionalMatchBlock(matches) = &first.body.clauses[0] else {
        panic!("expected optional match block");
    };
    assert_eq!(matches.len(), 2);
    assert!(!matches[0].optional);
    assert!(matches[1].optional);
    assert_eq!(
        matches[0].patterns[0].chains[0].relationship.labels,
        vec![ident("KNOWS")]
    );
    assert_eq!(
        matches[1].patterns[0].chains[0].relationship.labels,
        vec![ident("LIKES")]
    );

    let Statement::Query(second) = &program.statements[1] else {
        panic!("expected query statement");
    };
    let QueryClause::OptionalMatchBlock(matches) = &second.body.clauses[0] else {
        panic!("expected optional match block");
    };
    assert_eq!(matches.len(), 1);
    assert_eq!(
        matches[0].patterns[0].chains[0].relationship.labels,
        vec![ident("FOLLOWS")]
    );
}

#[test]
fn parses_current_and_home_graph_expressions() {
    let program = parse(
        "USE PROPERTY GRAPH CURRENT_PROPERTY_GRAPH MATCH (n) RETURN n; \
         SELECT n FROM CURRENT_GRAPH MATCH (n) WHERE true, HOME_GRAPH MATCH (m) WHERE true; \
         SESSION SET PROPERTY GRAPH HOME_PROPERTY_GRAPH; \
         USE GRAPH VARIABLE active MATCH (n) RETURN n; \
         SELECT n FROM (active) MATCH (n); \
         SET n.property = 1",
    )
    .unwrap();

    let Statement::Query(use_query) = &program.statements[0] else {
        panic!("expected query");
    };
    assert_eq!(
        use_query.use_graph,
        Some(GraphExpression::CurrentPropertyGraph)
    );

    let Statement::Query(select_query) = &program.statements[1] else {
        panic!("expected query");
    };
    assert_eq!(select_query.body.select_from.len(), 2);
    assert_eq!(
        select_query.body.select_from[0].graph,
        GraphExpression::CurrentGraph
    );
    assert_eq!(
        select_query.body.select_from[1].graph,
        GraphExpression::HomeGraph
    );

    let Statement::SessionSet(set_graph) = &program.statements[2] else {
        panic!("expected session set");
    };
    assert_eq!(
        set_graph.target,
        SessionSetTarget::Graph(GraphExpression::HomePropertyGraph)
    );

    let Statement::Query(variable_query) = &program.statements[3] else {
        panic!("expected query");
    };
    assert_eq!(
        variable_query.use_graph,
        Some(GraphExpression::Variable(Box::new(Expr::Identifier(
            ident("active")
        ))))
    );

    let Statement::Query(parenthesized_query) = &program.statements[4] else {
        panic!("expected query");
    };
    assert_eq!(
        parenthesized_query.body.select_from[0].graph,
        GraphExpression::Variable(Box::new(Expr::Identifier(ident("active"))))
    );

    let Statement::Set(set_statement) = &program.statements[5] else {
        panic!("expected property set");
    };
    assert_eq!(set_statement.items.len(), 1);
}

#[test]
fn parses_standard_set_and_remove_items() {
    let program = parse(
        "SET n.age = 42, m = {name: 'Alice', active: true}, n:Person, n IS Active; \
         REMOVE n.old, n:Temporary, n IS Active",
    )
    .unwrap();

    let Statement::Set(set_statement) = &program.statements[0] else {
        panic!("expected set statement");
    };
    assert_eq!(set_statement.items.len(), 4);
    let SetItem::Property { target, value } = &set_statement.items[0] else {
        panic!("expected set property item");
    };
    assert_eq!(
        target,
        &Expr::Property {
            base: Box::new(Expr::Identifier(ident("n"))),
            key: ident("age")
        }
    );
    assert_eq!(value, &Expr::Literal(Literal::Integer(42)));
    let SetItem::AllProperties {
        variable,
        properties,
    } = &set_statement.items[1]
    else {
        panic!("expected set all properties item");
    };
    assert_eq!(variable, &ident("m"));
    assert_eq!(properties.entries.len(), 2);
    assert!(matches!(
        &set_statement.items[2],
        SetItem::Label { variable, label }
            if variable == &ident("n") && label == &ident("Person")
    ));
    assert!(matches!(
        &set_statement.items[3],
        SetItem::Label { variable, label }
            if variable == &ident("n") && label == &ident("Active")
    ));

    let Statement::Remove(remove_statement) = &program.statements[1] else {
        panic!("expected remove statement");
    };
    assert_eq!(remove_statement.items.len(), 3);
    assert!(matches!(
        &remove_statement.items[0],
        RemoveItem::Property(_)
    ));
    assert!(matches!(
        &remove_statement.items[1],
        RemoveItem::Label { variable, label }
            if variable == &ident("n") && label == &ident("Temporary")
    ));
    assert!(matches!(
        &remove_statement.items[2],
        RemoveItem::Label { variable, label }
            if variable == &ident("n") && label == &ident("Active")
    ));
}

#[test]
fn parses_next_statement_chain() {
    let program =
        parse("RETURN 1 AS n NEXT YIELD n AS m RETURN m NEXT YIELD m AS kept FINISH").unwrap();
    assert_eq!(program.statements.len(), 3);

    let Statement::Query(first) = &program.statements[0] else {
        panic!("expected query statement");
    };
    assert_eq!(first.body.result_clause.items[0].alias, Some(ident("n")));

    let Statement::Next(second) = &program.statements[1] else {
        panic!("expected next statement");
    };
    let yield_clause = second.yield_clause.as_ref().unwrap();
    assert!(matches!(
        &yield_clause.items[0],
        YieldItem::Item {
            name,
            alias: Some(alias)
        } if name == &ident("n") && alias == &ident("m")
    ));
    let Statement::Query(next_query) = second.statement.as_ref() else {
        panic!("expected query after next");
    };
    assert_eq!(
        next_query.body.result_clause.items[0].expr,
        Expr::Identifier(ident("m"))
    );

    let Statement::Next(third) = &program.statements[2] else {
        panic!("expected next statement");
    };
    let yield_clause = third.yield_clause.as_ref().unwrap();
    assert!(matches!(
        &yield_clause.items[0],
        YieldItem::Item {
            name,
            alias: Some(alias)
        } if name == &ident("m") && alias == &ident("kept")
    ));
    let Statement::Query(finish_query) = third.statement.as_ref() else {
        panic!("expected query after next");
    };
    assert_eq!(finish_query.body.result_clause.kind, ResultKind::Finish);
}

#[test]
fn parses_binding_variable_definition_block() {
    let program = parse(
        "VALUE limit :: TYPED INTEGER = 10 \
         PROPERTY GRAPH work :: PROPERTY GRAPH { NODE Person {name STRING} } = HOME_PROPERTY_GRAPH \
         BINDING TABLE rows :: BINDING TABLE {node ANY NODE, score INTEGER} = app.rows \
         RETURN limit, GRAPH work AS g, BINDING TABLE rows AS rows",
    )
    .unwrap();

    assert_eq!(program.definitions.len(), 3);
    let BindingVariableDefinition::Value(value) = &program.definitions[0] else {
        panic!("expected value variable definition");
    };
    assert_eq!(value.name, ident("limit"));
    assert!(value.typed);
    assert_eq!(
        value.value_type,
        Some(ValueType::ExactNumeric {
            kind: ExactNumericTypeKind::Integer,
            signed: Some(true),
            precision: None,
            scale: None,
        })
    );
    assert_eq!(value.initializer, Expr::Literal(Literal::Integer(10)));

    let BindingVariableDefinition::Graph(graph) = &program.definitions[1] else {
        panic!("expected graph variable definition");
    };
    assert!(graph.property_graph);
    assert_eq!(graph.name, ident("work"));
    assert!(graph.typed);
    assert_eq!(graph.initializer, GraphExpression::HomePropertyGraph);
    assert!(matches!(
        graph.value_type,
        Some(ValueType::GraphReference {
            property_graph: true,
            body: Some(_)
        })
    ));

    let BindingVariableDefinition::BindingTable(table) = &program.definitions[2] else {
        panic!("expected binding table variable definition");
    };
    assert!(table.binding);
    assert_eq!(table.name, ident("rows"));
    assert!(table.typed);
    assert_eq!(
        table.initializer,
        BindingTableExpression::Name(binding_table_name(&["app", "rows"]))
    );
    assert!(matches!(
        table.value_type,
        Some(ValueType::BindingTable {
            binding: true,
            ref fields
        }) if fields.len() == 2
    ));

    assert_eq!(program.statements.len(), 1);
}

#[test]
fn parses_nested_query_binding_table_initializer() {
    let program = parse(
        "TABLE matches = { MATCH (n) RETURN n } \
         RETURN BINDING TABLE matches AS matches",
    )
    .unwrap();

    assert_eq!(program.definitions.len(), 1);
    let BindingVariableDefinition::BindingTable(table) = &program.definitions[0] else {
        panic!("expected binding table variable definition");
    };
    assert!(!table.binding);
    let BindingTableExpression::NestedQuery(nested) = &table.initializer else {
        panic!("expected nested binding table query");
    };
    assert_eq!(
        nested.body.result_clause.items[0].expr,
        Expr::Identifier(ident("n"))
    );
}

#[test]
fn parses_parameterized_graph_expressions() {
    let program = parse(
        "USE GRAPH $active MATCH (n) RETURN n; \
         SELECT n FROM $report MATCH (n); \
         SESSION SET GRAPH $next; \
         USE GRAPH $ctx.active MATCH (n) RETURN n",
    )
    .unwrap();

    let Statement::Query(use_query) = &program.statements[0] else {
        panic!("expected query");
    };
    assert_eq!(
        use_query.use_graph,
        Some(GraphExpression::Parameter("active".to_owned()))
    );

    let Statement::Query(select_query) = &program.statements[1] else {
        panic!("expected query");
    };
    assert_eq!(
        select_query.body.select_from[0].graph,
        GraphExpression::Parameter("report".to_owned())
    );

    let Statement::SessionSet(set_graph) = &program.statements[2] else {
        panic!("expected session set");
    };
    assert_eq!(
        set_graph.target,
        SessionSetTarget::Graph(GraphExpression::Parameter("next".to_owned()))
    );

    let Statement::Query(property_query) = &program.statements[3] else {
        panic!("expected query");
    };
    assert_eq!(
        property_query.use_graph,
        Some(GraphExpression::Variable(Box::new(Expr::Property {
            base: Box::new(Expr::Parameter("ctx".to_owned())),
            key: ident("active")
        })))
    );
}

#[test]
fn parses_object_expression_primary_graph_and_binding_table_expressions() {
    let program = parse(
        "USE GRAPH CASE WHEN true THEN active ELSE fallback END MATCH (n) RETURN n; \
         SELECT n FROM [active] MATCH (n); \
         USE GRAPH LET g = active IN g END MATCH (m) RETURN m; \
         RETURN GRAPH CASE WHEN true THEN active ELSE fallback END AS g, \
                TABLE CASE WHEN true THEN rows ELSE fallback END AS t, \
                GRAPH CAST($g AS ANY GRAPH) AS cast_g, \
                TABLE LET t = rows IN t END AS let_t, \
                TABLE CAST($t AS BINDING TABLE {node ANY NODE}) AS cast_t",
    )
    .unwrap();

    let Statement::Query(use_query) = &program.statements[0] else {
        panic!("expected query");
    };
    let Some(GraphExpression::Variable(use_graph)) = &use_query.use_graph else {
        panic!("expected object expression primary graph expression");
    };
    assert!(matches!(use_graph.as_ref(), Expr::Case { .. }));

    let Statement::Query(select_query) = &program.statements[1] else {
        panic!("expected query");
    };
    let GraphExpression::Variable(select_graph) = &select_query.body.select_from[0].graph else {
        panic!("expected object expression primary select graph");
    };
    assert!(matches!(select_graph.as_ref(), Expr::List(_)));

    let Statement::Query(let_query) = &program.statements[2] else {
        panic!("expected query");
    };
    let Some(GraphExpression::Variable(let_graph)) = &let_query.use_graph else {
        panic!("expected let graph expression");
    };
    assert!(matches!(let_graph.as_ref(), Expr::Let { .. }));

    let Statement::Query(return_query) = &program.statements[3] else {
        panic!("expected query");
    };
    let items = &return_query.body.result_clause.items;
    let Expr::GraphReference {
        property_graph: false,
        graph: GraphExpression::Variable(return_graph),
    } = &items[0].expr
    else {
        panic!("expected object expression primary graph reference value");
    };
    assert!(matches!(return_graph.as_ref(), Expr::Case { .. }));

    let Expr::BindingTableReference {
        binding: false,
        table: BindingTableExpression::Variable(return_table),
    } = &items[1].expr
    else {
        panic!("expected object expression primary binding table reference value");
    };
    assert!(matches!(return_table.as_ref(), Expr::Case { .. }));

    let Expr::GraphReference {
        property_graph: false,
        graph: GraphExpression::Variable(cast_graph),
    } = &items[2].expr
    else {
        panic!("expected cast graph reference value");
    };
    assert!(matches!(cast_graph.as_ref(), Expr::Cast { .. }));

    let Expr::BindingTableReference {
        binding: false,
        table: BindingTableExpression::Variable(let_table),
    } = &items[3].expr
    else {
        panic!("expected let binding table reference value");
    };
    assert!(matches!(let_table.as_ref(), Expr::Let { .. }));

    let Expr::BindingTableReference {
        binding: false,
        table: BindingTableExpression::Variable(cast_table),
    } = &items[4].expr
    else {
        panic!("expected cast binding table reference value");
    };
    assert!(matches!(cast_table.as_ref(), Expr::Cast { .. }));
}

#[test]
fn parses_delimited_and_extended_parameter_names() {
    let program = parse(
        r#"USE GRAPH $"active graph" MATCH (n) RETURN $@"raw\nname" AS raw, $123 AS ordinal"#,
    )
    .unwrap();

    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };
    assert_eq!(
        query.use_graph,
        Some(GraphExpression::Parameter("active graph".to_owned()))
    );

    let items = &query.body.result_clause.items;
    assert_eq!(items[0].expr, Expr::Parameter(r"raw\nname".to_owned()));
    assert_eq!(items[1].expr, Expr::Parameter("123".to_owned()));
}

#[test]
fn parses_graph_reference_value_expressions() {
    let program = parse(
        "RETURN GRAPH social AS g, \
         PROPERTY GRAPH CURRENT_PROPERTY_GRAPH AS pg, \
         GRAPH $ctx.active AS active_graph, \
         graph AS identifier",
    )
    .unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };
    let items = &query.body.result_clause.items;

    assert_eq!(
        items[0].expr,
        Expr::GraphReference {
            property_graph: false,
            graph: GraphExpression::Name(graph_name(&["social"]))
        }
    );
    assert_eq!(
        items[1].expr,
        Expr::GraphReference {
            property_graph: true,
            graph: GraphExpression::CurrentPropertyGraph
        }
    );
    assert_eq!(
        items[2].expr,
        Expr::GraphReference {
            property_graph: false,
            graph: GraphExpression::Variable(Box::new(Expr::Property {
                base: Box::new(Expr::Parameter("ctx".to_owned())),
                key: ident("active")
            }))
        }
    );
    assert_eq!(items[3].expr, Expr::Identifier(ident("graph")));
}

#[test]
fn parses_binding_table_reference_value_expressions() {
    let program = parse(
        "RETURN TABLE $rows AS t, \
         BINDING TABLE app.rows AS bt, \
         TABLE VARIABLE rows AS current_rows, \
         TABLE (rows) AS parenthesized_rows, \
         TABLE { MATCH (n) RETURN n } AS nested_rows, \
         TABLE $ctx.rows AS contextual_rows, \
         table AS identifier, \
         binding AS binding_identifier",
    )
    .unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };
    let items = &query.body.result_clause.items;

    assert_eq!(
        items[0].expr,
        Expr::BindingTableReference {
            binding: false,
            table: BindingTableExpression::Parameter("rows".to_owned())
        }
    );
    assert_eq!(
        items[1].expr,
        Expr::BindingTableReference {
            binding: true,
            table: BindingTableExpression::Name(binding_table_name(&["app", "rows"]))
        }
    );
    assert_eq!(
        items[2].expr,
        Expr::BindingTableReference {
            binding: false,
            table: BindingTableExpression::Variable(Box::new(Expr::Identifier(ident("rows"))))
        }
    );
    assert_eq!(
        items[3].expr,
        Expr::BindingTableReference {
            binding: false,
            table: BindingTableExpression::Variable(Box::new(Expr::Identifier(ident("rows"))))
        }
    );
    let Expr::BindingTableReference {
        binding: false,
        table: BindingTableExpression::NestedQuery(nested),
    } = &items[4].expr
    else {
        panic!("expected nested binding table reference");
    };
    assert_eq!(
        nested.body.result_clause.items[0].expr,
        Expr::Identifier(ident("n"))
    );
    assert_eq!(
        items[5].expr,
        Expr::BindingTableReference {
            binding: false,
            table: BindingTableExpression::Variable(Box::new(Expr::Property {
                base: Box::new(Expr::Parameter("ctx".to_owned())),
                key: ident("rows")
            }))
        }
    );
    assert_eq!(items[6].expr, Expr::Identifier(ident("table")));
    assert_eq!(items[7].expr, Expr::Identifier(ident("binding")));
}

#[test]
fn parses_linear_data_modifying_query_clauses() {
    let program = parse(
        "MATCH (n) \
         INSERT (:Seen) \
         SET n.flag = true \
         REMOVE n.old \
         DELETE n \
         RETURN n",
    )
    .unwrap();

    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };
    assert!(matches!(query.body.clauses[0], QueryClause::Match(_)));
    assert!(matches!(query.body.clauses[1], QueryClause::Insert(_)));
    assert!(matches!(query.body.clauses[2], QueryClause::Set(_)));
    assert!(matches!(query.body.clauses[3], QueryClause::Remove(_)));
    assert!(matches!(query.body.clauses[4], QueryClause::Delete(_)));
    assert_eq!(
        query.body.result_clause.items[0].expr,
        Expr::Identifier(ident("n"))
    );

    let program = parse("INSERT (:Seen) SET n.flag = true RETURN n").unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected linear data-modifying query");
    };
    assert_eq!(query.body.clauses.len(), 2);
    assert!(matches!(query.body.clauses[0], QueryClause::Insert(_)));
    assert!(matches!(query.body.clauses[1], QueryClause::Set(_)));
    assert_eq!(
        query.body.result_clause.items[0].expr,
        Expr::Identifier(ident("n"))
    );
}

#[test]
fn parses_implicit_finish_for_linear_data_modifying_queries() {
    let program = parse(
        "MATCH (n) SET n.flag = true; \
         USE GRAPH social INSERT (:Seen)",
    )
    .unwrap();

    let Statement::Query(update_query) = &program.statements[0] else {
        panic!("expected query");
    };
    assert!(matches!(update_query.body.clauses[1], QueryClause::Set(_)));
    assert_eq!(update_query.body.result_clause.kind, ResultKind::Finish);

    let Statement::Query(insert_query) = &program.statements[1] else {
        panic!("expected query");
    };
    assert_eq!(
        insert_query.use_graph,
        Some(GraphExpression::Name(graph_name(&["social"])))
    );
    assert!(matches!(
        insert_query.body.clauses[0],
        QueryClause::Insert(_)
    ));
    assert_eq!(insert_query.body.result_clause.kind, ResultKind::Finish);
}

#[test]
fn parses_catalog_and_data_modifying_statements() {
    let program = parse(
        "CREATE GRAPH IF NOT EXISTS demo.graph ANY GRAPH; \
         INSERT (:Person {name: $name})-[:KNOWS]->(:Person {name: 'Bob'}); \
         SET n.age = 42, n.active = true; \
         REMOVE n:Temporary, n.old; \
         DETACH DELETE n; \
         CREATE SCHEMA IF NOT EXISTS demo; \
         DROP SCHEMA IF EXISTS demo; \
         START TRANSACTION READ WRITE; \
         COMMIT; \
         ROLLBACK; \
         DROP GRAPH IF EXISTS demo.graph;",
    )
    .unwrap();

    assert_eq!(program.statements.len(), 11);
    assert!(matches!(program.statements[0], Statement::CreateGraph(_)));
    assert!(matches!(program.statements[1], Statement::Insert(_)));
    assert!(matches!(program.statements[2], Statement::Set(_)));
    let Statement::Remove(remove) = &program.statements[3] else {
        panic!("expected remove statement");
    };
    assert!(matches!(remove.items[0], RemoveItem::Label { .. }));
    assert!(matches!(remove.items[1], RemoveItem::Property(_)));
    assert!(matches!(program.statements[4], Statement::Delete(_)));
    assert!(matches!(program.statements[5], Statement::CreateSchema(_)));
    assert!(matches!(program.statements[6], Statement::DropSchema(_)));
    let Statement::StartTransaction(start) = &program.statements[7] else {
        panic!("expected start transaction");
    };
    assert_eq!(start.access_mode, Some(TransactionAccessMode::ReadWrite));
    assert_eq!(start.isolation_level, None);
    assert!(matches!(program.statements[8], Statement::Commit(_)));
    assert!(matches!(program.statements[9], Statement::Rollback(_)));
    assert!(matches!(program.statements[10], Statement::DropGraph(_)));
}

#[test]
fn parses_linear_catalog_modifying_statement() {
    let program = parse(
        "CREATE GRAPH demo.temp ANY GRAPH \
         DROP GRAPH demo.temp \
         CREATE GRAPH demo.copy ANY GRAPH AS COPY OF demo.source",
    )
    .unwrap();

    assert_eq!(program.statements.len(), 1);
    let Statement::LinearCatalog(statements) = &program.statements[0] else {
        panic!("expected linear catalog statement");
    };
    assert_eq!(statements.len(), 3);
    assert!(matches!(statements[0], Statement::CreateGraph(_)));
    assert!(matches!(statements[1], Statement::DropGraph(_)));
    let Statement::CreateGraph(copied) = &statements[2] else {
        panic!("expected copied graph");
    };
    assert!(matches!(
        copied.graph_type,
        Some(CreateGraphType::Any {
            property_graph: false,
            ..
        })
    ));
    assert_eq!(
        copied.source,
        Some(GraphExpression::Name(graph_name(&["demo", "source"])))
    );
}

#[test]
fn parses_call_prefixed_linear_catalog_modifying_statement() {
    let program = parse("CALL db.refresh() CREATE GRAPH demo.refreshed ANY GRAPH").unwrap();

    let Statement::LinearCatalog(statements) = &program.statements[0] else {
        panic!("expected call-prefixed linear catalog statement");
    };
    assert_eq!(statements.len(), 2);
    let Statement::Call(call) = &statements[0] else {
        panic!("expected catalog procedure call");
    };
    assert!(matches!(call.call.call, ProcedureCall::Named { .. }));
    assert!(matches!(statements[1], Statement::CreateGraph(_)));
}

#[test]
fn parses_insert_undirected_edge_pattern() {
    let program = parse(
        "INSERT (a Person & person & Employee)~[r KNOWS & knows & RELATED {since: 2024}]~(b Person)",
    )
    .unwrap();

    let Statement::Insert(insert) = &program.statements[0] else {
        panic!("expected insert statement");
    };
    assert_eq!(insert.patterns.len(), 1);
    let pattern = &insert.patterns[0];
    assert_eq!(pattern.start.variable, Some(ident("a")));
    assert_eq!(
        pattern.start.labels,
        vec![ident("Person"), ident("Employee")]
    );
    assert_eq!(pattern.chains.len(), 1);
    let relationship = &pattern.chains[0].relationship;
    assert_eq!(relationship.direction, Direction::Undirected);
    assert_eq!(relationship.variable, Some(ident("r")));
    assert_eq!(relationship.labels, vec![ident("KNOWS"), ident("RELATED")]);
    assert_eq!(
        relationship
            .properties
            .as_ref()
            .expect("expected edge properties")
            .entries
            .len(),
        1
    );
    assert_eq!(pattern.chains[0].node.variable, Some(ident("b")));
    assert_eq!(pattern.chains[0].node.labels, vec![ident("Person")]);
}

#[test]
fn parses_insert_element_variables_with_properties() {
    let program = parse("INSERT (n {name: 'Alice'})-[r {since: 2024}]->(m)").unwrap();

    let Statement::Insert(insert) = &program.statements[0] else {
        panic!("expected insert statement");
    };
    let pattern = &insert.patterns[0];
    assert_eq!(pattern.start.variable, Some(ident("n")));
    assert!(pattern.start.labels.is_empty());
    assert_eq!(
        pattern
            .start
            .properties
            .as_ref()
            .expect("expected node properties")
            .entries
            .len(),
        1
    );

    let relationship = &pattern.chains[0].relationship;
    assert_eq!(relationship.variable, Some(ident("r")));
    assert!(relationship.labels.is_empty());
    assert_eq!(
        relationship
            .properties
            .as_ref()
            .expect("expected relationship properties")
            .entries
            .len(),
        1
    );
    assert_eq!(pattern.chains[0].node.variable, Some(ident("m")));
}

#[test]
fn parses_repeated_insert_node_variable_without_additional_filler() {
    let program = parse("INSERT (n:Person)-[:KNOWS]->(n)").unwrap();

    let Statement::Insert(insert) = &program.statements[0] else {
        panic!("expected insert statement");
    };
    let pattern = &insert.patterns[0];
    assert_eq!(pattern.start.variable, Some(ident("n")));
    assert_eq!(pattern.start.labels, vec![ident("Person")]);
    assert_eq!(pattern.chains[0].node.variable, Some(ident("n")));
    assert!(pattern.chains[0].node.labels.is_empty());
    assert!(pattern.chains[0].node.properties.is_none());
}

#[test]
fn parses_delete_detach_modes() {
    let program = parse("DELETE e; DETACH DELETE n; NODETACH DELETE m").unwrap();

    let Statement::Delete(default_delete) = &program.statements[0] else {
        panic!("expected delete statement");
    };
    assert!(!default_delete.detach);
    assert_eq!(default_delete.items, vec![Expr::Identifier(ident("e"))]);

    let Statement::Delete(detach_delete) = &program.statements[1] else {
        panic!("expected detach delete statement");
    };
    assert!(detach_delete.detach);
    assert_eq!(detach_delete.items, vec![Expr::Identifier(ident("n"))]);

    let Statement::Delete(nodetach_delete) = &program.statements[2] else {
        panic!("expected nodetach delete statement");
    };
    assert!(!nodetach_delete.detach);
    assert_eq!(nodetach_delete.items, vec![Expr::Identifier(ident("m"))]);
}

#[test]
fn parses_graph_type_catalog_statements() {
    let program = parse(
        "CREATE GRAPH TYPE IF NOT EXISTS social AS { \
           NODE Person {name STRING, age INTEGER, tags LIST<STRING>, meta RECORD {score INTEGER}}, \
           DIRECTED EDGE Knows FROM Person TO Person {since INTEGER, weights ARRAY<INTEGER>} \
         }; \
         DROP GRAPH TYPE IF EXISTS social;",
    )
    .unwrap();

    assert_eq!(program.statements.len(), 2);
    let Statement::CreateGraphType(create) = &program.statements[0] else {
        panic!("expected create graph type");
    };
    assert_eq!(create.name, graph_type_name(&["social"]));
    let CreateGraphTypeSource::Nested(body) = &create.source else {
        panic!("expected nested graph type source");
    };
    assert_eq!(body.elements.len(), 2);
    let GraphTypeElement::Node(node) = &body.elements[0] else {
        panic!("expected node type");
    };
    assert_eq!(node.name, ident("Person"));
    assert_eq!(node.properties.len(), 4);
    assert!(matches!(node.properties[2].value_type, ValueType::List(_)));
    assert!(matches!(
        node.properties[3].value_type,
        ValueType::Record(_)
    ));
    let GraphTypeElement::Edge(edge) = &body.elements[1] else {
        panic!("expected edge type");
    };
    assert_eq!(edge.source, Some(ident("Person")));
    assert_eq!(edge.destination, Some(ident("Person")));
    assert!(matches!(edge.properties[1].value_type, ValueType::Array(_)));
    assert!(matches!(program.statements[1], Statement::DropGraphType(_)));
}

#[test]
fn parses_field_type_typed_markers() {
    let program = parse(
        "CREATE GRAPH TYPE typed_fields AS { \
           NODE Person {name TYPED STRING, age :: INTEGER, meta RECORD {score TYPED INTEGER, tags :: LIST<STRING>}} \
         }",
    )
    .unwrap();

    let Statement::CreateGraphType(create) = &program.statements[0] else {
        panic!("expected create graph type");
    };
    let CreateGraphTypeSource::Nested(body) = &create.source else {
        panic!("expected nested graph type source");
    };
    let GraphTypeElement::Node(node) = &body.elements[0] else {
        panic!("expected node type");
    };

    assert!(matches!(
        node.properties[0].value_type,
        ValueType::CharacterString {
            kind: CharacterStringTypeKind::String,
            max_length: None,
        }
    ));
    assert!(matches!(
        node.properties[1].value_type,
        ValueType::ExactNumeric {
            kind: ExactNumericTypeKind::Integer,
            signed: Some(true),
            precision: None,
            scale: None,
        }
    ));
    let ValueType::Record(fields) = &node.properties[2].value_type else {
        panic!("expected record value type");
    };
    assert_eq!(fields[0].name, ident("score"));
    assert!(matches!(
        fields[0].value_type,
        ValueType::ExactNumeric {
            kind: ExactNumericTypeKind::Integer,
            signed: Some(true),
            precision: None,
            scale: None,
        }
    ));
    assert_eq!(fields[1].name, ident("tags"));
    assert!(matches!(fields[1].value_type, ValueType::List(_)));
}

#[test]
fn parses_graph_type_label_set_definitions() {
    let program = parse(
        "CREATE GRAPH TYPE labels AS { \
           NODE Person LABELS Person & person & Employee {name STRING}, \
           DIRECTED EDGE Knows LABEL KNOWS FROM Person TO Person {since INTEGER}, \
           NODE Membership IS User & Group & user, \
           DIRECTED EDGE Likes IS LIKES FROM Person TO Person, \
           NODE LABEL {value STRING} \
         }",
    )
    .unwrap();

    let Statement::CreateGraphType(create) = &program.statements[0] else {
        panic!("expected create graph type");
    };
    let CreateGraphTypeSource::Nested(body) = &create.source else {
        panic!("expected nested graph type source");
    };
    let GraphTypeElement::Node(person) = &body.elements[0] else {
        panic!("expected node type");
    };
    assert_eq!(person.labels, vec![ident("Person"), ident("Employee")]);

    let GraphTypeElement::Edge(knows) = &body.elements[1] else {
        panic!("expected edge type");
    };
    assert_eq!(knows.labels, vec![ident("KNOWS")]);

    let GraphTypeElement::Node(label_named_node) = &body.elements[2] else {
        panic!("expected node type");
    };
    assert_eq!(label_named_node.labels, vec![ident("User"), ident("Group")]);

    let GraphTypeElement::Edge(likes) = &body.elements[3] else {
        panic!("expected edge type");
    };
    assert_eq!(likes.labels, vec![ident("LIKES")]);

    let GraphTypeElement::Node(label_named_node) = &body.elements[4] else {
        panic!("expected node type");
    };
    assert_eq!(label_named_node.name, ident("LABEL"));
    assert!(label_named_node.labels.is_empty());
}

#[test]
fn parses_graph_type_filler_only_phrases() {
    let program = parse(
        "CREATE GRAPH TYPE filler_only AS { \
           NODE Person, \
           NODE :Person {name STRING}, \
           (:Person)-[:KNOWS {since INTEGER}]->(:Person), \
           NODE Holder {n NODE :Person, e (:Person)-[:KNOWS]->(:Person)} \
         }",
    )
    .unwrap();

    let Statement::CreateGraphType(create) = &program.statements[0] else {
        panic!("expected create graph type");
    };
    let CreateGraphTypeSource::Nested(body) = &create.source else {
        panic!("expected nested graph type source");
    };

    let GraphTypeElement::Node(person) = &body.elements[1] else {
        panic!("expected filler-only node type");
    };
    assert_eq!(person.name, ident(""));
    assert_eq!(person.labels, vec![ident("Person")]);
    assert_eq!(person.properties[0].name, ident("name"));

    let GraphTypeElement::Edge(knows) = &body.elements[2] else {
        panic!("expected filler-only edge type");
    };
    assert_eq!(knows.name, ident(""));
    assert_eq!(knows.labels, vec![ident("KNOWS")]);
    assert_eq!(knows.source, None);
    assert_eq!(knows.destination, None);
    assert_eq!(knows.properties[0].name, ident("since"));

    let GraphTypeElement::Node(holder) = &body.elements[3] else {
        panic!("expected holder node type");
    };
    let ValueType::NodeReference {
        definition: Some(node),
    } = &holder.properties[0].value_type
    else {
        panic!("expected closed node reference type");
    };
    assert_eq!(node.name, ident(""));
    assert_eq!(node.labels, vec![ident("Person")]);

    let ValueType::EdgeReference {
        definition: Some(edge),
    } = &holder.properties[1].value_type
    else {
        panic!("expected closed edge reference type");
    };
    assert_eq!(edge.name, ident(""));
    assert_eq!(edge.labels, vec![ident("KNOWS")]);
    assert_eq!(edge.source, None);
    assert_eq!(edge.destination, None);
}

#[test]
fn parses_parenthesized_node_type_patterns() {
    let program = parse(
        "CREATE GRAPH TYPE patterns AS { \
           (Person LABELS Person & Employee {name STRING}), \
           NODE Account {id STRING} \
         }",
    )
    .unwrap();

    let Statement::CreateGraphType(create) = &program.statements[0] else {
        panic!("expected create graph type");
    };
    let CreateGraphTypeSource::Nested(body) = &create.source else {
        panic!("expected nested graph type source");
    };
    let GraphTypeElement::Node(person) = &body.elements[0] else {
        panic!("expected node type");
    };
    assert_eq!(person.name, ident("Person"));
    assert_eq!(person.labels, vec![ident("Person"), ident("Employee")]);
    assert_eq!(person.properties[0].name, ident("name"));
}

#[test]
fn parses_anonymous_graph_type_patterns() {
    let program = parse(
        "CREATE GRAPH TYPE anonymous AS { \
           (:Person {name STRING}), \
           (:Person)-[:KNOWS {since INTEGER}]->(:Company) \
         }",
    )
    .unwrap();

    let Statement::CreateGraphType(create) = &program.statements[0] else {
        panic!("expected create graph type");
    };
    let CreateGraphTypeSource::Nested(body) = &create.source else {
        panic!("expected nested graph type source");
    };
    let GraphTypeElement::Node(person) = &body.elements[0] else {
        panic!("expected anonymous node type");
    };
    assert_eq!(person.name, ident(""));
    assert_eq!(person.labels, vec![ident("Person")]);
    assert_eq!(person.properties[0].name, ident("name"));

    let GraphTypeElement::Edge(knows) = &body.elements[1] else {
        panic!("expected anonymous edge type");
    };
    assert_eq!(knows.name, ident(""));
    assert_eq!(knows.labels, vec![ident("KNOWS")]);
    assert_eq!(knows.source, None);
    assert_eq!(knows.destination, None);
    assert_eq!(knows.properties[0].name, ident("since"));
}

#[test]
fn parses_abbreviated_graph_type_edge_patterns() {
    let program = parse(
        "CREATE GRAPH TYPE abbreviated AS { \
           NODE Person, \
           NODE Company, \
           (Person)->(Company), \
           (Company)<-(Person), \
           (Person)~(Person), \
           DIRECTED EDGE Visits CONNECTING (Person)->(Company) \
         }",
    )
    .unwrap();

    let Statement::CreateGraphType(create) = &program.statements[0] else {
        panic!("expected create graph type");
    };
    let CreateGraphTypeSource::Nested(body) = &create.source else {
        panic!("expected nested graph type source");
    };

    let GraphTypeElement::Edge(right) = &body.elements[2] else {
        panic!("expected right edge type");
    };
    assert_eq!(right.name, ident(""));
    assert_eq!(right.direction, Direction::Right);
    assert_eq!(right.source, Some(ident("Person")));
    assert_eq!(right.destination, Some(ident("Company")));

    let GraphTypeElement::Edge(left) = &body.elements[3] else {
        panic!("expected left edge type");
    };
    assert_eq!(left.direction, Direction::Right);
    assert_eq!(left.source, Some(ident("Person")));
    assert_eq!(left.destination, Some(ident("Company")));

    let GraphTypeElement::Edge(undirected) = &body.elements[4] else {
        panic!("expected undirected edge type");
    };
    assert_eq!(undirected.direction, Direction::Undirected);
    assert_eq!(undirected.source, Some(ident("Person")));
    assert_eq!(undirected.destination, Some(ident("Person")));

    let GraphTypeElement::Edge(connecting) = &body.elements[5] else {
        panic!("expected connecting edge type");
    };
    assert_eq!(connecting.name, ident("Visits"));
    assert_eq!(connecting.source, Some(ident("Person")));
    assert_eq!(connecting.destination, Some(ident("Company")));
}

#[test]
fn parses_full_edge_type_patterns() {
    let program = parse(
        "CREATE GRAPH TYPE edge_patterns AS { \
           NODE Person, \
           NODE Company, \
           (Person)-[Knows LABELS KNOWS & RELATED {since INTEGER}]->(Company), \
           (Company)<-[WorksAt LABEL WORKS_AT]-(Person) \
         }",
    )
    .unwrap();

    let Statement::CreateGraphType(create) = &program.statements[0] else {
        panic!("expected create graph type");
    };
    let CreateGraphTypeSource::Nested(body) = &create.source else {
        panic!("expected nested graph type source");
    };
    let GraphTypeElement::Edge(knows) = &body.elements[2] else {
        panic!("expected edge type");
    };
    assert_eq!(knows.name, ident("Knows"));
    assert_eq!(knows.direction, Direction::Right);
    assert_eq!(knows.source, Some(ident("Person")));
    assert_eq!(knows.destination, Some(ident("Company")));
    assert_eq!(knows.labels, vec![ident("KNOWS"), ident("RELATED")]);
    assert_eq!(knows.properties[0].name, ident("since"));

    let GraphTypeElement::Edge(works_at) = &body.elements[3] else {
        panic!("expected edge type");
    };
    assert_eq!(works_at.name, ident("WorksAt"));
    assert_eq!(works_at.direction, Direction::Right);
    assert_eq!(works_at.source, Some(ident("Person")));
    assert_eq!(works_at.destination, Some(ident("Company")));
    assert_eq!(works_at.labels, vec![ident("WORKS_AT")]);
}

#[test]
fn parses_edge_type_connecting_endpoint_definitions() {
    let program = parse(
        "CREATE GRAPH TYPE endpoints AS { \
           NODE Person, \
           NODE Company, \
           DIRECTED EDGE Knows LABEL KNOWS {since INTEGER} CONNECTING (Person TO Company), \
           DIRECTED EDGE WorksAt CONNECTING (Company <- Person), \
           UNDIRECTED EDGE Related CONNECTING (Person TO Company) \
         }",
    )
    .unwrap();

    let Statement::CreateGraphType(create) = &program.statements[0] else {
        panic!("expected create graph type");
    };
    let CreateGraphTypeSource::Nested(body) = &create.source else {
        panic!("expected nested graph type source");
    };
    let GraphTypeElement::Edge(knows) = &body.elements[2] else {
        panic!("expected edge type");
    };
    assert_eq!(knows.name, ident("Knows"));
    assert_eq!(knows.direction, Direction::Right);
    assert_eq!(knows.source, Some(ident("Person")));
    assert_eq!(knows.destination, Some(ident("Company")));
    assert_eq!(knows.labels, vec![ident("KNOWS")]);
    assert_eq!(knows.properties[0].name, ident("since"));

    let GraphTypeElement::Edge(works_at) = &body.elements[3] else {
        panic!("expected edge type");
    };
    assert_eq!(works_at.name, ident("WorksAt"));
    assert_eq!(works_at.direction, Direction::Right);
    assert_eq!(works_at.source, Some(ident("Person")));
    assert_eq!(works_at.destination, Some(ident("Company")));
}

#[test]
fn parses_undirected_edge_type_definitions() {
    let program = parse(
        "CREATE GRAPH TYPE undirected_edges AS { \
           NODE Person, \
           NODE Company, \
           (Person)~[Knows LABEL KNOWS]~(Person), \
           UNDIRECTED EDGE Related CONNECTING (Person ~ Company), \
           NODE Holder {rel (:Person)~[:RELATED]~(:Company)} \
         }",
    )
    .unwrap();

    let Statement::CreateGraphType(create) = &program.statements[0] else {
        panic!("expected create graph type");
    };
    let CreateGraphTypeSource::Nested(body) = &create.source else {
        panic!("expected nested graph type source");
    };
    let GraphTypeElement::Edge(knows) = &body.elements[2] else {
        panic!("expected edge type");
    };
    assert_eq!(knows.name, ident("Knows"));
    assert_eq!(knows.direction, Direction::Undirected);
    assert_eq!(knows.source, Some(ident("Person")));
    assert_eq!(knows.destination, Some(ident("Person")));

    let GraphTypeElement::Edge(related) = &body.elements[3] else {
        panic!("expected edge type");
    };
    assert_eq!(related.name, ident("Related"));
    assert_eq!(related.direction, Direction::Undirected);
    assert_eq!(related.source, Some(ident("Person")));
    assert_eq!(related.destination, Some(ident("Company")));

    let GraphTypeElement::Node(holder) = &body.elements[4] else {
        panic!("expected node type");
    };
    let ValueType::EdgeReference {
        definition: Some(edge),
    } = &holder.properties[0].value_type
    else {
        panic!("expected closed edge reference type");
    };
    assert_eq!(edge.name, ident(""));
    assert_eq!(edge.direction, Direction::Undirected);
    assert_eq!(edge.source, None);
    assert_eq!(edge.destination, None);
}

#[test]
fn parses_node_and_edge_synonyms() {
    let program = parse(
        "CREATE GRAPH TYPE synonyms AS { \
           VERTEX Person LABEL Person {name STRING}, \
           DIRECTED RELATIONSHIP Knows LABEL KNOWS CONNECTING (Person TO Person), \
           NODE Holder {v ANY VERTEX, cv VERTEX Person {name STRING}, r ANY RELATIONSHIP, cr (:Person)-[:KNOWS]->(:Person)} \
         }; \
         MATCH DIFFERENT RELATIONSHIP BINDINGS (a)-[r]->(b) RETURN r; \
         MATCH DIFFERENT RELATIONSHIPS (a)-[r]->(b) RETURN r",
    )
    .unwrap();

    let Statement::CreateGraphType(create) = &program.statements[0] else {
        panic!("expected create graph type");
    };
    let CreateGraphTypeSource::Nested(body) = &create.source else {
        panic!("expected nested graph type source");
    };
    let GraphTypeElement::Node(person) = &body.elements[0] else {
        panic!("expected node type");
    };
    assert_eq!(person.name, ident("Person"));
    assert_eq!(person.labels, vec![ident("Person")]);

    let GraphTypeElement::Edge(knows) = &body.elements[1] else {
        panic!("expected edge type");
    };
    assert_eq!(knows.name, ident("Knows"));
    assert_eq!(knows.labels, vec![ident("KNOWS")]);

    let GraphTypeElement::Node(holder) = &body.elements[2] else {
        panic!("expected node type");
    };
    assert!(matches!(
        holder.properties[0].value_type,
        ValueType::NodeReference { definition: None }
    ));
    assert!(matches!(
        holder.properties[1].value_type,
        ValueType::NodeReference {
            definition: Some(_)
        }
    ));
    assert!(matches!(
        holder.properties[2].value_type,
        ValueType::EdgeReference { definition: None }
    ));
    let ValueType::EdgeReference {
        definition: Some(edge),
    } = &holder.properties[3].value_type
    else {
        panic!("expected closed edge reference type");
    };
    assert_eq!(edge.name, ident(""));
    assert_eq!(edge.source, None);
    assert_eq!(edge.destination, None);

    let Statement::Query(single_relationship_query) = &program.statements[1] else {
        panic!("expected query");
    };
    let QueryClause::Match(single_match) = &single_relationship_query.body.clauses[0] else {
        panic!("expected match clause");
    };
    assert_eq!(
        single_match.mode,
        Some(MatchMode::DifferentEdges { bindings: true })
    );

    let Statement::Query(plural_relationship_query) = &program.statements[2] else {
        panic!("expected query");
    };
    let QueryClause::Match(plural_match) = &plural_relationship_query.body.clauses[0] else {
        panic!("expected match clause");
    };
    assert_eq!(
        plural_match.mode,
        Some(MatchMode::DifferentEdges { bindings: false })
    );
}

#[test]
fn parses_type_phrase_type_markers() {
    let program = parse(
        "CREATE GRAPH TYPE typed_phrases AS { \
           NODE TYPE Person LABEL Person {name STRING}, \
           VERTEX TYPE Account {id STRING}, \
           DIRECTED EDGE TYPE Knows LABEL KNOWS CONNECTING (Person TO Account), \
           DIRECTED RELATIONSHIP TYPE Owns CONNECTING (Account <- Person), \
           NODE Holder {person NODE TYPE Person {name STRING}, account VERTEX TYPE Account {id STRING}, relation (:Person)-[:KNOWS]->(:Account)} \
         }",
    )
    .unwrap();

    let Statement::CreateGraphType(create) = &program.statements[0] else {
        panic!("expected create graph type");
    };
    let CreateGraphTypeSource::Nested(body) = &create.source else {
        panic!("expected nested graph type source");
    };
    let GraphTypeElement::Node(person) = &body.elements[0] else {
        panic!("expected node type");
    };
    assert_eq!(person.name, ident("Person"));
    assert_eq!(person.labels, vec![ident("Person")]);

    let GraphTypeElement::Node(account) = &body.elements[1] else {
        panic!("expected node type");
    };
    assert_eq!(account.name, ident("Account"));

    let GraphTypeElement::Edge(knows) = &body.elements[2] else {
        panic!("expected edge type");
    };
    assert_eq!(knows.name, ident("Knows"));
    assert_eq!(knows.source, Some(ident("Person")));
    assert_eq!(knows.destination, Some(ident("Account")));

    let GraphTypeElement::Edge(owns) = &body.elements[3] else {
        panic!("expected edge type");
    };
    assert_eq!(owns.name, ident("Owns"));
    assert_eq!(owns.source, Some(ident("Person")));
    assert_eq!(owns.destination, Some(ident("Account")));

    let GraphTypeElement::Node(holder) = &body.elements[4] else {
        panic!("expected node type");
    };
    let ValueType::NodeReference {
        definition: Some(person_ref),
    } = &holder.properties[0].value_type
    else {
        panic!("expected closed node reference type");
    };
    assert_eq!(person_ref.name, ident("Person"));

    let ValueType::NodeReference {
        definition: Some(account_ref),
    } = &holder.properties[1].value_type
    else {
        panic!("expected closed node reference type");
    };
    assert_eq!(account_ref.name, ident("Account"));

    let ValueType::EdgeReference {
        definition: Some(relation_ref),
    } = &holder.properties[2].value_type
    else {
        panic!("expected closed edge reference type");
    };
    assert_eq!(relation_ref.name, ident(""));
    assert_eq!(relation_ref.source, None);
    assert_eq!(relation_ref.destination, None);
}

#[test]
fn parses_not_null_value_types() {
    let program = parse(
        "CREATE GRAPH TYPE strict AS { \
           NODE Person {name STRING NOT NULL, tags LIST<STRING NOT NULL> NOT NULL} \
         }; \
         RETURN CAST($age AS INTEGER NOT NULL) AS age; \
         MATCH (n) WHERE n.tags IS TYPED LIST<STRING> NOT NULL RETURN n",
    )
    .unwrap();

    let Statement::CreateGraphType(create) = &program.statements[0] else {
        panic!("expected create graph type");
    };
    let CreateGraphTypeSource::Nested(body) = &create.source else {
        panic!("expected nested graph type source");
    };
    let GraphTypeElement::Node(node) = &body.elements[0] else {
        panic!("expected node type");
    };
    assert!(matches!(
        node.properties[0].value_type,
        ValueType::NotNull(_)
    ));
    let ValueType::NotNull(tags) = &node.properties[1].value_type else {
        panic!("expected outer not null list");
    };
    let ValueType::List(inner) = tags.as_ref() else {
        panic!("expected list value type");
    };
    assert!(matches!(inner.as_ref(), ValueType::NotNull(_)));

    let Statement::Query(cast_query) = &program.statements[1] else {
        panic!("expected query");
    };
    let Expr::Cast { value_type, .. } = &cast_query.body.result_clause.items[0].expr else {
        panic!("expected cast expression");
    };
    assert!(matches!(value_type, ValueType::NotNull(_)));

    let Statement::Query(predicate_query) = &program.statements[2] else {
        panic!("expected query");
    };
    let QueryClause::Match(match_clause) = &predicate_query.body.clauses[0] else {
        panic!("expected match clause");
    };
    let Some(Expr::IsTyped { value_type, .. }) = &match_clause.where_clause else {
        panic!("expected is typed predicate");
    };
    assert!(matches!(value_type, ValueType::NotNull(_)));
}

#[test]
fn parses_list_value_type_postfix_and_max_length() {
    let program = parse(
        "CREATE GRAPH TYPE sized AS { \
           NODE Person {aliases STRING LIST[5], scores ARRAY<INTEGER>[3], tags STRING NOT NULL LIST NOT NULL} \
         }; \
         RETURN CAST($names AS STRING LIST) AS names; \
         MATCH (n) WHERE n.scores IS TYPED INTEGER ARRAY[3] RETURN n",
    )
    .unwrap();

    let Statement::CreateGraphType(create) = &program.statements[0] else {
        panic!("expected create graph type");
    };
    let CreateGraphTypeSource::Nested(body) = &create.source else {
        panic!("expected nested graph type source");
    };
    let GraphTypeElement::Node(node) = &body.elements[0] else {
        panic!("expected node type");
    };

    assert!(matches!(
        node.properties[0].value_type,
        ValueType::ListWithLength { max_length: 5, .. }
    ));
    assert!(matches!(
        node.properties[1].value_type,
        ValueType::ArrayWithLength { max_length: 3, .. }
    ));
    let ValueType::NotNull(tags) = &node.properties[2].value_type else {
        panic!("expected outer not null list");
    };
    let ValueType::List(inner) = tags.as_ref() else {
        panic!("expected postfix list type");
    };
    assert!(matches!(inner.as_ref(), ValueType::NotNull(_)));

    let Statement::Query(cast_query) = &program.statements[1] else {
        panic!("expected query");
    };
    let Expr::Cast { value_type, .. } = &cast_query.body.result_clause.items[0].expr else {
        panic!("expected cast expression");
    };
    assert!(matches!(value_type, ValueType::List(_)));

    let Statement::Query(predicate_query) = &program.statements[2] else {
        panic!("expected query");
    };
    let QueryClause::Match(match_clause) = &predicate_query.body.clauses[0] else {
        panic!("expected match clause");
    };
    let Some(Expr::IsTyped { value_type, .. }) = &match_clause.where_clause else {
        panic!("expected is typed predicate");
    };
    assert!(matches!(
        value_type,
        ValueType::ArrayWithLength { max_length: 3, .. }
    ));
}

#[test]
fn parses_path_and_list_value_types() {
    let program = parse(
        "CREATE GRAPH TYPE paths AS { \
           NODE Route {route PATH, nodes LIST<NODE>[4], edges EDGE ARRAY NOT NULL} \
         }; \
         RETURN CAST($path AS PATH NOT NULL) AS path; \
         MATCH (n) WHERE n.nodes IS TYPED LIST<NODE> RETURN n",
    )
    .unwrap();

    let Statement::CreateGraphType(create) = &program.statements[0] else {
        panic!("expected create graph type");
    };
    let CreateGraphTypeSource::Nested(body) = &create.source else {
        panic!("expected nested graph type source");
    };
    let GraphTypeElement::Node(node) = &body.elements[0] else {
        panic!("expected node type");
    };

    assert_eq!(node.properties[0].value_type, ValueType::Path);
    assert!(matches!(
        node.properties[1].value_type,
        ValueType::ListWithLength { max_length: 4, .. }
    ));
    assert!(matches!(
        node.properties[2].value_type,
        ValueType::NotNull(ref inner) if matches!(inner.as_ref(), ValueType::Array(_))
    ));

    let Statement::Query(cast_query) = &program.statements[1] else {
        panic!("expected query");
    };
    let Expr::Cast { value_type, .. } = &cast_query.body.result_clause.items[0].expr else {
        panic!("expected cast expression");
    };
    assert_eq!(value_type, &ValueType::NotNull(Box::new(ValueType::Path)));

    let Statement::Query(predicate_query) = &program.statements[2] else {
        panic!("expected query");
    };
    let QueryClause::Match(match_clause) = &predicate_query.body.clauses[0] else {
        panic!("expected match clause");
    };
    let Some(Expr::IsTyped { value_type, .. }) = &match_clause.where_clause else {
        panic!("expected is typed predicate");
    };
    assert!(matches!(value_type, ValueType::List(_)));
}

#[test]
fn parses_record_value_type_forms() {
    let program = parse(
        "CREATE GRAPH TYPE records AS { \
           NODE Person {open RECORD, any_open ANY RECORD NOT NULL, closed {score INTEGER}, explicit RECORD {score TYPED INTEGER}} \
         }; \
         RETURN CAST($payload AS {name STRING, tags STRING LIST}) AS payload; \
         MATCH (n) WHERE n.meta IS TYPED RECORD NOT NULL RETURN n",
    )
    .unwrap();

    let Statement::CreateGraphType(create) = &program.statements[0] else {
        panic!("expected create graph type");
    };
    let CreateGraphTypeSource::Nested(body) = &create.source else {
        panic!("expected nested graph type source");
    };
    let GraphTypeElement::Node(node) = &body.elements[0] else {
        panic!("expected node type");
    };

    assert_eq!(node.properties[0].value_type, ValueType::AnyRecord);
    assert_eq!(
        node.properties[1].value_type,
        ValueType::NotNull(Box::new(ValueType::AnyRecord))
    );
    assert!(matches!(
        node.properties[2].value_type,
        ValueType::Record(ref fields) if fields.len() == 1
    ));
    assert!(matches!(
        node.properties[3].value_type,
        ValueType::Record(ref fields) if fields.len() == 1
    ));

    let Statement::Query(cast_query) = &program.statements[1] else {
        panic!("expected query");
    };
    let Expr::Cast { value_type, .. } = &cast_query.body.result_clause.items[0].expr else {
        panic!("expected cast expression");
    };
    assert!(matches!(
        value_type,
        ValueType::Record(fields) if fields.len() == 2
    ));

    let Statement::Query(predicate_query) = &program.statements[2] else {
        panic!("expected query");
    };
    let QueryClause::Match(match_clause) = &predicate_query.body.clauses[0] else {
        panic!("expected match clause");
    };
    let Some(Expr::IsTyped { value_type, .. }) = &match_clause.where_clause else {
        panic!("expected is typed predicate");
    };
    assert_eq!(
        value_type,
        &ValueType::NotNull(Box::new(ValueType::AnyRecord))
    );
}

#[test]
fn parses_character_string_value_types() {
    let program = parse(
        "CREATE GRAPH TYPE strings AS { \
           NODE Person {name STRING, handle VARCHAR(32), required STRING(255) NOT NULL} \
         }; \
         RETURN CAST($name AS VARCHAR(64)) AS name; \
         MATCH (n) WHERE n.name IS TYPED STRING(128) RETURN n",
    )
    .unwrap();

    let Statement::CreateGraphType(create) = &program.statements[0] else {
        panic!("expected create graph type");
    };
    let CreateGraphTypeSource::Nested(body) = &create.source else {
        panic!("expected nested graph type source");
    };
    let GraphTypeElement::Node(node) = &body.elements[0] else {
        panic!("expected node type");
    };

    assert_eq!(
        node.properties[0].value_type,
        ValueType::CharacterString {
            kind: CharacterStringTypeKind::String,
            max_length: None,
        }
    );
    assert_eq!(
        node.properties[1].value_type,
        ValueType::CharacterString {
            kind: CharacterStringTypeKind::Varchar,
            max_length: Some(32),
        }
    );
    assert_eq!(
        node.properties[2].value_type,
        ValueType::NotNull(Box::new(ValueType::CharacterString {
            kind: CharacterStringTypeKind::String,
            max_length: Some(255),
        }))
    );

    let Statement::Query(cast_query) = &program.statements[1] else {
        panic!("expected query");
    };
    let Expr::Cast { value_type, .. } = &cast_query.body.result_clause.items[0].expr else {
        panic!("expected cast expression");
    };
    assert_eq!(
        value_type,
        &ValueType::CharacterString {
            kind: CharacterStringTypeKind::Varchar,
            max_length: Some(64),
        }
    );

    let Statement::Query(predicate_query) = &program.statements[2] else {
        panic!("expected query");
    };
    let QueryClause::Match(match_clause) = &predicate_query.body.clauses[0] else {
        panic!("expected match clause");
    };
    let Some(Expr::IsTyped { value_type, .. }) = &match_clause.where_clause else {
        panic!("expected is typed predicate");
    };
    assert_eq!(
        value_type,
        &ValueType::CharacterString {
            kind: CharacterStringTypeKind::String,
            max_length: Some(128),
        }
    );
}

#[test]
fn parses_boolean_value_types() {
    let program = parse(
        "CREATE GRAPH TYPE flags AS { \
           NODE Feature {enabled BOOL, visible BOOLEAN NOT NULL} \
         }; \
         RETURN CAST($enabled AS BOOLEAN) AS enabled; \
         MATCH (n) WHERE n.enabled IS TYPED BOOL RETURN n",
    )
    .unwrap();

    let Statement::CreateGraphType(create) = &program.statements[0] else {
        panic!("expected create graph type");
    };
    let CreateGraphTypeSource::Nested(body) = &create.source else {
        panic!("expected nested graph type source");
    };
    let GraphTypeElement::Node(node) = &body.elements[0] else {
        panic!("expected node type");
    };

    assert_eq!(
        node.properties[0].value_type,
        ValueType::Boolean {
            kind: BooleanTypeKind::Bool,
        }
    );
    assert_eq!(
        node.properties[1].value_type,
        ValueType::NotNull(Box::new(ValueType::Boolean {
            kind: BooleanTypeKind::Boolean,
        }))
    );

    let Statement::Query(cast_query) = &program.statements[1] else {
        panic!("expected query");
    };
    let Expr::Cast { value_type, .. } = &cast_query.body.result_clause.items[0].expr else {
        panic!("expected cast expression");
    };
    assert_eq!(
        value_type,
        &ValueType::Boolean {
            kind: BooleanTypeKind::Boolean,
        }
    );

    let Statement::Query(predicate_query) = &program.statements[2] else {
        panic!("expected query");
    };
    let QueryClause::Match(match_clause) = &predicate_query.body.clauses[0] else {
        panic!("expected match clause");
    };
    let Some(Expr::IsTyped { value_type, .. }) = &match_clause.where_clause else {
        panic!("expected is typed predicate");
    };
    assert_eq!(
        value_type,
        &ValueType::Boolean {
            kind: BooleanTypeKind::Bool,
        }
    );
}

#[test]
fn parses_temporal_value_types() {
    let program = parse(
        "CREATE GRAPH TYPE events AS { \
           NODE Event {created ZONED DATETIME, observed TIMESTAMP WITH TIME ZONE, local_created LOCAL DATETIME, stored TIMESTAMP WITHOUT TIME ZONE, day DATE, zoned_at ZONED TIME, local_at TIME WITHOUT TIME ZONE, elapsed DURATION NOT NULL} \
         }; \
         RETURN CAST($created AS TIMESTAMP) AS created; \
         MATCH (n) WHERE n.local_at IS TYPED LOCAL TIME RETURN n",
    )
    .unwrap();

    let Statement::CreateGraphType(create) = &program.statements[0] else {
        panic!("expected create graph type");
    };
    let CreateGraphTypeSource::Nested(body) = &create.source else {
        panic!("expected nested graph type source");
    };
    let GraphTypeElement::Node(node) = &body.elements[0] else {
        panic!("expected node type");
    };

    assert_eq!(
        node.properties[0].value_type,
        ValueType::Temporal {
            kind: TemporalTypeKind::ZonedDateTime,
        }
    );
    assert_eq!(
        node.properties[1].value_type,
        ValueType::Temporal {
            kind: TemporalTypeKind::ZonedDateTime,
        }
    );
    assert_eq!(
        node.properties[2].value_type,
        ValueType::Temporal {
            kind: TemporalTypeKind::LocalDateTime,
        }
    );
    assert_eq!(
        node.properties[3].value_type,
        ValueType::Temporal {
            kind: TemporalTypeKind::LocalDateTime,
        }
    );
    assert_eq!(
        node.properties[4].value_type,
        ValueType::Temporal {
            kind: TemporalTypeKind::Date,
        }
    );
    assert_eq!(
        node.properties[5].value_type,
        ValueType::Temporal {
            kind: TemporalTypeKind::ZonedTime,
        }
    );
    assert_eq!(
        node.properties[6].value_type,
        ValueType::Temporal {
            kind: TemporalTypeKind::LocalTime,
        }
    );
    assert_eq!(
        node.properties[7].value_type,
        ValueType::NotNull(Box::new(ValueType::Temporal {
            kind: TemporalTypeKind::Duration,
        }))
    );

    let Statement::Query(cast_query) = &program.statements[1] else {
        panic!("expected query");
    };
    let Expr::Cast { value_type, .. } = &cast_query.body.result_clause.items[0].expr else {
        panic!("expected cast expression");
    };
    assert_eq!(
        value_type,
        &ValueType::Temporal {
            kind: TemporalTypeKind::LocalDateTime,
        }
    );

    let Statement::Query(predicate_query) = &program.statements[2] else {
        panic!("expected query");
    };
    let QueryClause::Match(match_clause) = &predicate_query.body.clauses[0] else {
        panic!("expected match clause");
    };
    let Some(Expr::IsTyped { value_type, .. }) = &match_clause.where_clause else {
        panic!("expected is typed predicate");
    };
    assert_eq!(
        value_type,
        &ValueType::Temporal {
            kind: TemporalTypeKind::LocalTime,
        }
    );
}

#[test]
fn parses_dynamic_union_value_types() {
    let program = parse(
        "CREATE GRAPH TYPE dynamic AS { \
           NODE Item {payload ANY, required ANY VALUE NOT NULL, prop PROPERTY VALUE, any_prop ANY PROPERTY VALUE NOT NULL, unioned STRING | INTEGER, closed ANY VALUE<STRING | INTEGER>, nested LIST<STRING | INTEGER>} \
         }; \
         RETURN CAST($payload AS ANY<STRING | INTEGER>) AS payload; \
         MATCH (n) WHERE n.payload IS TYPED PROPERTY VALUE RETURN n",
    )
    .unwrap();

    let Statement::CreateGraphType(create) = &program.statements[0] else {
        panic!("expected create graph type");
    };
    let CreateGraphTypeSource::Nested(body) = &create.source else {
        panic!("expected nested graph type source");
    };
    let GraphTypeElement::Node(node) = &body.elements[0] else {
        panic!("expected node type");
    };

    assert_eq!(node.properties[0].value_type, ValueType::AnyDynamic);
    assert_eq!(
        node.properties[1].value_type,
        ValueType::NotNull(Box::new(ValueType::AnyDynamic))
    );
    assert_eq!(node.properties[2].value_type, ValueType::PropertyValue);
    assert_eq!(
        node.properties[3].value_type,
        ValueType::NotNull(Box::new(ValueType::PropertyValue))
    );
    assert!(matches!(
        node.properties[4].value_type,
        ValueType::DynamicUnion(ref components) if components.len() == 2
    ));
    assert!(matches!(
        node.properties[5].value_type,
        ValueType::DynamicUnion(ref components) if components.len() == 2
    ));
    assert!(matches!(
        node.properties[6].value_type,
        ValueType::List(ref inner)
            if matches!(inner.as_ref(), ValueType::DynamicUnion(components) if components.len() == 2)
    ));

    let Statement::Query(cast_query) = &program.statements[1] else {
        panic!("expected query");
    };
    let Expr::Cast { value_type, .. } = &cast_query.body.result_clause.items[0].expr else {
        panic!("expected cast expression");
    };
    assert!(matches!(
        value_type,
        ValueType::DynamicUnion(components) if components.len() == 2
    ));

    let Statement::Query(predicate_query) = &program.statements[2] else {
        panic!("expected query");
    };
    let QueryClause::Match(match_clause) = &predicate_query.body.clauses[0] else {
        panic!("expected match clause");
    };
    let Some(Expr::IsTyped { value_type, .. }) = &match_clause.where_clause else {
        panic!("expected is typed predicate");
    };
    assert_eq!(value_type, &ValueType::PropertyValue);
}

#[test]
fn parses_reference_value_types() {
    let program = parse(
        "CREATE GRAPH TYPE refs AS { \
           NODE Holder {g ANY GRAPH, pg ANY PROPERTY GRAPH NOT NULL, closed_graph GRAPH { NODE Person {name STRING} }, n ANY NODE, cn NODE Person {name STRING}, e ANY EDGE, ce (:Person)-[:KNOWS {since INTEGER}]->(:Person), anon_node (:Person {name STRING}), anon_edge (:Person)->(:Company)} \
         }; \
         RETURN CAST($g AS PROPERTY GRAPH { NODE City {name STRING} }) AS g; \
         MATCH (n) WHERE n IS TYPED ANY NODE RETURN n",
    )
    .unwrap();

    let Statement::CreateGraphType(create) = &program.statements[0] else {
        panic!("expected create graph type");
    };
    let CreateGraphTypeSource::Nested(body) = &create.source else {
        panic!("expected nested graph type source");
    };
    let GraphTypeElement::Node(node) = &body.elements[0] else {
        panic!("expected node type");
    };

    assert_eq!(
        node.properties[0].value_type,
        ValueType::GraphReference {
            property_graph: false,
            body: None,
        }
    );
    assert_eq!(
        node.properties[1].value_type,
        ValueType::NotNull(Box::new(ValueType::GraphReference {
            property_graph: true,
            body: None,
        }))
    );
    assert!(matches!(
        node.properties[2].value_type,
        ValueType::GraphReference {
            property_graph: false,
            body: Some(ref body),
        } if body.elements.len() == 1
    ));
    assert_eq!(
        node.properties[3].value_type,
        ValueType::NodeReference { definition: None }
    );
    assert!(matches!(
        node.properties[4].value_type,
        ValueType::NodeReference {
            definition: Some(ref definition),
        } if definition.name == ident("Person") && definition.properties.len() == 1
    ));
    assert_eq!(
        node.properties[5].value_type,
        ValueType::EdgeReference { definition: None }
    );
    assert!(matches!(
        node.properties[6].value_type,
        ValueType::EdgeReference {
            definition: Some(ref definition),
        } if definition.name == ident("")
            && definition.source.is_none()
            && definition.destination.is_none()
            && definition.properties.len() == 1
    ));
    assert!(matches!(
        node.properties[7].value_type,
        ValueType::NodeReference {
            definition: Some(ref definition),
        } if definition.name == ident("")
            && definition.labels == vec![ident("Person")]
            && definition.properties.len() == 1
    ));
    assert!(matches!(
        node.properties[8].value_type,
        ValueType::EdgeReference {
            definition: Some(ref definition),
        } if definition.name == ident("")
            && definition.source.is_none()
            && definition.destination.is_none()
    ));

    let Statement::Query(cast_query) = &program.statements[1] else {
        panic!("expected query");
    };
    let Expr::Cast { value_type, .. } = &cast_query.body.result_clause.items[0].expr else {
        panic!("expected cast expression");
    };
    assert!(matches!(
        value_type,
        ValueType::GraphReference {
            property_graph: true,
            body: Some(body),
        } if body.elements.len() == 1
    ));

    let Statement::Query(predicate_query) = &program.statements[2] else {
        panic!("expected query");
    };
    let QueryClause::Match(match_clause) = &predicate_query.body.clauses[0] else {
        panic!("expected match clause");
    };
    let Some(Expr::IsTyped { value_type, .. }) = &match_clause.where_clause else {
        panic!("expected is typed predicate");
    };
    assert_eq!(value_type, &ValueType::NodeReference { definition: None });
}

#[test]
fn parses_binding_table_reference_value_types() {
    let program = parse(
        "CREATE GRAPH TYPE table_refs AS { \
           NODE Holder {rows TABLE {id INTEGER, name STRING}, bindings BINDING TABLE {node ANY NODE, score INTEGER} NOT NULL} \
         }; \
         RETURN CAST($rows AS TABLE {id INTEGER}) AS rows; \
         MATCH (n) WHERE $rows IS TYPED BINDING TABLE {node ANY NODE} RETURN n",
    )
    .unwrap();

    let Statement::CreateGraphType(create) = &program.statements[0] else {
        panic!("expected create graph type");
    };
    let CreateGraphTypeSource::Nested(body) = &create.source else {
        panic!("expected nested graph type source");
    };
    let GraphTypeElement::Node(node) = &body.elements[0] else {
        panic!("expected node type");
    };

    assert!(matches!(
        node.properties[0].value_type,
        ValueType::BindingTable {
            binding: false,
            ref fields,
        } if fields.len() == 2
    ));
    let ValueType::NotNull(bindings) = &node.properties[1].value_type else {
        panic!("expected not null binding table");
    };
    assert!(matches!(
        bindings.as_ref(),
        ValueType::BindingTable {
            binding: true,
            fields,
        } if fields.len() == 2
    ));

    let Statement::Query(cast_query) = &program.statements[1] else {
        panic!("expected query");
    };
    let Expr::Cast { value_type, .. } = &cast_query.body.result_clause.items[0].expr else {
        panic!("expected cast expression");
    };
    assert!(matches!(
        value_type,
        ValueType::BindingTable {
            binding: false,
            fields,
        } if fields.len() == 1
    ));

    let Statement::Query(predicate_query) = &program.statements[2] else {
        panic!("expected query");
    };
    let QueryClause::Match(match_clause) = &predicate_query.body.clauses[0] else {
        panic!("expected match clause");
    };
    let Some(Expr::IsTyped { value_type, .. }) = &match_clause.where_clause else {
        panic!("expected is typed predicate");
    };
    assert!(matches!(
        value_type,
        ValueType::BindingTable {
            binding: true,
            fields,
        } if fields.len() == 1
    ));
}

#[test]
fn parses_byte_string_value_types() {
    let program = parse(
        "CREATE GRAPH TYPE bytes AS { \
           NODE Blob {raw BYTES, bounded BYTES(2, 16), digest BINARY(32), default_digest BINARY, chunk VARBINARY(1024) NOT NULL} \
         }; \
         RETURN CAST($raw AS BYTES(16)) AS raw; \
         MATCH (n) WHERE n.raw IS TYPED VARBINARY(512) RETURN n",
    )
    .unwrap();

    let Statement::CreateGraphType(create) = &program.statements[0] else {
        panic!("expected create graph type");
    };
    let CreateGraphTypeSource::Nested(body) = &create.source else {
        panic!("expected nested graph type source");
    };
    let GraphTypeElement::Node(node) = &body.elements[0] else {
        panic!("expected node type");
    };

    assert_eq!(
        node.properties[0].value_type,
        ValueType::ByteString {
            kind: ByteStringTypeKind::Bytes,
            min_length: Some(0),
            max_length: None,
        }
    );
    assert_eq!(
        node.properties[1].value_type,
        ValueType::ByteString {
            kind: ByteStringTypeKind::Bytes,
            min_length: Some(2),
            max_length: Some(16),
        }
    );
    assert_eq!(
        node.properties[2].value_type,
        ValueType::ByteString {
            kind: ByteStringTypeKind::Binary,
            min_length: Some(32),
            max_length: Some(32),
        }
    );
    assert_eq!(
        node.properties[3].value_type,
        ValueType::ByteString {
            kind: ByteStringTypeKind::Binary,
            min_length: Some(1),
            max_length: Some(1),
        }
    );
    assert_eq!(
        node.properties[4].value_type,
        ValueType::NotNull(Box::new(ValueType::ByteString {
            kind: ByteStringTypeKind::Varbinary,
            min_length: Some(0),
            max_length: Some(1024),
        }))
    );

    let Statement::Query(cast_query) = &program.statements[1] else {
        panic!("expected query");
    };
    let Expr::Cast { value_type, .. } = &cast_query.body.result_clause.items[0].expr else {
        panic!("expected cast expression");
    };
    assert_eq!(
        value_type,
        &ValueType::ByteString {
            kind: ByteStringTypeKind::Bytes,
            min_length: Some(0),
            max_length: Some(16),
        }
    );

    let Statement::Query(predicate_query) = &program.statements[2] else {
        panic!("expected query");
    };
    let QueryClause::Match(match_clause) = &predicate_query.body.clauses[0] else {
        panic!("expected match clause");
    };
    let Some(Expr::IsTyped { value_type, .. }) = &match_clause.where_clause else {
        panic!("expected is typed predicate");
    };
    assert_eq!(
        value_type,
        &ValueType::ByteString {
            kind: ByteStringTypeKind::Varbinary,
            min_length: Some(0),
            max_length: Some(512),
        }
    );
}

#[test]
fn parses_precision_scale_numeric_value_types() {
    let program = parse(
        "CREATE GRAPH TYPE numbers AS { \
           NODE Measurement {price DECIMAL(12, 2), ratio DEC(10), estimate FLOAT(24, 4), score FLOAT NOT NULL} \
         }; \
         RETURN CAST($price AS DECIMAL(8, 2)) AS price; \
         MATCH (n) WHERE n.score IS TYPED FLOAT(32) RETURN n",
    )
    .unwrap();

    let Statement::CreateGraphType(create) = &program.statements[0] else {
        panic!("expected create graph type");
    };
    let CreateGraphTypeSource::Nested(body) = &create.source else {
        panic!("expected nested graph type source");
    };
    let GraphTypeElement::Node(node) = &body.elements[0] else {
        panic!("expected node type");
    };

    assert_eq!(
        node.properties[0].value_type,
        ValueType::ExactNumeric {
            kind: ExactNumericTypeKind::Decimal,
            signed: None,
            precision: Some(12),
            scale: Some(2),
        }
    );
    assert_eq!(
        node.properties[1].value_type,
        ValueType::ExactNumeric {
            kind: ExactNumericTypeKind::Decimal,
            signed: None,
            precision: Some(10),
            scale: None,
        }
    );
    assert_eq!(
        node.properties[2].value_type,
        ValueType::ApproximateNumeric {
            kind: ApproximateNumericTypeKind::Float,
            precision: Some(24),
            scale: Some(4),
        }
    );
    assert_eq!(
        node.properties[3].value_type,
        ValueType::NotNull(Box::new(ValueType::ApproximateNumeric {
            kind: ApproximateNumericTypeKind::Float,
            precision: None,
            scale: None,
        }))
    );

    let Statement::Query(cast_query) = &program.statements[1] else {
        panic!("expected query");
    };
    let Expr::Cast { value_type, .. } = &cast_query.body.result_clause.items[0].expr else {
        panic!("expected cast expression");
    };
    assert_eq!(
        value_type,
        &ValueType::ExactNumeric {
            kind: ExactNumericTypeKind::Decimal,
            signed: None,
            precision: Some(8),
            scale: Some(2),
        }
    );

    let Statement::Query(predicate_query) = &program.statements[2] else {
        panic!("expected query");
    };
    let QueryClause::Match(match_clause) = &predicate_query.body.clauses[0] else {
        panic!("expected match clause");
    };
    let Some(Expr::IsTyped { value_type, .. }) = &match_clause.where_clause else {
        panic!("expected is typed predicate");
    };
    assert_eq!(
        value_type,
        &ValueType::ApproximateNumeric {
            kind: ApproximateNumericTypeKind::Float,
            precision: Some(32),
            scale: None,
        }
    );
}

#[test]
fn parses_approximate_numeric_value_types() {
    let program = parse(
        "CREATE GRAPH TYPE measurements AS { \
           NODE Measurement {half FLOAT16, precise FLOAT256, real_value REAL, double_value DOUBLE PRECISION NOT NULL} \
         }; \
         RETURN CAST($score AS DOUBLE) AS score; \
         MATCH (n) WHERE n.score IS TYPED FLOAT64 RETURN n",
    )
    .unwrap();

    let Statement::CreateGraphType(create) = &program.statements[0] else {
        panic!("expected create graph type");
    };
    let CreateGraphTypeSource::Nested(body) = &create.source else {
        panic!("expected nested graph type source");
    };
    let GraphTypeElement::Node(node) = &body.elements[0] else {
        panic!("expected node type");
    };

    assert_eq!(
        node.properties[0].value_type,
        ValueType::ApproximateNumeric {
            kind: ApproximateNumericTypeKind::Float,
            precision: Some(16),
            scale: None,
        }
    );
    assert_eq!(
        node.properties[1].value_type,
        ValueType::ApproximateNumeric {
            kind: ApproximateNumericTypeKind::Float,
            precision: Some(256),
            scale: None,
        }
    );
    assert_eq!(
        node.properties[2].value_type,
        ValueType::ApproximateNumeric {
            kind: ApproximateNumericTypeKind::Real,
            precision: None,
            scale: None,
        }
    );
    assert_eq!(
        node.properties[3].value_type,
        ValueType::NotNull(Box::new(ValueType::ApproximateNumeric {
            kind: ApproximateNumericTypeKind::Double,
            precision: None,
            scale: None,
        }))
    );

    let Statement::Query(cast_query) = &program.statements[1] else {
        panic!("expected query");
    };
    let Expr::Cast { value_type, .. } = &cast_query.body.result_clause.items[0].expr else {
        panic!("expected cast expression");
    };
    assert_eq!(
        value_type,
        &ValueType::ApproximateNumeric {
            kind: ApproximateNumericTypeKind::Double,
            precision: None,
            scale: None,
        }
    );

    let Statement::Query(predicate_query) = &program.statements[2] else {
        panic!("expected query");
    };
    let QueryClause::Match(match_clause) = &predicate_query.body.clauses[0] else {
        panic!("expected match clause");
    };
    let Some(Expr::IsTyped { value_type, .. }) = &match_clause.where_clause else {
        panic!("expected is typed predicate");
    };
    assert_eq!(
        value_type,
        &ValueType::ApproximateNumeric {
            kind: ApproximateNumericTypeKind::Float,
            precision: Some(64),
            scale: None,
        }
    );
}

#[test]
fn parses_binary_exact_numeric_value_types() {
    let program = parse(
        "CREATE GRAPH TYPE ints AS { \
           NODE Measurement {tiny INT8, unsigned16 UINT16, regular UNSIGNED INTEGER(32), small SIGNED SMALL INTEGER, big BIGINT NOT NULL} \
         }; \
         RETURN CAST($value AS UNSIGNED BIG INTEGER) AS value; \
         MATCH (n) WHERE n.value IS TYPED INT(64) RETURN n",
    )
    .unwrap();

    let Statement::CreateGraphType(create) = &program.statements[0] else {
        panic!("expected create graph type");
    };
    let CreateGraphTypeSource::Nested(body) = &create.source else {
        panic!("expected nested graph type source");
    };
    let GraphTypeElement::Node(node) = &body.elements[0] else {
        panic!("expected node type");
    };

    assert_eq!(
        node.properties[0].value_type,
        ValueType::ExactNumeric {
            kind: ExactNumericTypeKind::Integer,
            signed: Some(true),
            precision: Some(8),
            scale: None,
        }
    );
    assert_eq!(
        node.properties[1].value_type,
        ValueType::ExactNumeric {
            kind: ExactNumericTypeKind::Integer,
            signed: Some(false),
            precision: Some(16),
            scale: None,
        }
    );
    assert_eq!(
        node.properties[2].value_type,
        ValueType::ExactNumeric {
            kind: ExactNumericTypeKind::Integer,
            signed: Some(false),
            precision: Some(32),
            scale: None,
        }
    );
    assert_eq!(
        node.properties[3].value_type,
        ValueType::ExactNumeric {
            kind: ExactNumericTypeKind::Integer,
            signed: Some(true),
            precision: None,
            scale: None,
        }
    );
    assert_eq!(
        node.properties[4].value_type,
        ValueType::NotNull(Box::new(ValueType::ExactNumeric {
            kind: ExactNumericTypeKind::Integer,
            signed: Some(true),
            precision: None,
            scale: None,
        }))
    );

    let Statement::Query(cast_query) = &program.statements[1] else {
        panic!("expected query");
    };
    let Expr::Cast { value_type, .. } = &cast_query.body.result_clause.items[0].expr else {
        panic!("expected cast expression");
    };
    assert_eq!(
        value_type,
        &ValueType::ExactNumeric {
            kind: ExactNumericTypeKind::Integer,
            signed: Some(false),
            precision: None,
            scale: None,
        }
    );

    let Statement::Query(predicate_query) = &program.statements[2] else {
        panic!("expected query");
    };
    let QueryClause::Match(match_clause) = &predicate_query.body.clauses[0] else {
        panic!("expected match clause");
    };
    let Some(Expr::IsTyped { value_type, .. }) = &match_clause.where_clause else {
        panic!("expected is typed predicate");
    };
    assert_eq!(
        value_type,
        &ValueType::ExactNumeric {
            kind: ExactNumericTypeKind::Integer,
            signed: Some(true),
            precision: Some(64),
            scale: None,
        }
    );
}

#[test]
fn parses_numeric_prefix_type_names_as_named_types() {
    let program = parse(
        "CREATE GRAPH TYPE custom AS { \
           NODE Item {intensity INTENSITY, unsigned_token UINTOKEN, floating FLOATING} \
         }",
    )
    .unwrap();

    let Statement::CreateGraphType(create) = &program.statements[0] else {
        panic!("expected create graph type");
    };
    let CreateGraphTypeSource::Nested(body) = &create.source else {
        panic!("expected nested graph type source");
    };
    let GraphTypeElement::Node(node) = &body.elements[0] else {
        panic!("expected node type");
    };

    assert_eq!(
        node.properties[0].value_type,
        ValueType::Named {
            name: ident("INTENSITY"),
            parameters: Vec::new()
        }
    );
    assert_eq!(
        node.properties[1].value_type,
        ValueType::Named {
            name: ident("UINTOKEN"),
            parameters: Vec::new()
        }
    );
    assert_eq!(
        node.properties[2].value_type,
        ValueType::Named {
            name: ident("FLOATING"),
            parameters: Vec::new()
        }
    );
}

#[test]
fn parses_multi_word_value_types() {
    let program = parse(
        "CREATE GRAPH TYPE typed AS { \
           NODE Measurement {value DOUBLE PRECISION, label CHARACTER VARYING(40), captured_at TIMESTAMP WITH TIME ZONE} \
         }; \
         RETURN CAST($started AS TIMESTAMP WITHOUT TIME ZONE) AS started; \
         MATCH (n) WHERE n.duration IS TYPED TIME WITH TIME ZONE RETURN n",
    )
    .unwrap();

    let Statement::CreateGraphType(create) = &program.statements[0] else {
        panic!("expected create graph type");
    };
    let CreateGraphTypeSource::Nested(body) = &create.source else {
        panic!("expected nested graph type source");
    };
    let GraphTypeElement::Node(node) = &body.elements[0] else {
        panic!("expected node type");
    };
    assert_eq!(
        node.properties[0].value_type,
        ValueType::ApproximateNumeric {
            kind: ApproximateNumericTypeKind::Double,
            precision: None,
            scale: None,
        }
    );
    assert_eq!(
        node.properties[1].value_type,
        ValueType::Named {
            name: ident("CHARACTER VARYING"),
            parameters: vec![40]
        }
    );
    assert_eq!(
        node.properties[2].value_type,
        ValueType::Temporal {
            kind: TemporalTypeKind::ZonedDateTime,
        }
    );

    let Statement::Query(cast_query) = &program.statements[1] else {
        panic!("expected query");
    };
    let Expr::Cast { value_type, .. } = &cast_query.body.result_clause.items[0].expr else {
        panic!("expected cast expression");
    };
    assert_eq!(
        value_type,
        &ValueType::Temporal {
            kind: TemporalTypeKind::LocalDateTime,
        }
    );

    let Statement::Query(predicate_query) = &program.statements[2] else {
        panic!("expected query");
    };
    let QueryClause::Match(match_clause) = &predicate_query.body.clauses[0] else {
        panic!("expected match clause");
    };
    let Some(Expr::IsTyped { value_type, .. }) = &match_clause.where_clause else {
        panic!("expected is typed predicate");
    };
    assert_eq!(
        value_type,
        &ValueType::Temporal {
            kind: TemporalTypeKind::ZonedTime,
        }
    );
}

#[test]
fn parses_create_graph_type_source_forms() {
    let program = parse(
        "CREATE OR REPLACE PROPERTY GRAPH TYPE copied AS COPY OF app.base_type; \
         CREATE GRAPH TYPE copied_from_parameter AS COPY OF $base_type; \
         CREATE GRAPH TYPE copied_without_as COPY OF app.base_type; \
         CREATE GRAPH TYPE copied_from_external AS COPY OF 'https://example.com/types/social#g'; \
         CREATE GRAPH TYPE derived LIKE HOME_GRAPH; \
         CREATE GRAPH TYPE nested { NODE Person {name STRING} }; \
         DROP PROPERTY GRAPH TYPE IF EXISTS copied",
    )
    .unwrap();

    let Statement::CreateGraphType(copied) = &program.statements[0] else {
        panic!("expected create graph type");
    };
    assert!(copied.or_replace);
    assert!(copied.property_graph);
    assert_eq!(copied.name, graph_type_name(&["copied"]));
    assert_eq!(
        copied.source,
        CreateGraphTypeSource::CopyOf(graph_type_ref(&["app", "base_type"]))
    );

    let Statement::CreateGraphType(copied_from_parameter) = &program.statements[1] else {
        panic!("expected create graph type");
    };
    assert_eq!(
        copied_from_parameter.source,
        CreateGraphTypeSource::CopyOf(GraphTypeReference::Parameter("base_type".to_owned()))
    );

    let Statement::CreateGraphType(copied_without_as) = &program.statements[2] else {
        panic!("expected create graph type");
    };
    assert_eq!(
        copied_without_as.source,
        CreateGraphTypeSource::CopyOf(graph_type_ref(&["app", "base_type"]))
    );

    let Statement::CreateGraphType(copied_from_external) = &program.statements[3] else {
        panic!("expected create graph type");
    };
    assert_eq!(
        copied_from_external.source,
        CreateGraphTypeSource::CopyOfExternal("https://example.com/types/social#g".to_owned())
    );

    let Statement::CreateGraphType(derived) = &program.statements[4] else {
        panic!("expected create graph type");
    };
    assert_eq!(
        derived.source,
        CreateGraphTypeSource::Like(GraphExpression::HomeGraph)
    );

    let Statement::CreateGraphType(nested) = &program.statements[5] else {
        panic!("expected create graph type");
    };
    let CreateGraphTypeSource::Nested(body) = &nested.source else {
        panic!("expected nested graph type source");
    };
    assert_eq!(body.elements.len(), 1);

    let Statement::DropGraphType(drop_type) = &program.statements[6] else {
        panic!("expected drop graph type");
    };
    assert!(drop_type.property_graph);
}

#[test]
fn parses_create_graph_type_and_source_forms() {
    let program = parse(
        "CREATE OR REPLACE PROPERTY GRAPH social ANY PROPERTY GRAPH AS COPY OF HOME_GRAPH; \
         CREATE GRAPH typed :: TYPED app.social_type; \
         CREATE GRAPH typed_from_parameter :: TYPED $social_type; \
         CREATE GRAPH derived LIKE $template; \
         CREATE GRAPH nested PROPERTY GRAPH { NODE Person {name STRING} }; \
         DROP PROPERTY GRAPH IF EXISTS social",
    )
    .unwrap();

    let Statement::CreateGraph(any_graph) = &program.statements[0] else {
        panic!("expected create graph");
    };
    assert!(any_graph.or_replace);
    assert!(any_graph.property_graph);
    assert_eq!(any_graph.name, graph_name(&["social"]));
    assert_eq!(
        any_graph.graph_type,
        Some(CreateGraphType::Any {
            typed: false,
            property_graph: true
        })
    );
    assert_eq!(any_graph.source, Some(GraphExpression::HomeGraph));

    let Statement::CreateGraph(typed_graph) = &program.statements[1] else {
        panic!("expected create graph");
    };
    assert_eq!(
        typed_graph.graph_type,
        Some(CreateGraphType::Named {
            typed: true,
            name: graph_type_ref(&["app", "social_type"])
        })
    );

    let Statement::CreateGraph(parameter_type_graph) = &program.statements[2] else {
        panic!("expected create graph");
    };
    assert_eq!(
        parameter_type_graph.graph_type,
        Some(CreateGraphType::Named {
            typed: true,
            name: GraphTypeReference::Parameter("social_type".to_owned())
        })
    );

    let Statement::CreateGraph(like_graph) = &program.statements[3] else {
        panic!("expected create graph");
    };
    assert_eq!(
        like_graph.graph_type,
        Some(CreateGraphType::Like(GraphExpression::Parameter(
            "template".to_owned()
        )))
    );

    let Statement::CreateGraph(nested_graph) = &program.statements[4] else {
        panic!("expected create graph");
    };
    let Some(CreateGraphType::Nested {
        typed: false,
        property_graph: true,
        body,
    }) = &nested_graph.graph_type
    else {
        panic!("expected nested graph type");
    };
    assert_eq!(body.elements.len(), 1);

    let Statement::DropGraph(drop_graph) = &program.statements[5] else {
        panic!("expected drop graph");
    };
    assert!(drop_graph.property_graph);
}

#[test]
fn parses_transaction_characteristics() {
    let program = parse(
        "START TRANSACTION; \
         START TRANSACTION READ ONLY; \
         START TRANSACTION READ WRITE;",
    )
    .unwrap();

    let Statement::StartTransaction(first) = &program.statements[0] else {
        panic!("expected start transaction");
    };
    assert_eq!(first.access_mode, None);
    assert_eq!(first.isolation_level, None);

    let Statement::StartTransaction(second) = &program.statements[1] else {
        panic!("expected start transaction");
    };
    assert_eq!(second.access_mode, Some(TransactionAccessMode::ReadOnly));
    assert_eq!(second.isolation_level, None);

    let Statement::StartTransaction(third) = &program.statements[2] else {
        panic!("expected start transaction");
    };
    assert_eq!(third.access_mode, Some(TransactionAccessMode::ReadWrite));
    assert_eq!(third.isolation_level, None);
}

#[test]
fn parses_transaction_activity_without_semicolon_separators() {
    let program = parse(
        "START TRANSACTION RETURN 1 AS ok COMMIT; \
         START TRANSACTION CREATE GRAPH demo.temp ANY GRAPH ROLLBACK",
    )
    .unwrap();

    assert_eq!(program.statements.len(), 6);
    assert!(matches!(
        program.statements[0],
        Statement::StartTransaction(_)
    ));
    assert!(matches!(program.statements[1], Statement::Query(_)));
    assert!(matches!(program.statements[2], Statement::Commit(_)));
    assert!(matches!(
        program.statements[3],
        Statement::StartTransaction(_)
    ));
    assert!(matches!(program.statements[4], Statement::CreateGraph(_)));
    assert!(matches!(program.statements[5], Statement::Rollback(_)));
}

#[test]
fn parses_program_activity_with_session_close_without_semicolon() {
    let transaction_program =
        parse("START TRANSACTION RETURN 1 AS ok COMMIT SESSION CLOSE").unwrap();
    assert_eq!(transaction_program.statements.len(), 4);
    assert!(matches!(
        transaction_program.statements[0],
        Statement::StartTransaction(_)
    ));
    assert!(matches!(
        transaction_program.statements[1],
        Statement::Query(_)
    ));
    assert!(matches!(
        transaction_program.statements[2],
        Statement::Commit(_)
    ));
    assert!(matches!(
        transaction_program.statements[3],
        Statement::SessionClose(_)
    ));

    let session_program =
        parse("SESSION SET SCHEMA app.main SESSION RESET SCHEMA SESSION CLOSE").unwrap();
    assert_eq!(session_program.statements.len(), 3);
    assert!(matches!(
        session_program.statements[0],
        Statement::SessionSet(_)
    ));
    assert!(matches!(
        session_program.statements[1],
        Statement::SessionReset(_)
    ));
    assert!(matches!(
        session_program.statements[2],
        Statement::SessionClose(_)
    ));
}

#[test]
fn parses_session_commands() {
    let program = parse(
        "SESSION SET SCHEMA app.main; \
         SESSION SET GRAPH social; \
         SESSION RESET SCHEMA; \
         SESSION RESET GRAPH; \
         SESSION RESET ALL CHARACTERISTICS; \
         SESSION CLOSE;",
    )
    .unwrap();

    assert_eq!(program.statements.len(), 6);
    let Statement::SessionSet(set_schema) = &program.statements[0] else {
        panic!("expected session set");
    };
    assert!(matches!(set_schema.target, SessionSetTarget::Schema(_)));
    let Statement::SessionSet(set_graph) = &program.statements[1] else {
        panic!("expected session set");
    };
    assert!(matches!(set_graph.target, SessionSetTarget::Graph(_)));
    let Statement::SessionReset(reset_schema) = &program.statements[2] else {
        panic!("expected session reset");
    };
    assert_eq!(reset_schema.target, SessionResetTarget::Schema);
    assert!(matches!(program.statements[5], Statement::SessionClose(_)));
}

#[test]
fn parses_standard_session_commands() {
    let program = parse(
        "SESSION SET SCHEMA CURRENT_SCHEMA; \
         SESSION SET TIME ZONE 'UTC'; \
         SESSION SET TIME ZONE \"Z\"; \
         SESSION SET PROPERTY GRAPH HOME_PROPERTY_GRAPH; \
         SESSION RESET; \
         SESSION RESET ALL PARAMETERS; \
         SESSION RESET CHARACTERISTICS; \
         SESSION RESET PROPERTY GRAPH; \
         SESSION RESET TIME ZONE; \
         SESSION RESET PARAMETER $limit; \
         SESSION RESET $flag; \
         SESSION CLOSE",
    )
    .unwrap();

    let Statement::SessionSet(set_schema) = &program.statements[0] else {
        panic!("expected session set");
    };
    assert_eq!(
        set_schema.target,
        SessionSetTarget::Schema(SchemaReference::CurrentSchema)
    );

    let Statement::SessionSet(set_time_zone) = &program.statements[1] else {
        panic!("expected session set");
    };
    assert_eq!(
        set_time_zone.target,
        SessionSetTarget::TimeZone(Expr::Literal(Literal::String("UTC".to_owned())))
    );

    let Statement::SessionSet(set_quoted_time_zone) = &program.statements[2] else {
        panic!("expected session set");
    };
    assert_eq!(
        set_quoted_time_zone.target,
        SessionSetTarget::TimeZone(Expr::Literal(Literal::String("Z".to_owned())))
    );

    let Statement::SessionSet(set_graph) = &program.statements[3] else {
        panic!("expected session set");
    };
    assert_eq!(
        set_graph.target,
        SessionSetTarget::Graph(GraphExpression::HomePropertyGraph)
    );

    let Statement::SessionReset(reset_default) = &program.statements[4] else {
        panic!("expected session reset");
    };
    assert_eq!(reset_default.target, SessionResetTarget::AllCharacteristics);

    let Statement::SessionReset(reset_parameters) = &program.statements[5] else {
        panic!("expected session reset");
    };
    assert_eq!(reset_parameters.target, SessionResetTarget::AllParameters);

    let Statement::SessionReset(reset_characteristics) = &program.statements[6] else {
        panic!("expected session reset");
    };
    assert_eq!(
        reset_characteristics.target,
        SessionResetTarget::AllCharacteristics
    );

    let Statement::SessionReset(reset_graph) = &program.statements[7] else {
        panic!("expected session reset");
    };
    assert_eq!(reset_graph.target, SessionResetTarget::Graph);

    let Statement::SessionReset(reset_time_zone) = &program.statements[8] else {
        panic!("expected session reset");
    };
    assert_eq!(reset_time_zone.target, SessionResetTarget::TimeZone);

    let Statement::SessionReset(reset_parameter_keyword) = &program.statements[9] else {
        panic!("expected session reset");
    };
    assert_eq!(
        reset_parameter_keyword.target,
        SessionResetTarget::Parameter("limit".to_owned())
    );

    let Statement::SessionReset(reset_parameter) = &program.statements[10] else {
        panic!("expected session reset");
    };
    assert_eq!(
        reset_parameter.target,
        SessionResetTarget::Parameter("flag".to_owned())
    );

    assert!(matches!(program.statements[11], Statement::SessionClose(_)));
}

#[test]
fn parses_session_value_parameter_commands() {
    let program = parse(
        "SESSION SET VALUE IF NOT EXISTS $limit :: INTEGER = 10; \
         SESSION SET VALUE timezone = 'UTC'; \
         SESSION SET VALUE $flag = true",
    )
    .unwrap();

    let Statement::SessionSet(limit) = &program.statements[0] else {
        panic!("expected session set");
    };
    let SessionSetTarget::ValueParameter(limit) = &limit.target else {
        panic!("expected value parameter");
    };
    assert!(limit.if_not_exists);
    assert_eq!(limit.name, "limit");
    assert!(limit.typed);
    assert!(matches!(
        limit.value_type,
        Some(ValueType::ExactNumeric {
            kind: ExactNumericTypeKind::Integer,
            signed: Some(true),
            precision: None,
            scale: None,
        })
    ));
    assert_eq!(limit.initializer, Expr::Literal(Literal::Integer(10)));

    let Statement::SessionSet(timezone) = &program.statements[1] else {
        panic!("expected session set");
    };
    let SessionSetTarget::ValueParameter(timezone) = &timezone.target else {
        panic!("expected value parameter");
    };
    assert!(!timezone.if_not_exists);
    assert_eq!(timezone.name, "timezone");
    assert!(timezone.value_type.is_none());
    assert_eq!(
        timezone.initializer,
        Expr::Literal(Literal::String("UTC".to_owned()))
    );

    let Statement::SessionSet(flag) = &program.statements[2] else {
        panic!("expected session set");
    };
    let SessionSetTarget::ValueParameter(flag) = &flag.target else {
        panic!("expected value parameter");
    };
    assert_eq!(flag.name, "flag");
    assert_eq!(flag.initializer, Expr::Literal(Literal::Boolean(true)));
}

#[test]
fn parses_session_graph_and_binding_table_parameter_commands() {
    let program = parse(
        "SESSION SET GRAPH $active = HOME_GRAPH; \
         SESSION SET PROPERTY GRAPH IF NOT EXISTS $pg :: ANY PROPERTY GRAPH = HOME_PROPERTY_GRAPH; \
         SESSION SET BINDING TABLE $rows :: BINDING TABLE {id INTEGER} = app.rows; \
         SESSION SET TABLE $derived = { MATCH (n) RETURN n }",
    )
    .unwrap();

    let Statement::SessionSet(graph) = &program.statements[0] else {
        panic!("expected session set");
    };
    let SessionSetTarget::GraphParameter(graph) = &graph.target else {
        panic!("expected graph parameter");
    };
    assert!(!graph.property_graph);
    assert!(!graph.if_not_exists);
    assert_eq!(graph.name, "active");
    assert!(!graph.typed);
    assert!(graph.value_type.is_none());
    assert_eq!(graph.initializer, GraphExpression::HomeGraph);

    let Statement::SessionSet(property_graph) = &program.statements[1] else {
        panic!("expected session set");
    };
    let SessionSetTarget::GraphParameter(property_graph) = &property_graph.target else {
        panic!("expected property graph parameter");
    };
    assert!(property_graph.property_graph);
    assert!(property_graph.if_not_exists);
    assert_eq!(property_graph.name, "pg");
    assert!(property_graph.typed);
    assert_eq!(
        property_graph.value_type,
        Some(ValueType::GraphReference {
            property_graph: true,
            body: None,
        })
    );
    assert_eq!(
        property_graph.initializer,
        GraphExpression::HomePropertyGraph
    );

    let Statement::SessionSet(rows) = &program.statements[2] else {
        panic!("expected session set");
    };
    let SessionSetTarget::BindingTableParameter(rows) = &rows.target else {
        panic!("expected binding table parameter");
    };
    assert!(rows.binding);
    assert_eq!(rows.name, "rows");
    assert!(matches!(
        rows.value_type,
        Some(ValueType::BindingTable {
            binding: true,
            ref fields,
        }) if fields.len() == 1
    ));
    assert_eq!(
        rows.initializer,
        BindingTableExpression::Name(binding_table_name(&["app", "rows"]))
    );

    let Statement::SessionSet(derived) = &program.statements[3] else {
        panic!("expected session set");
    };
    let SessionSetTarget::BindingTableParameter(derived) = &derived.target else {
        panic!("expected binding table parameter");
    };
    assert!(!derived.binding);
    assert_eq!(derived.name, "derived");
    let BindingTableExpression::NestedQuery(nested) = &derived.initializer else {
        panic!("expected nested binding table query");
    };
    assert_eq!(
        nested.body.result_clause.items[0].expr,
        Expr::Identifier(ident("n"))
    );
}

#[test]
fn parses_session_graph_parameter_before_procedure_body_boundaries() {
    let program = parse(
        "CALL { SESSION SET GRAPH $active = HOME_GRAPH }; \
         CALL { SESSION SET GRAPH $next = HOME_GRAPH NEXT RETURN 1 AS ok }",
    )
    .unwrap();

    let Statement::Call(first_call) = &program.statements[0] else {
        panic!("expected call statement");
    };
    let ProcedureCall::Inline(first_inline) = &first_call.call.call else {
        panic!("expected inline procedure call");
    };
    let Statement::SessionSet(first_set) = &first_inline.body.statements[0] else {
        panic!("expected session set");
    };
    let SessionSetTarget::GraphParameter(first_graph) = &first_set.target else {
        panic!("expected graph parameter");
    };
    assert_eq!(first_graph.name, "active");
    assert_eq!(first_graph.initializer, GraphExpression::HomeGraph);

    let Statement::Call(second_call) = &program.statements[1] else {
        panic!("expected call statement");
    };
    let ProcedureCall::Inline(second_inline) = &second_call.call.call else {
        panic!("expected inline procedure call");
    };
    let Statement::SessionSet(second_set) = &second_inline.body.statements[0] else {
        panic!("expected session set");
    };
    let SessionSetTarget::GraphParameter(second_graph) = &second_set.target else {
        panic!("expected graph parameter");
    };
    assert_eq!(second_graph.name, "next");
    assert!(matches!(
        second_inline.body.statements[1],
        Statement::Next(_)
    ));
}

#[test]
fn parses_inline_procedure_empty_variable_scope() {
    let program = parse("CALL () { RETURN 1 AS value }").unwrap();

    let Statement::Call(call) = &program.statements[0] else {
        panic!("expected call statement");
    };
    let ProcedureCall::Inline(inline) = &call.call.call else {
        panic!("expected inline procedure call");
    };
    assert_eq!(inline.variable_scope, Some(Vec::new()));
    assert_eq!(inline.body.statements.len(), 1);
}

#[test]
fn parses_at_schema_query_context() {
    let program = parse("AT SCHEMA app.main USE GRAPH social MATCH (n) RETURN n").unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };

    assert_eq!(
        query.at_schema.as_ref().unwrap(),
        &SchemaReference::Name(schema_name(&["app", "main"]))
    );
    assert_eq!(
        query.use_graph.as_ref().unwrap(),
        &GraphExpression::Name(graph_name(&["social"]))
    );
}

#[test]
fn parses_schema_references() {
    let program = parse(
        "AT SCHEMA CURRENT_SCHEMA MATCH (n) RETURN n; \
         AT SCHEMA $tenant MATCH (n) RETURN n; \
         SESSION SET SCHEMA HOME_SCHEMA; \
         SESSION SET SCHEMA $next; \
         AT SCHEMA . MATCH (n) RETURN n; \
         AT SCHEMA / MATCH (n) RETURN n; \
         AT SCHEMA /catalog/main MATCH (n) RETURN n; \
         AT SCHEMA ../main MATCH (n) RETURN n; \
         AT SCHEMA ../../tenant/main MATCH (n) RETURN n",
    )
    .unwrap();

    let Statement::Query(current_query) = &program.statements[0] else {
        panic!("expected query");
    };
    assert_eq!(
        current_query.at_schema,
        Some(SchemaReference::CurrentSchema)
    );

    let Statement::Query(parameter_query) = &program.statements[1] else {
        panic!("expected query");
    };
    assert_eq!(
        parameter_query.at_schema,
        Some(SchemaReference::Parameter("tenant".to_owned()))
    );

    let Statement::SessionSet(home_schema) = &program.statements[2] else {
        panic!("expected session set");
    };
    assert_eq!(
        home_schema.target,
        SessionSetTarget::Schema(SchemaReference::HomeSchema)
    );

    let Statement::SessionSet(parameter_schema) = &program.statements[3] else {
        panic!("expected session set");
    };
    assert_eq!(
        parameter_schema.target,
        SessionSetTarget::Schema(SchemaReference::Parameter("next".to_owned()))
    );

    let Statement::Query(period_query) = &program.statements[4] else {
        panic!("expected query");
    };
    assert_eq!(period_query.at_schema, Some(SchemaReference::CurrentSchema));

    let Statement::Query(root_query) = &program.statements[5] else {
        panic!("expected query");
    };
    assert_eq!(root_query.at_schema, Some(SchemaReference::Root));

    let Statement::Query(absolute_query) = &program.statements[6] else {
        panic!("expected query");
    };
    assert_eq!(
        absolute_query.at_schema,
        Some(SchemaReference::Absolute(schema_name(&["catalog", "main"])))
    );

    let Statement::Query(parent_query) = &program.statements[7] else {
        panic!("expected query");
    };
    assert_eq!(
        parent_query.at_schema,
        Some(SchemaReference::Parent {
            levels: 1,
            name: schema_name(&["main"])
        })
    );

    let Statement::Query(grandparent_query) = &program.statements[8] else {
        panic!("expected query");
    };
    assert_eq!(
        grandparent_query.at_schema,
        Some(SchemaReference::Parent {
            levels: 2,
            name: schema_name(&["tenant", "main"])
        })
    );
}

#[test]
fn parses_solidus_separated_catalog_references() {
    let program = parse(
        "AT SCHEMA app/main USE GRAPH app/social MATCH (n) RETURN n; \
         RETURN BINDING TABLE app/rows AS rows; \
         CALL db/refresh('social'); \
         CREATE GRAPH app/social :: TYPED app/social_type; \
         DROP GRAPH TYPE IF EXISTS app/social_type",
    )
    .unwrap();

    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };
    assert_eq!(
        query.at_schema,
        Some(SchemaReference::Name(schema_name(&["app", "main"])))
    );
    assert_eq!(
        query.use_graph,
        Some(GraphExpression::Name(graph_name(&["app", "social"])))
    );

    let Statement::Query(table_query) = &program.statements[1] else {
        panic!("expected query");
    };
    assert_eq!(
        table_query.body.result_clause.items[0].expr,
        Expr::BindingTableReference {
            binding: true,
            table: BindingTableExpression::Name(binding_table_name(&["app", "rows"]))
        }
    );

    let Statement::Call(call) = &program.statements[2] else {
        panic!("expected call");
    };
    let ProcedureCall::Named { procedure, .. } = &call.call.call else {
        panic!("expected named procedure");
    };
    assert_eq!(procedure, &procedure_ref(&["db", "refresh"]));

    let Statement::CreateGraph(create_graph) = &program.statements[3] else {
        panic!("expected create graph");
    };
    assert_eq!(create_graph.name, graph_name(&["app", "social"]));
    assert_eq!(
        create_graph.graph_type,
        Some(CreateGraphType::Named {
            typed: true,
            name: graph_type_ref(&["app", "social_type"])
        })
    );

    let Statement::DropGraphType(drop_type) = &program.statements[4] else {
        panic!("expected drop graph type");
    };
    assert_eq!(drop_type.name, graph_type_name(&["app", "social_type"]));
}

#[test]
fn parses_absolute_catalog_object_references() {
    let program = parse(
        "USE GRAPH /app/social MATCH (n) RETURN n; \
         RETURN BINDING TABLE /app/rows AS rows; \
         CALL /db/refresh('social'); \
         CREATE GRAPH /app/typed :: TYPED /app/social_type; \
         CREATE GRAPH TYPE copied AS COPY OF /app/base_type",
    )
    .unwrap();

    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };
    assert_eq!(
        query.use_graph,
        Some(GraphExpression::Name(absolute_graph_name(&[
            "app", "social"
        ])))
    );

    let Statement::Query(table_query) = &program.statements[1] else {
        panic!("expected query");
    };
    assert_eq!(
        table_query.body.result_clause.items[0].expr,
        Expr::BindingTableReference {
            binding: true,
            table: BindingTableExpression::Name(absolute_binding_table_name(&["app", "rows"]))
        }
    );

    let Statement::Call(call) = &program.statements[2] else {
        panic!("expected call");
    };
    let ProcedureCall::Named { procedure, .. } = &call.call.call else {
        panic!("expected named procedure");
    };
    assert_eq!(procedure, &absolute_procedure_ref(&["db", "refresh"]));

    let Statement::CreateGraph(create_graph) = &program.statements[3] else {
        panic!("expected create graph");
    };
    assert_eq!(create_graph.name, absolute_graph_name(&["app", "typed"]));
    assert_eq!(
        create_graph.graph_type,
        Some(CreateGraphType::Named {
            typed: true,
            name: GraphTypeReference::Name(absolute_graph_type_name(&["app", "social_type"]))
        })
    );

    let Statement::CreateGraphType(create_type) = &program.statements[4] else {
        panic!("expected create graph type");
    };
    assert_eq!(
        create_type.source,
        CreateGraphTypeSource::CopyOf(GraphTypeReference::Name(absolute_graph_type_name(&[
            "app",
            "base_type"
        ])))
    );
}

#[test]
fn parses_current_schema_catalog_object_references() {
    let program = parse(
        "USE GRAPH ./social MATCH (n) RETURN n; \
         RETURN BINDING TABLE ./rows AS rows; \
         CALL ./refresh('social'); \
         CREATE GRAPH ./typed :: TYPED ./social_type; \
         CREATE GRAPH TYPE copied AS COPY OF ./base_type",
    )
    .unwrap();

    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };
    assert_eq!(
        query.use_graph,
        Some(GraphExpression::Name(current_schema_graph_name(&[
            "social"
        ])))
    );

    let Statement::Query(table_query) = &program.statements[1] else {
        panic!("expected query");
    };
    assert_eq!(
        table_query.body.result_clause.items[0].expr,
        Expr::BindingTableReference {
            binding: true,
            table: BindingTableExpression::Name(current_schema_binding_table_name(&["rows"]))
        }
    );

    let Statement::Call(call) = &program.statements[2] else {
        panic!("expected call");
    };
    let ProcedureCall::Named { procedure, .. } = &call.call.call else {
        panic!("expected named procedure");
    };
    assert_eq!(procedure, &current_schema_procedure_ref(&["refresh"]));

    let Statement::CreateGraph(create_graph) = &program.statements[3] else {
        panic!("expected create graph");
    };
    assert_eq!(create_graph.name, current_schema_graph_name(&["typed"]));
    assert_eq!(
        create_graph.graph_type,
        Some(CreateGraphType::Named {
            typed: true,
            name: GraphTypeReference::Name(current_schema_graph_type_name(&["social_type"]))
        })
    );

    let Statement::CreateGraphType(create_type) = &program.statements[4] else {
        panic!("expected create graph type");
    };
    assert_eq!(
        create_type.source,
        CreateGraphTypeSource::CopyOf(GraphTypeReference::Name(current_schema_graph_type_name(&[
            "base_type"
        ])))
    );
}

#[test]
fn parses_schema_reference_catalog_object_parents() {
    let program = parse(
        "USE GRAPH CURRENT_SCHEMA/social MATCH (n) RETURN n; \
         USE GRAPH HOME_SCHEMA/home_social MATCH (h) RETURN h; \
         RETURN BINDING TABLE HOME_SCHEMA/rows AS rows; \
         CALL HOME_SCHEMA/refresh('social'); \
         CREATE GRAPH CURRENT_SCHEMA/typed :: TYPED HOME_SCHEMA/social_type; \
         CREATE GRAPH TYPE copied AS COPY OF HOME_SCHEMA/base_type",
    )
    .unwrap();

    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };
    assert_eq!(
        query.use_graph,
        Some(GraphExpression::Name(current_schema_graph_name(&[
            "social"
        ])))
    );

    let Statement::Query(home_query) = &program.statements[1] else {
        panic!("expected query");
    };
    assert_eq!(
        home_query.use_graph,
        Some(GraphExpression::Name(home_schema_graph_name(&[
            "home_social"
        ])))
    );

    let Statement::Query(table_query) = &program.statements[2] else {
        panic!("expected query");
    };
    assert_eq!(
        table_query.body.result_clause.items[0].expr,
        Expr::BindingTableReference {
            binding: true,
            table: BindingTableExpression::Name(home_schema_binding_table_name(&["rows"]))
        }
    );

    let Statement::Call(call) = &program.statements[3] else {
        panic!("expected call");
    };
    let ProcedureCall::Named { procedure, .. } = &call.call.call else {
        panic!("expected named procedure");
    };
    assert_eq!(procedure, &home_schema_procedure_ref(&["refresh"]));

    let Statement::CreateGraph(create_graph) = &program.statements[4] else {
        panic!("expected create graph");
    };
    assert_eq!(create_graph.name, current_schema_graph_name(&["typed"]));
    assert_eq!(
        create_graph.graph_type,
        Some(CreateGraphType::Named {
            typed: true,
            name: GraphTypeReference::Name(home_schema_graph_type_name(&["social_type"]))
        })
    );

    let Statement::CreateGraphType(create_type) = &program.statements[5] else {
        panic!("expected create graph type");
    };
    assert_eq!(
        create_type.source,
        CreateGraphTypeSource::CopyOf(GraphTypeReference::Name(home_schema_graph_type_name(&[
            "base_type"
        ])))
    );
}

#[test]
fn parses_parent_relative_catalog_object_references() {
    let program = parse(
        "USE GRAPH ../main/social MATCH (n) RETURN n; \
         RETURN BINDING TABLE ../../tenant/main/rows AS rows; \
         CALL ../main/refresh('social'); \
         CREATE GRAPH ../main/typed :: TYPED ../../tenant/main/social_type; \
         CREATE GRAPH TYPE copied AS COPY OF ../main/base_type",
    )
    .unwrap();

    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };
    assert_eq!(
        query.use_graph,
        Some(GraphExpression::Name(parent_graph_name(
            1,
            &["main", "social"]
        )))
    );

    let Statement::Query(table_query) = &program.statements[1] else {
        panic!("expected query");
    };
    assert_eq!(
        table_query.body.result_clause.items[0].expr,
        Expr::BindingTableReference {
            binding: true,
            table: BindingTableExpression::Name(parent_binding_table_name(
                2,
                &["tenant", "main", "rows"]
            ))
        }
    );

    let Statement::Call(call) = &program.statements[2] else {
        panic!("expected call");
    };
    let ProcedureCall::Named { procedure, .. } = &call.call.call else {
        panic!("expected named procedure");
    };
    assert_eq!(procedure, &parent_procedure_ref(1, &["main", "refresh"]));

    let Statement::CreateGraph(create_graph) = &program.statements[3] else {
        panic!("expected create graph");
    };
    assert_eq!(create_graph.name, parent_graph_name(1, &["main", "typed"]));
    assert_eq!(
        create_graph.graph_type,
        Some(CreateGraphType::Named {
            typed: true,
            name: GraphTypeReference::Name(parent_graph_type_name(
                2,
                &["tenant", "main", "social_type"]
            ))
        })
    );

    let Statement::CreateGraphType(create_type) = &program.statements[4] else {
        panic!("expected create graph type");
    };
    assert_eq!(
        create_type.source,
        CreateGraphTypeSource::CopyOf(GraphTypeReference::Name(parent_graph_type_name(
            1,
            &["main", "base_type"]
        )))
    );
}

#[test]
fn parses_standalone_call_statement() {
    let program =
        parse("CALL db.refresh('social'); CALL $refresh('social'); OPTIONAL CALL db.refresh()")
            .unwrap();
    let Statement::Call(call) = &program.statements[0] else {
        panic!("expected standalone call");
    };

    let ProcedureCall::Named { procedure, args } = &call.call.call else {
        panic!("expected named procedure call");
    };
    assert_eq!(procedure, &procedure_ref(&["db", "refresh"]));
    assert_eq!(
        args,
        &vec![Expr::Literal(Literal::String("social".to_owned()))]
    );

    let Statement::Call(parameter_call) = &program.statements[1] else {
        panic!("expected standalone call");
    };
    let ProcedureCall::Named { procedure, args } = &parameter_call.call.call else {
        panic!("expected named procedure call");
    };
    assert_eq!(
        procedure,
        &ProcedureReference::Parameter("refresh".to_owned())
    );
    assert_eq!(
        args,
        &vec![Expr::Literal(Literal::String("social".to_owned()))]
    );

    let Statement::Call(optional_call) = &program.statements[2] else {
        panic!("expected optional standalone call");
    };
    assert!(optional_call.call.optional);
    assert!(matches!(
        optional_call.call.call,
        ProcedureCall::Named { .. }
    ));
}

#[test]
fn parses_standalone_call_before_procedure_body_boundaries() {
    let program = parse(
        "CALL { CALL db.refresh() }; \
         CALL { CALL db.refresh() NEXT INSERT (:Seen) NEXT RETURN 1 AS ok }",
    )
    .unwrap();

    let Statement::Call(first_call) = &program.statements[0] else {
        panic!("expected call statement");
    };
    let ProcedureCall::Inline(first_inline) = &first_call.call.call else {
        panic!("expected inline procedure call");
    };
    assert!(matches!(
        first_inline.body.statements[0],
        Statement::Call(_)
    ));

    let Statement::Call(second_call) = &program.statements[1] else {
        panic!("expected call statement");
    };
    let ProcedureCall::Inline(second_inline) = &second_call.call.call else {
        panic!("expected inline procedure call");
    };
    assert!(matches!(
        second_inline.body.statements[0],
        Statement::Call(_)
    ));
    let Statement::Next(first_next) = &second_inline.body.statements[1] else {
        panic!("expected next statement");
    };
    assert!(matches!(
        first_next.statement.as_ref(),
        Statement::Insert(_)
    ));
    let Statement::Next(second_next) = &second_inline.body.statements[2] else {
        panic!("expected next statement");
    };
    assert!(matches!(
        second_next.statement.as_ref(),
        Statement::Query(_)
    ));
}

#[test]
fn parses_inline_and_optional_procedure_calls() {
    let program = parse(
        "MATCH (n) \
         CALL (n) { VALUE fallback STRING = 'unknown' MATCH (n)-[:KNOWS]->(m) RETURN m } \
         OPTIONAL CALL graph.expand(m) YIELD node AS friend \
         OPTIONAL CALL $\"enrich proc\"(friend) YIELD node AS enriched \
         RETURN friend",
    )
    .unwrap();

    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query statement");
    };
    assert_eq!(query.body.clauses.len(), 4);

    let QueryClause::Call(inline_call) = &query.body.clauses[1] else {
        panic!("expected inline call clause");
    };
    assert!(!inline_call.optional);
    let ProcedureCall::Inline(inline) = &inline_call.call else {
        panic!("expected inline procedure call");
    };
    assert_eq!(inline.variable_scope.as_ref().unwrap(), &vec![ident("n")]);
    assert_eq!(inline.body.definitions.len(), 1);
    assert_eq!(inline.body.statements.len(), 1);
    let Statement::Query(inline_query) = &inline.body.statements[0] else {
        panic!("expected inline query body");
    };
    assert_eq!(
        inline_query.body.result_clause.items[0].expr,
        Expr::Identifier(ident("m"))
    );

    let QueryClause::Call(optional_call) = &query.body.clauses[2] else {
        panic!("expected optional named call clause");
    };
    assert!(optional_call.optional);
    let ProcedureCall::Named { procedure, args } = &optional_call.call else {
        panic!("expected named procedure call");
    };
    assert_eq!(procedure, &procedure_ref(&["graph", "expand"]));
    assert_eq!(args, &vec![Expr::Identifier(ident("m"))]);
    assert!(matches!(
        optional_call.yield_clause.as_ref().unwrap().items[0],
        YieldItem::Item {
            ref name,
            alias: Some(_)
        } if name == &ident("node")
    ));

    let QueryClause::Call(parameter_call) = &query.body.clauses[3] else {
        panic!("expected parameter call clause");
    };
    assert!(parameter_call.optional);
    let ProcedureCall::Named { procedure, args } = &parameter_call.call else {
        panic!("expected named procedure call");
    };
    assert_eq!(
        procedure,
        &ProcedureReference::Parameter("enrich proc".to_owned())
    );
    assert_eq!(args, &vec![Expr::Identifier(ident("friend"))]);
}

#[test]
fn parses_label_expressions_and_path_quantifiers() {
    let program = parse(
        "MATCH (n IS Person|Employee)-[r IS KNOWS]->{1,3}(m IS !Archived), \
         (wild:%)-[any IS !%]->(m IS %|Active), \
         (entry:Entry)+-[step]->(exit){2} \
         RETURN m",
    )
    .unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };
    let match_clause = first_match(query);
    let pattern = &match_clause.patterns[0];

    assert!(matches!(
        pattern.start.label_expression,
        Some(LabelExpression::Or(_, _))
    ));
    assert_eq!(
        pattern.chains[0].relationship.quantifier,
        Some(PathPatternQuantifier::Range {
            min: Some(1),
            max: Some(3)
        })
    );
    assert!(matches!(
        pattern.chains[0].relationship.label_expression,
        Some(LabelExpression::Label(_))
    ));
    assert!(matches!(
        pattern.chains[0].node.label_expression,
        Some(LabelExpression::Not(_))
    ));

    let wildcard_pattern = &match_clause.patterns[1];
    assert!(wildcard_pattern.start.labels.is_empty());
    assert_eq!(
        wildcard_pattern.start.label_expression,
        Some(LabelExpression::Wildcard)
    );
    assert!(wildcard_pattern.chains[0].relationship.labels.is_empty());
    assert!(matches!(
        wildcard_pattern.chains[0].relationship.label_expression,
        Some(LabelExpression::Not(_))
    ));
    assert!(matches!(
        wildcard_pattern.chains[0].node.label_expression,
        Some(LabelExpression::Or(_, _))
    ));

    let quantified_node_pattern = &match_clause.patterns[2];
    assert_eq!(
        quantified_node_pattern.start.quantifier,
        Some(PathPatternQuantifier::OneOrMore)
    );
    assert_eq!(
        quantified_node_pattern.chains[0].node.quantifier,
        Some(PathPatternQuantifier::Fixed(2))
    );
}

#[test]
fn parses_path_variable_declarations() {
    let program =
        parse("MATCH path = (a)-[:KNOWS]->(b), other = (b)-[:KNOWS]->(c) RETURN path, other")
            .unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };
    let match_clause = first_match(query);

    assert_eq!(match_clause.patterns.len(), 2);
    assert_eq!(match_clause.patterns[0].variable, Some(ident("path")));
    assert_eq!(match_clause.patterns[1].variable, Some(ident("other")));
    assert_eq!(match_clause.patterns[0].chains.len(), 1);
    assert_eq!(match_clause.patterns[1].chains.len(), 1);
}

#[test]
fn parses_parenthesized_path_pattern_expressions() {
    let program = parse(
        "MATCH path = (sub = TRAIL PATHS (a)-[:KNOWS]->(b) WHERE a.active){1,3}, \
         maybe = ((b)-[:LIKES]->(c))? \
         RETURN path, maybe",
    )
    .unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };
    let patterns = &first_match(query).patterns;

    assert_eq!(patterns[0].variable, Some(ident("path")));
    let parenthesized = patterns[0]
        .parenthesized
        .as_ref()
        .expect("expected parenthesized path pattern expression");
    assert_eq!(parenthesized.variable, Some(ident("sub")));
    assert_eq!(
        parenthesized.prefix,
        Some(PathPatternPrefix::Mode {
            mode: PathMode::Trail,
            path_or_paths: Some(PathOrPaths::Paths)
        })
    );
    assert!(parenthesized.where_clause.is_some());
    assert_eq!(
        parenthesized.quantifier,
        Some(PathPatternQuantifier::Range {
            min: Some(1),
            max: Some(3)
        })
    );
    assert!(!parenthesized.questioned);
    assert_eq!(parenthesized.pattern.start.variable, Some(ident("a")));
    assert_eq!(parenthesized.pattern.chains.len(), 1);

    let questioned = patterns[1]
        .parenthesized
        .as_ref()
        .expect("expected questioned path pattern expression");
    assert!(questioned.questioned);
    assert_eq!(questioned.quantifier, None);
    assert_eq!(questioned.pattern.start.variable, Some(ident("b")));
}

#[test]
fn parses_parenthesized_path_pattern_factor_in_concatenation() {
    let program = parse("MATCH (a)-[:KNOWS]->((b)-[:LIKES]->(c)){1,2} RETURN a").unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };
    let pattern = &first_match(query).patterns[0];

    assert_eq!(pattern.start.variable, Some(ident("a")));
    assert_eq!(pattern.chains.len(), 1);
    assert_eq!(pattern.chains[0].relationship.labels, vec![ident("KNOWS")]);
    assert_eq!(pattern.chains[0].node.variable, Some(ident("b")));
    assert_eq!(pattern.factors.len(), 3);

    let PathPatternFactor::Parenthesized(parenthesized) = &pattern.factors[2] else {
        panic!("expected parenthesized path factor");
    };
    assert_eq!(
        parenthesized.quantifier,
        Some(PathPatternQuantifier::Range {
            min: Some(1),
            max: Some(2)
        })
    );
    assert_eq!(parenthesized.pattern.start.variable, Some(ident("b")));
    assert_eq!(
        parenthesized.pattern.chains[0].node.variable,
        Some(ident("c"))
    );
}

#[test]
fn parses_path_mode_prefixes() {
    let program = parse(
        "MATCH WALK (a)-[:KNOWS]->(b), \
         trail_path = TRAIL PATHS (b)-[:KNOWS]->(c), \
         simple_path = SIMPLE PATH (c)-[:KNOWS]->(d), \
         ACYCLIC (d)-[:KNOWS]->(e) \
         RETURN a",
    )
    .unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };
    let patterns = &first_match(query).patterns;

    assert_eq!(
        patterns[0].prefix,
        Some(PathPatternPrefix::Mode {
            mode: PathMode::Walk,
            path_or_paths: None
        })
    );
    assert_eq!(patterns[1].variable, Some(ident("trail_path")));
    assert_eq!(
        patterns[1].prefix,
        Some(PathPatternPrefix::Mode {
            mode: PathMode::Trail,
            path_or_paths: Some(PathOrPaths::Paths)
        })
    );
    assert_eq!(
        patterns[2].prefix,
        Some(PathPatternPrefix::Mode {
            mode: PathMode::Simple,
            path_or_paths: Some(PathOrPaths::Path)
        })
    );
    assert_eq!(
        patterns[3].prefix,
        Some(PathPatternPrefix::Mode {
            mode: PathMode::Acyclic,
            path_or_paths: None
        })
    );
}

#[test]
fn parses_path_search_prefixes() {
    let program = parse(
        "MATCH ALL WALK (a)-[:KNOWS]->(b), \
         ANY TRAIL PATH (b)-[:KNOWS]->(c), \
         ANY 2 SIMPLE PATHS (c)-[:KNOWS]->(d), \
         SHORTEST ACYCLIC PATH (d)-[:KNOWS]->(e), \
         ALL SHORTEST TRAIL PATHS (e)-[:KNOWS]->(f), \
         ANY SHORTEST SIMPLE (f)-[:KNOWS]->(g), \
         counted = SHORTEST 3 ACYCLIC (g)-[:KNOWS]->(h), \
         shortest_group = SHORTEST WALK GROUP (h)-[:KNOWS]->(i), \
         shortest_groups = SHORTEST 2 TRAIL PATHS GROUPS (i)-[:KNOWS]->(j) \
         RETURN a",
    )
    .unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };
    let patterns = &first_match(query).patterns;

    assert_eq!(
        patterns[0].prefix,
        Some(PathPatternPrefix::Search(PathSearchPrefix::All {
            mode: Some(PathMode::Walk),
            path_or_paths: None
        }))
    );
    assert_eq!(
        patterns[1].prefix,
        Some(PathPatternPrefix::Search(PathSearchPrefix::Any {
            count: None,
            mode: Some(PathMode::Trail),
            path_or_paths: Some(PathOrPaths::Path)
        }))
    );
    assert_eq!(
        patterns[2].prefix,
        Some(PathPatternPrefix::Search(PathSearchPrefix::Any {
            count: Some(UnsignedIntegerSpecification::Literal(2)),
            mode: Some(PathMode::Simple),
            path_or_paths: Some(PathOrPaths::Paths)
        }))
    );
    assert_eq!(
        patterns[3].prefix,
        Some(PathPatternPrefix::Search(PathSearchPrefix::Shortest {
            mode: Some(PathMode::Acyclic),
            path_or_paths: Some(PathOrPaths::Path)
        }))
    );
    assert_eq!(
        patterns[4].prefix,
        Some(PathPatternPrefix::Search(PathSearchPrefix::AllShortest {
            mode: Some(PathMode::Trail),
            path_or_paths: Some(PathOrPaths::Paths)
        }))
    );
    assert_eq!(
        patterns[5].prefix,
        Some(PathPatternPrefix::Search(PathSearchPrefix::AnyShortest {
            mode: Some(PathMode::Simple),
            path_or_paths: None
        }))
    );
    assert_eq!(patterns[6].variable, Some(ident("counted")));
    assert_eq!(
        patterns[6].prefix,
        Some(PathPatternPrefix::Search(
            PathSearchPrefix::CountedShortest {
                count: UnsignedIntegerSpecification::Literal(3),
                mode: Some(PathMode::Acyclic),
                path_or_paths: None
            }
        ))
    );
    assert_eq!(patterns[7].variable, Some(ident("shortest_group")));
    assert_eq!(
        patterns[7].prefix,
        Some(PathPatternPrefix::Search(
            PathSearchPrefix::CountedShortestGroup {
                count: None,
                mode: Some(PathMode::Walk),
                path_or_paths: None,
                groups: false
            }
        ))
    );
    assert_eq!(patterns[8].variable, Some(ident("shortest_groups")));
    assert_eq!(
        patterns[8].prefix,
        Some(PathPatternPrefix::Search(
            PathSearchPrefix::CountedShortestGroup {
                count: Some(UnsignedIntegerSpecification::Literal(2)),
                mode: Some(PathMode::Trail),
                path_or_paths: Some(PathOrPaths::Paths),
                groups: true
            }
        ))
    );
}

#[test]
fn parses_path_search_parameter_counts() {
    let program = parse(
        "MATCH ANY $path_count WALK PATHS (a)-[:KNOWS]->(b), \
         counted = SHORTEST $shortest_count TRAIL PATHS (b)-[:KNOWS]->(c), \
         grouped = SHORTEST $group_count SIMPLE GROUPS (c)-[:KNOWS]->(d) \
         RETURN a",
    )
    .unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };
    let patterns = &first_match(query).patterns;

    assert_eq!(
        patterns[0].prefix,
        Some(PathPatternPrefix::Search(PathSearchPrefix::Any {
            count: Some(UnsignedIntegerSpecification::Parameter(
                "path_count".to_owned()
            )),
            mode: Some(PathMode::Walk),
            path_or_paths: Some(PathOrPaths::Paths)
        }))
    );
    assert_eq!(
        patterns[1].prefix,
        Some(PathPatternPrefix::Search(
            PathSearchPrefix::CountedShortest {
                count: UnsignedIntegerSpecification::Parameter("shortest_count".to_owned()),
                mode: Some(PathMode::Trail),
                path_or_paths: Some(PathOrPaths::Paths)
            }
        ))
    );
    assert_eq!(
        patterns[2].prefix,
        Some(PathPatternPrefix::Search(
            PathSearchPrefix::CountedShortestGroup {
                count: Some(UnsignedIntegerSpecification::Parameter(
                    "group_count".to_owned()
                )),
                mode: Some(PathMode::Simple),
                path_or_paths: None,
                groups: true
            }
        ))
    );
}

#[test]
fn parses_graph_pattern_keep_clause() {
    let program = parse(
        "MATCH (a)-[:KNOWS]->(b), path = (b)-[:KNOWS]->(c) KEEP TRAIL PATHS WHERE a.active RETURN path; \
         MATCH (n)-[:FOLLOWS]->(m) KEEP ANY SHORTEST RETURN n",
    )
    .unwrap();

    let Statement::Query(mode_query) = &program.statements[0] else {
        panic!("expected query");
    };
    let mode_match = first_match(mode_query);
    assert_eq!(
        mode_match.keep,
        Some(PathPatternPrefix::Mode {
            mode: PathMode::Trail,
            path_or_paths: Some(PathOrPaths::Paths)
        })
    );
    assert_eq!(mode_match.patterns.len(), 2);
    assert_eq!(mode_match.patterns[0].prefix, None);
    assert_eq!(mode_match.patterns[1].variable, Some(ident("path")));
    assert!(mode_match.where_clause.is_some());

    let Statement::Query(search_query) = &program.statements[1] else {
        panic!("expected query");
    };
    assert_eq!(
        first_match(search_query).keep,
        Some(PathPatternPrefix::Search(PathSearchPrefix::AnyShortest {
            mode: None,
            path_or_paths: None
        }))
    );
}

#[test]
fn parses_path_pattern_unions() {
    let program = parse(
        "MATCH path = (a)-[:KNOWS]->(b) \
         | (a)-[:LIKES]->(b) \
         | (a)<-[:FOLLOWS]-(b) \
         RETURN path",
    )
    .unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };
    let pattern = &first_match(query).patterns[0];

    assert_eq!(pattern.variable, Some(ident("path")));
    assert_eq!(pattern.chains.len(), 1);
    assert_eq!(pattern.alternation, Some(PathPatternAlternation::Union));
    assert_eq!(pattern.alternatives.len(), 2);
    assert_eq!(pattern.alternatives[0].start.variable, Some(ident("a")));
    assert_eq!(pattern.alternatives[0].chains.len(), 1);
    assert_eq!(
        pattern.alternatives[1].chains[0].relationship.direction,
        Direction::Left
    );
    assert_eq!(
        pattern.alternatives[1].chains[0].node.variable,
        Some(ident("b"))
    );
}

#[test]
fn parses_path_multiset_alternation() {
    let program = parse(
        "MATCH path = (a)-[:KNOWS]->(b) \
         |+| (a)-[:LIKES]->(b) \
         |+| (a)<-[:FOLLOWS]-(b) \
         RETURN path",
    )
    .unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };
    let pattern = &first_match(query).patterns[0];

    assert_eq!(pattern.variable, Some(ident("path")));
    assert_eq!(pattern.alternation, Some(PathPatternAlternation::Multiset));
    assert_eq!(pattern.alternatives.len(), 2);
    assert_eq!(
        pattern.alternatives[1].chains[0].relationship.direction,
        Direction::Left
    );
}

#[test]
fn parses_inline_procedure_body_at_schema_with_definitions() {
    let program =
        parse("CALL { AT SCHEMA app.main VALUE fallback STRING = 'unknown' RETURN fallback } RETURN fallback")
            .unwrap();

    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query statement");
    };
    let QueryClause::Call(call) = &query.body.clauses[0] else {
        panic!("expected inline call clause");
    };
    let ProcedureCall::Inline(inline) = &call.call else {
        panic!("expected inline procedure call");
    };

    assert_eq!(
        inline.body.at_schema.as_ref().unwrap(),
        &SchemaReference::Name(schema_name(&["app", "main"]))
    );
    assert_eq!(inline.body.definitions.len(), 1);
    let BindingVariableDefinition::Value(value) = &inline.body.definitions[0] else {
        panic!("expected value variable definition");
    };
    assert_eq!(value.name, ident("fallback"));
    assert_eq!(inline.body.statements.len(), 1);
}

#[test]
fn parses_element_pattern_where_predicates() {
    let program =
        parse("MATCH (n WHERE n.age > 21)-[r WHERE r.since >= 2020]->(m) RETURN n, r, m").unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };
    let match_clause = first_match(query);
    let pattern = &match_clause.patterns[0];

    assert!(matches!(
        pattern.start.where_clause,
        Some(Expr::Binary {
            op: BinaryOp::Gt,
            ..
        })
    ));
    assert!(matches!(
        pattern.chains[0].relationship.where_clause,
        Some(Expr::Binary {
            op: BinaryOp::Ge,
            ..
        })
    ));
    assert!(pattern.chains[0].node.where_clause.is_none());
}

#[test]
fn parses_anonymous_element_pattern_label_and_where_fillers() {
    let program =
        parse("MATCH (IS Person WHERE true)-[IS KNOWS WHERE true]->(WHERE true) RETURN 1 AS ok")
            .unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };
    let pattern = &first_match(query).patterns[0];

    assert_eq!(pattern.start.variable, None);
    assert_eq!(pattern.start.labels, vec![ident("Person")]);
    assert_eq!(
        pattern.start.label_expression,
        Some(LabelExpression::Label(ident("Person")))
    );
    assert_eq!(
        pattern.start.where_clause,
        Some(Expr::Literal(Literal::Boolean(true)))
    );
    assert_eq!(pattern.chains[0].relationship.variable, None);
    assert_eq!(pattern.chains[0].relationship.labels, vec![ident("KNOWS")]);
    assert_eq!(
        pattern.chains[0].relationship.where_clause,
        Some(Expr::Literal(Literal::Boolean(true)))
    );
    assert_eq!(pattern.chains[0].node.variable, None);
    assert_eq!(
        pattern.chains[0].node.where_clause,
        Some(Expr::Literal(Literal::Boolean(true)))
    );
}

#[test]
fn parses_query_set_operations() {
    let program = parse(
        "RETURN 1 AS value \
         UNION ALL RETURN 2 AS value \
         UNION DISTINCT RETURN 3 AS value \
         UNION RETURN 4 AS value; \
         RETURN 1 AS value EXCEPT DISTINCT RETURN 2 AS value; \
         RETURN 1 AS value EXCEPT RETURN 2 AS value; \
         RETURN 1 AS value INTERSECT ALL RETURN 2 AS value; \
         RETURN 1 AS value OTHERWISE RETURN 2 AS value OTHERWISE RETURN 3 AS value",
    )
    .unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };

    assert_eq!(query.set_operations.len(), 3);
    assert_eq!(query.set_operations[0].operator, QuerySetOperator::Union);
    assert_eq!(query.set_operations[0].quantifier, Some(SetQuantifier::All));
    assert_eq!(query.set_operations[1].operator, QuerySetOperator::Union);
    assert_eq!(
        query.set_operations[1].quantifier,
        Some(SetQuantifier::Distinct)
    );
    assert_eq!(query.set_operations[2].operator, QuerySetOperator::Union);
    assert_eq!(
        query.set_operations[2].quantifier,
        Some(SetQuantifier::Distinct)
    );

    let Statement::Query(except_query) = &program.statements[1] else {
        panic!("expected except query");
    };
    assert_eq!(
        except_query.set_operations[0].operator,
        QuerySetOperator::Except
    );
    assert_eq!(
        except_query.set_operations[0].quantifier,
        Some(SetQuantifier::Distinct)
    );

    let Statement::Query(implicit_except_query) = &program.statements[2] else {
        panic!("expected implicit except query");
    };
    assert_eq!(
        implicit_except_query.set_operations[0].operator,
        QuerySetOperator::Except
    );
    assert_eq!(
        implicit_except_query.set_operations[0].quantifier,
        Some(SetQuantifier::Distinct)
    );

    let Statement::Query(intersect_query) = &program.statements[3] else {
        panic!("expected intersect query");
    };
    assert_eq!(
        intersect_query.set_operations[0].operator,
        QuerySetOperator::Intersect
    );
    assert_eq!(
        intersect_query.set_operations[0].quantifier,
        Some(SetQuantifier::All)
    );

    let Statement::Query(otherwise_query) = &program.statements[4] else {
        panic!("expected otherwise query");
    };
    assert_eq!(otherwise_query.set_operations.len(), 2);
    assert_eq!(
        otherwise_query.set_operations[0].operator,
        QuerySetOperator::Otherwise
    );
    assert_eq!(
        otherwise_query.set_operations[1].operator,
        QuerySetOperator::Otherwise
    );
}

#[test]
fn parses_for_ordinality_and_offset() {
    let program = parse(
        "FOR item IN [10, 20, 30] WITH ORDINALITY ord RETURN item, ord \
         UNION ALL \
         FOR item IN [10, 20, 30] WITH OFFSET pos RETURN item, pos",
    )
    .unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };

    let QueryClause::For(for_clause) = &query.body.clauses[0] else {
        panic!("expected for clause");
    };
    let ordinality = for_clause.ordinality_or_offset.as_ref().unwrap();
    assert_eq!(ordinality.kind, ForOrdinalityOrOffsetKind::Ordinality);
    assert_eq!(ordinality.variable, ident("ord"));

    let QueryClause::For(offset_clause) = &query.set_operations[0].body.clauses[0] else {
        panic!("expected for clause");
    };
    let offset = offset_clause.ordinality_or_offset.as_ref().unwrap();
    assert_eq!(offset.kind, ForOrdinalityOrOffsetKind::Offset);
    assert_eq!(offset.variable, ident("pos"));
}

#[test]
fn parses_filter_where_result_quantifier_and_finish() {
    let program = parse(
        "MATCH (n) FILTER WHERE n.age > 21 RETURN ALL n; \
         MATCH (n) FINISH",
    )
    .unwrap();

    let Statement::Query(first) = &program.statements[0] else {
        panic!("expected query");
    };
    assert_eq!(
        first.body.result_clause.quantifier,
        Some(SetQuantifier::All)
    );
    assert!(!first.body.result_clause.distinct);
    assert!(matches!(first.body.clauses[1], QueryClause::Filter(_)));
    assert_eq!(first.body.result_clause.items[0].alias, Some(ident("n")));

    let Statement::Query(second) = &program.statements[1] else {
        panic!("expected query");
    };
    assert_eq!(second.body.result_clause.kind, ResultKind::Finish);
    assert!(second.body.result_clause.items.is_empty());
}

#[test]
fn parses_implicit_all_result_quantifier() {
    let program = parse("MATCH (n) RETURN n; SELECT n FROM HOME_GRAPH MATCH (n)").unwrap();

    let Statement::Query(return_query) = &program.statements[0] else {
        panic!("expected return query");
    };
    assert_eq!(
        return_query.body.result_clause.quantifier,
        Some(SetQuantifier::All)
    );
    assert!(!return_query.body.result_clause.distinct);

    let Statement::Query(select_query) = &program.statements[1] else {
        panic!("expected select query");
    };
    assert_eq!(
        select_query.body.result_clause.quantifier,
        Some(SetQuantifier::All)
    );
    assert!(!select_query.body.result_clause.distinct);
}

#[test]
fn parses_linear_query_filter_and_let_clauses() {
    let program = parse(
        "MATCH (n:Person) FILTER n.age > 21 \
         LET VALUE score :: TYPED INTEGER = n.score, decade = n.age / 10 \
         RETURN n.name AS name, score, decade ORDER BY name ASC OFFSET 5 LIMIT 10",
    )
    .unwrap();

    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query statement");
    };

    assert_eq!(query.body.clauses.len(), 3);
    assert!(matches!(query.body.clauses[0], QueryClause::Match(_)));
    assert!(matches!(query.body.clauses[1], QueryClause::Filter(_)));
    let QueryClause::Let(let_clause) = &query.body.clauses[2] else {
        panic!("expected let clause");
    };
    assert_eq!(let_clause.items.len(), 2);
    assert_eq!(let_clause.items[0].name, ident("score"));
    assert!(let_clause.items[0].typed);
    assert_eq!(
        let_clause.items[0].value_type,
        Some(ValueType::ExactNumeric {
            kind: ExactNumericTypeKind::Integer,
            signed: Some(true),
            precision: None,
            scale: None,
        })
    );
    assert!(matches!(let_clause.items[0].value, Expr::Property { .. }));
    assert_eq!(let_clause.items[1].name, ident("decade"));
    assert!(!let_clause.items[1].typed);
    assert_eq!(let_clause.items[1].value_type, None);
    assert_eq!(query.body.order_by[0].direction, Some(SortDirection::Asc));
    assert_eq!(
        query.body.offset,
        Some(UnsignedIntegerSpecification::Literal(5))
    );
    assert_eq!(
        query.body.limit,
        Some(UnsignedIntegerSpecification::Literal(10))
    );
}

#[test]
fn parses_linear_order_by_page_clause_before_result() {
    let program = parse(
        "MATCH (n) ORDER BY n.name ASC NULLS LAST SKIP 5 LIMIT 10 \
         RETURN n ORDER BY n.score DESC LIMIT 3",
    )
    .unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query statement");
    };

    assert_eq!(query.body.clauses.len(), 2);
    let QueryClause::OrderByPage(page) = &query.body.clauses[1] else {
        panic!("expected order/page clause");
    };
    assert_eq!(page.order_by.len(), 1);
    assert_eq!(page.order_by[0].direction, Some(SortDirection::Asc));
    assert_eq!(page.order_by[0].null_ordering, Some(NullOrdering::Last));
    assert_eq!(page.offset, Some(UnsignedIntegerSpecification::Literal(5)));
    assert_eq!(page.limit, Some(UnsignedIntegerSpecification::Literal(10)));

    assert_eq!(query.body.order_by.len(), 1);
    assert_eq!(query.body.order_by[0].direction, Some(SortDirection::Desc));
    assert_eq!(
        query.body.limit,
        Some(UnsignedIntegerSpecification::Literal(3))
    );
}

#[test]
fn parses_order_by_page_as_primitive_query_statement() {
    let program = parse("SKIP 5 LIMIT 10 RETURN n").unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query statement");
    };

    assert_eq!(query.body.clauses.len(), 1);
    let QueryClause::OrderByPage(page) = &query.body.clauses[0] else {
        panic!("expected order/page clause");
    };
    assert!(page.order_by.is_empty());
    assert_eq!(page.offset, Some(UnsignedIntegerSpecification::Literal(5)));
    assert_eq!(page.limit, Some(UnsignedIntegerSpecification::Literal(10)));
    assert_eq!(query.body.result_clause.kind, ResultKind::Return);
}

#[test]
fn parses_call_yield_and_for_query_clauses() {
    let program = parse(
        "CALL graph.expand($start) YIELD node AS n, depth \
         FOR score IN [1, 2, 3] \
         FILTER depth >= 1 \
         RETURN n, score",
    )
    .unwrap();

    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query statement");
    };

    assert_eq!(query.body.clauses.len(), 3);
    let QueryClause::Call(call) = &query.body.clauses[0] else {
        panic!("expected call clause");
    };
    let ProcedureCall::Named { procedure, args } = &call.call else {
        panic!("expected named procedure call");
    };
    assert_eq!(procedure, &procedure_ref(&["graph", "expand"]));
    assert_eq!(args.len(), 1);
    let yield_clause = call.yield_clause.as_ref().unwrap();
    assert!(matches!(
        yield_clause.items[0],
        YieldItem::Item {
            ref name,
            alias: Some(_)
        } if name == &ident("node")
    ));

    let QueryClause::For(for_clause) = &query.body.clauses[1] else {
        panic!("expected for clause");
    };
    assert_eq!(for_clause.variable, ident("score"));
    assert!(matches!(for_clause.source, Expr::List(_)));
}

#[test]
fn parses_select_result_with_group_by() {
    let program = parse(
        "MATCH (p:Person) SELECT p.city AS city, count(*) AS total \
         GROUP BY city ORDER BY total DESC",
    )
    .unwrap();

    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query statement");
    };

    assert_eq!(query.body.result_clause.items.len(), 2);
    assert_eq!(query.body.group_by.len(), 1);
    assert_eq!(query.body.order_by[0].direction, Some(SortDirection::Desc));
    assert_eq!(query.body.order_by[0].null_ordering, None);
}

#[test]
fn parses_aggregate_set_quantifiers() {
    let program = parse(
        "MATCH (p:Person) \
         SELECT count(DISTINCT p.city) AS cities, sum(ALL p.score) AS total",
    )
    .unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query statement");
    };

    let Expr::Function {
        name,
        quantifier,
        args,
    } = &query.body.result_clause.items[0].expr
    else {
        panic!("expected count function");
    };
    assert_eq!(name, &ident("count"));
    assert_eq!(*quantifier, Some(SetQuantifier::Distinct));
    assert_eq!(args.len(), 1);
    assert!(matches!(args[0], Expr::Property { .. }));

    let Expr::Function {
        name,
        quantifier,
        args,
    } = &query.body.result_clause.items[1].expr
    else {
        panic!("expected sum function");
    };
    assert_eq!(name, &ident("sum"));
    assert_eq!(*quantifier, Some(SetQuantifier::All));
    assert_eq!(args.len(), 1);
}

#[test]
fn parses_standard_aggregate_function_names() {
    let program = parse(
        "MATCH (p:Person) RETURN COUNT(*) AS total, AVG(p.score) AS avg_score, \
         COLLECT_LIST(DISTINCT p.name) AS names, STDDEV_POP(p.score) AS spread, \
         PERCENTILE_CONT(DISTINCT p.score, 0.5) AS median, \
         PERCENTILE_DISC(ALL p.score, 0.9) AS percentile",
    )
    .unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };
    let items = &query.body.result_clause.items;

    let Expr::Function {
        name,
        quantifier,
        args,
    } = &items[0].expr
    else {
        panic!("expected count function");
    };
    assert_eq!(name, &ident("COUNT"));
    assert_eq!(*quantifier, None);
    assert_eq!(args, &vec![Expr::Wildcard]);

    let Expr::Function {
        name,
        quantifier,
        args,
    } = &items[1].expr
    else {
        panic!("expected avg function");
    };
    assert_eq!(name, &ident("AVG"));
    assert_eq!(*quantifier, Some(SetQuantifier::All));
    assert_eq!(args.len(), 1);

    let Expr::Function {
        name,
        quantifier,
        args,
    } = &items[2].expr
    else {
        panic!("expected collect_list function");
    };
    assert_eq!(name, &ident("COLLECT_LIST"));
    assert_eq!(*quantifier, Some(SetQuantifier::Distinct));
    assert_eq!(args.len(), 1);

    let Expr::Function {
        name,
        quantifier,
        args,
    } = &items[3].expr
    else {
        panic!("expected stddev_pop function");
    };
    assert_eq!(name, &ident("STDDEV_POP"));
    assert_eq!(*quantifier, Some(SetQuantifier::All));
    assert_eq!(args.len(), 1);

    let Expr::Function {
        name,
        quantifier,
        args,
    } = &items[4].expr
    else {
        panic!("expected percentile_cont function");
    };
    assert_eq!(name, &ident("PERCENTILE_CONT"));
    assert_eq!(*quantifier, Some(SetQuantifier::Distinct));
    assert_eq!(args.len(), 2);

    let Expr::Function {
        name,
        quantifier,
        args,
    } = &items[5].expr
    else {
        panic!("expected percentile_disc function");
    };
    assert_eq!(name, &ident("PERCENTILE_DISC"));
    assert_eq!(*quantifier, Some(SetQuantifier::All));
    assert_eq!(args.len(), 2);
}

#[test]
fn parses_having_after_group_by() {
    let program = parse(
        "MATCH (p:Person) SELECT p.city AS city, count(*) AS total \
         GROUP BY city HAVING total > 1 ORDER BY total DESC LIMIT 5",
    )
    .unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };

    assert_eq!(query.body.group_by.len(), 1);
    assert!(matches!(
        query.body.having,
        Some(Expr::Binary {
            op: BinaryOp::Gt,
            ..
        })
    ));
    assert_eq!(query.body.order_by[0].direction, Some(SortDirection::Desc));
    assert_eq!(
        query.body.limit,
        Some(UnsignedIntegerSpecification::Literal(5))
    );
}

#[test]
fn parses_empty_grouping_set() {
    let program =
        parse("MATCH (p:Person) SELECT count(*) AS total GROUP BY () HAVING total > 0").unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };

    assert!(query.body.empty_grouping_set);
    assert!(query.body.group_by.is_empty());
    assert!(matches!(
        query.body.having,
        Some(Expr::Binary {
            op: BinaryOp::Gt,
            ..
        })
    ));
}

#[test]
fn parses_select_from_graph_match_list() {
    let program = parse(
        "SELECT n.name AS name, m.name AS colleague \
         FROM social MATCH (n:Person) WHERE n.active = true, \
              work.graph MATCH (m:Person)-[:KNOWS]->(n) \
         GROUP BY name, colleague HAVING count(*) > 0 ORDER BY name",
    )
    .unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };

    assert!(query.body.clauses.is_empty());
    assert_eq!(query.body.select_from.len(), 2);
    assert_eq!(
        query.body.select_from[0].graph,
        GraphExpression::Name(graph_name(&["social"]))
    );
    assert_eq!(
        query.body.select_from[1].graph,
        GraphExpression::Name(graph_name(&["work", "graph"]))
    );
    assert!(matches!(
        query.body.select_from[0].match_clause.where_clause,
        Some(Expr::Binary {
            op: BinaryOp::Eq,
            ..
        })
    ));
    assert_eq!(
        query.body.select_from[1].match_clause.patterns[0]
            .chains
            .len(),
        1
    );
    assert_eq!(query.body.group_by.len(), 2);
    assert!(query.body.having.is_some());
}

#[test]
fn parses_select_star_from_graph_match() {
    let program = parse("SELECT * FROM social MATCH (n:Person) ORDER BY n.name LIMIT 10").unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };

    assert_eq!(query.body.result_clause.kind, ResultKind::Select);
    assert_eq!(query.body.result_clause.items.len(), 1);
    assert_eq!(query.body.result_clause.items[0].expr, Expr::Wildcard);
    assert_eq!(query.body.result_clause.items[0].alias, None);
    assert_eq!(query.body.select_from.len(), 1);
    assert_eq!(
        query.body.select_from[0].graph,
        GraphExpression::Name(graph_name(&["social"]))
    );
    assert_eq!(
        query.body.limit,
        Some(UnsignedIntegerSpecification::Literal(10))
    );
}

#[test]
fn parses_select_from_nested_query_specifications() {
    let program = parse(
        "SELECT name FROM { MATCH (n:Person) RETURN n.name AS name LIMIT 5 } WHERE name IS NOT NULL; \
         SELECT name FROM social { MATCH (n:Person) RETURN n.name AS name LIMIT 5 }",
    )
    .unwrap();

    let Statement::Query(first) = &program.statements[0] else {
        panic!("expected query");
    };
    assert_eq!(first.body.result_clause.items[0].alias, Some(ident("name")));
    assert!(first.body.select_from.is_empty());
    let first_select = first
        .body
        .select_query
        .as_ref()
        .expect("expected select query specification");
    assert_eq!(first_select.graph, None);
    assert!(matches!(
        first_select.query.body.clauses[0],
        QueryClause::Match(_)
    ));
    assert_eq!(
        first_select.query.body.limit,
        Some(UnsignedIntegerSpecification::Literal(5))
    );
    assert!(matches!(
        first.body.select_where,
        Some(Expr::IsNull { negated: true, .. })
    ));

    let Statement::Query(second) = &program.statements[1] else {
        panic!("expected query");
    };
    assert!(second.body.select_from.is_empty());
    let second_select = second
        .body
        .select_query
        .as_ref()
        .expect("expected graph select query specification");
    assert_eq!(
        second_select.graph,
        Some(GraphExpression::Name(graph_name(&["social"])))
    );
    assert!(matches!(
        second_select.query.body.result_clause.items[0].expr,
        Expr::Property { .. }
    ));
    assert_eq!(second.body.select_where, None);
}

#[test]
fn parses_linear_nested_query_specifications() {
    let program = parse(
        "{ MATCH (n) RETURN n }; \
         USE social { MATCH (n) RETURN n }; \
         RETURN 1 AS value UNION USE reports { MATCH (r) RETURN r }",
    )
    .unwrap();
    assert_eq!(program.statements.len(), 3);

    let Statement::Query(first) = &program.statements[0] else {
        panic!("expected first query");
    };
    let QueryClause::NestedQuery(first_nested) = &first.body.clauses[0] else {
        panic!("expected nested query clause");
    };
    assert!(matches!(
        first_nested.body.clauses[0],
        QueryClause::Match(_)
    ));

    let Statement::Query(second) = &program.statements[1] else {
        panic!("expected second query");
    };
    assert_eq!(
        second.use_graph,
        Some(GraphExpression::Name(graph_name(&["social"])))
    );
    assert!(matches!(
        second.body.clauses[0],
        QueryClause::NestedQuery(_)
    ));

    let Statement::Query(third) = &program.statements[2] else {
        panic!("expected third query");
    };
    assert_eq!(third.set_operations.len(), 1);
    assert_eq!(third.set_operations[0].operator, QuerySetOperator::Union);
    assert_eq!(
        third.set_operations[0].body.clauses[0],
        QueryClause::UseGraph(GraphExpression::Name(graph_name(&["reports"])))
    );
    assert!(matches!(
        third.set_operations[0].body.clauses[1],
        QueryClause::NestedQuery(_)
    ));
}

#[test]
fn parses_order_by_null_ordering() {
    let program =
        parse("RETURN n ORDER BY n.name ASC NULLS LAST, n.score DESC NULLS FIRST").unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };

    assert_eq!(query.body.order_by.len(), 2);
    assert_eq!(query.body.order_by[0].direction, Some(SortDirection::Asc));
    assert_eq!(
        query.body.order_by[0].null_ordering,
        Some(NullOrdering::Last)
    );
    assert_eq!(query.body.order_by[1].direction, Some(SortDirection::Desc));
    assert_eq!(
        query.body.order_by[1].null_ordering,
        Some(NullOrdering::First)
    );
}

#[test]
fn parses_ordering_and_offset_synonyms() {
    let program = parse(
        "RETURN n ORDER BY n.name ASCENDING, n.score DESCENDING SKIP 5 LIMIT 10; \
         RETURN n OFFSET $offset LIMIT $limit",
    )
    .unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };

    assert_eq!(query.body.order_by.len(), 2);
    assert_eq!(query.body.order_by[0].direction, Some(SortDirection::Asc));
    assert_eq!(query.body.order_by[1].direction, Some(SortDirection::Desc));
    assert_eq!(
        query.body.offset,
        Some(UnsignedIntegerSpecification::Literal(5))
    );
    assert_eq!(
        query.body.limit,
        Some(UnsignedIntegerSpecification::Literal(10))
    );

    let Statement::Query(parameterized) = &program.statements[1] else {
        panic!("expected parameterized query");
    };
    assert_eq!(
        parameterized.body.offset,
        Some(UnsignedIntegerSpecification::Parameter("offset".to_owned()))
    );
    assert_eq!(
        parameterized.body.limit,
        Some(UnsignedIntegerSpecification::Parameter("limit".to_owned()))
    );
}

#[test]
fn parses_primitive_result_statement_without_linear_clauses() {
    let program = parse("SELECT 1 AS one, 'x' AS label").unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query statement");
    };

    assert!(query.body.clauses.is_empty());
    assert_eq!(query.body.result_clause.items.len(), 2);
    assert_eq!(query.body.result_clause.items[0].alias, Some(ident("one")));
}

#[test]
fn parses_null_predicates() {
    let program =
        parse("MATCH (n) WHERE n.deleted_at IS NULL OR n.name IS NOT NULL RETURN n").unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };
    let match_clause = first_match(query);

    let Some(Expr::Binary {
        left,
        op: BinaryOp::Or,
        right,
    }) = &match_clause.where_clause
    else {
        panic!("expected disjunction");
    };
    assert!(matches!(left.as_ref(), Expr::IsNull { negated: false, .. }));
    assert!(matches!(right.as_ref(), Expr::IsNull { negated: true, .. }));
}

#[test]
fn parses_unknown_truth_value_predicates() {
    let program = parse(
        "RETURN TRUE AS t, FALSE AS f, UNKNOWN AS u, \
         $flag IS TRUE AS truthy, $flag IS NOT FALSE AS not_false, \
         $flag IS UNKNOWN AS maybe, $flag IS NOT UNKNOWN AS known",
    )
    .unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };
    let items = &query.body.result_clause.items;

    assert_eq!(items[0].expr, Expr::Literal(Literal::Boolean(true)));
    assert_eq!(items[1].expr, Expr::Literal(Literal::Boolean(false)));
    assert_eq!(items[2].expr, Expr::Literal(Literal::Unknown));
    assert!(matches!(
        items[3].expr,
        Expr::IsTruth {
            negated: false,
            value: true,
            ..
        }
    ));
    assert!(matches!(
        items[4].expr,
        Expr::IsTruth {
            negated: true,
            value: false,
            ..
        }
    ));
    assert!(matches!(
        items[5].expr,
        Expr::IsUnknown { negated: false, .. }
    ));
    assert!(matches!(
        items[6].expr,
        Expr::IsUnknown { negated: true, .. }
    ));
}

#[test]
fn parses_property_exists_predicate() {
    let program = parse("MATCH (n) WHERE PROPERTY_EXISTS(n, name) RETURN n").unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };
    let match_clause = first_match(query);

    let Some(Expr::PropertyExists { variable, property }) = &match_clause.where_clause else {
        panic!("expected property exists predicate");
    };
    assert_eq!(variable, &ident("n"));
    assert_eq!(property, &ident("name"));
}

#[test]
fn parses_element_id_function() {
    let program = parse("MATCH (n) RETURN ELEMENT_ID(n) AS id").unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };

    let Expr::ElementId { variable } = &query.body.result_clause.items[0].expr else {
        panic!("expected element id function");
    };
    assert_eq!(variable, &ident("n"));
    assert_eq!(query.body.result_clause.items[0].alias, Some(ident("id")));
}

#[test]
fn parses_exists_graph_pattern_predicate() {
    let program = parse("MATCH (n) WHERE EXISTS { (n)-[:KNOWS]->(:Person) } RETURN n").unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };
    let match_clause = first_match(query);

    let Some(Expr::Exists { patterns }) = &match_clause.where_clause else {
        panic!("expected exists predicate");
    };
    assert_eq!(patterns.len(), 1);
    assert_eq!(patterns[0].chains.len(), 1);
    assert_eq!(
        patterns[0].chains[0].relationship.labels,
        vec![ident("KNOWS")]
    );
    assert_eq!(patterns[0].chains[0].node.labels, vec![ident("Person")]);
}

#[test]
fn parses_exists_predicate_variants() {
    let program = parse(
        "MATCH (n) WHERE EXISTS ((n)-[:KNOWS]->(:Person)) \
         OR EXISTS { MATCH (m) RETURN m LIMIT 1 } RETURN n",
    )
    .unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };
    let match_clause = first_match(query);

    let Some(Expr::Binary { left, right, .. }) = &match_clause.where_clause else {
        panic!("expected combined predicate");
    };
    let Expr::Exists { patterns } = left.as_ref() else {
        panic!("expected graph pattern exists predicate");
    };
    assert_eq!(patterns.len(), 1);
    assert_eq!(
        patterns[0].chains[0].relationship.labels,
        vec![ident("KNOWS")]
    );

    let Expr::ExistsQuery { query: nested } = right.as_ref() else {
        panic!("expected nested query exists predicate");
    };
    assert!(matches!(nested.body.clauses[0], QueryClause::Match(_)));
    assert_eq!(
        nested.body.limit,
        Some(UnsignedIntegerSpecification::Literal(1))
    );
}

#[test]
fn parses_exists_match_statement_block() {
    let program = parse(
        "MATCH (n) WHERE EXISTS { MATCH (n)-[:KNOWS]->(m) \
         OPTIONAL MATCH (m)-[:LIKES]->(x) } \
         AND EXISTS (MATCH (n)-[:FOLLOWS]->(f)) RETURN n",
    )
    .unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };
    let match_clause = first_match(query);

    let Some(Expr::Binary { left, right, .. }) = &match_clause.where_clause else {
        panic!("expected combined predicate");
    };
    let Expr::ExistsMatch { matches } = left.as_ref() else {
        panic!("expected exists match block");
    };
    assert_eq!(matches.len(), 2);
    assert!(!matches[0].optional);
    assert!(matches[1].optional);
    assert_eq!(
        matches[0].patterns[0].chains[0].relationship.labels,
        vec![ident("KNOWS")]
    );
    assert_eq!(
        matches[1].patterns[0].chains[0].relationship.labels,
        vec![ident("LIKES")]
    );

    let Expr::ExistsMatch { matches } = right.as_ref() else {
        panic!("expected parenthesized exists match block");
    };
    assert_eq!(matches.len(), 1);
    assert_eq!(
        matches[0].patterns[0].chains[0].relationship.labels,
        vec![ident("FOLLOWS")]
    );
}

#[test]
fn parses_same_predicate() {
    let program = parse("MATCH (a), (b), (c) WHERE SAME(a, b, c) RETURN a").unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };
    let match_clause = first_match(query);

    let Some(Expr::Same { variables }) = &match_clause.where_clause else {
        panic!("expected same predicate");
    };
    assert_eq!(variables, &vec![ident("a"), ident("b"), ident("c")]);
}

#[test]
fn parses_all_different_predicate() {
    let program = parse("MATCH (a), (b), (c) WHERE ALL_DIFFERENT(a, b, c) RETURN a").unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };
    let match_clause = first_match(query);

    let Some(Expr::AllDifferent { variables }) = &match_clause.where_clause else {
        panic!("expected all different predicate");
    };
    assert_eq!(variables, &vec![ident("a"), ident("b"), ident("c")]);
}

#[test]
fn parses_source_destination_predicates() {
    let program =
        parse("MATCH (a)-[e]->(b) WHERE a IS SOURCE OF e AND b IS NOT DESTINATION OF e RETURN e")
            .unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };
    let match_clause = first_match(query);

    let Some(Expr::Binary {
        left,
        op: BinaryOp::And,
        right,
    }) = &match_clause.where_clause
    else {
        panic!("expected conjunction");
    };
    assert!(matches!(
        left.as_ref(),
        Expr::SourceDestination {
            node,
            negated: false,
            kind: SourceDestinationKind::Source,
            edge,
        } if node == &ident("a") && edge == &ident("e")
    ));
    assert!(matches!(
        right.as_ref(),
        Expr::SourceDestination {
            node,
            negated: true,
            kind: SourceDestinationKind::Destination,
            edge,
        } if node == &ident("b") && edge == &ident("e")
    ));
}

#[test]
fn parses_normalized_predicates() {
    let program =
        parse("RETURN 'cafe' IS NORMALIZED AS plain, 'cafe' IS NOT NFC NORMALIZED AS nfc").unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };

    let plain = &query.body.result_clause.items[0].expr;
    assert!(matches!(
        plain,
        Expr::IsNormalized {
            negated: false,
            normal_form: None,
            ..
        }
    ));

    let nfc = &query.body.result_clause.items[1].expr;
    assert!(matches!(
        nfc,
        Expr::IsNormalized {
            negated: true,
            normal_form: Some(form),
            ..
        } if form == &ident("NFC")
    ));
}

#[test]
fn parses_directed_predicates() {
    let program =
        parse("MATCH ()-[e]->() WHERE e IS DIRECTED AND e IS NOT DIRECTED RETURN e").unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };
    let match_clause = first_match(query);

    let Some(Expr::Binary {
        left,
        op: BinaryOp::And,
        right,
    }) = &match_clause.where_clause
    else {
        panic!("expected conjunction");
    };
    assert!(matches!(
        left.as_ref(),
        Expr::IsDirected {
            variable,
            negated: false,
        } if variable == &ident("e")
    ));
    assert!(matches!(
        right.as_ref(),
        Expr::IsDirected {
            variable,
            negated: true,
        } if variable == &ident("e")
    ));
}

#[test]
fn parses_labeled_predicates() {
    let program =
        parse("MATCH (n) WHERE n IS NOT LABELED Person|Admin OR n:Employee RETURN n").unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };
    let match_clause = first_match(query);

    let Some(Expr::Binary {
        left,
        op: BinaryOp::Or,
        right,
    }) = &match_clause.where_clause
    else {
        panic!("expected disjunction");
    };
    assert!(matches!(
        left.as_ref(),
        Expr::IsLabeled {
            variable,
            negated: true,
            label_expression: LabelExpression::Or(_, _),
        } if variable == &ident("n")
    ));
    assert!(matches!(
        right.as_ref(),
        Expr::IsLabeled {
            variable,
            negated: false,
            label_expression: LabelExpression::Label(label),
        } if variable == &ident("n") && label == &ident("Employee")
    ));
}

#[test]
fn parses_value_type_predicates() {
    let program = parse(
        "MATCH (n) WHERE n.tags IS TYPED LIST<STRING> \
         AND n.payload IS NOT TYPED RECORD {score INTEGER} \
         AND n.age IS NOT :: INTEGER RETURN n",
    )
    .unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };
    let match_clause = first_match(query);

    let Some(Expr::Binary {
        left,
        op: BinaryOp::And,
        right,
    }) = &match_clause.where_clause
    else {
        panic!("expected conjunction");
    };
    let Expr::Binary {
        left: typed_list,
        op: BinaryOp::And,
        right: typed_record,
    } = left.as_ref()
    else {
        panic!("expected nested conjunction");
    };
    assert!(matches!(
        typed_list.as_ref(),
        Expr::IsTyped {
            negated: false,
            value_type: ValueType::List(_),
            ..
        }
    ));
    assert!(matches!(
        typed_record.as_ref(),
        Expr::IsTyped {
            negated: true,
            value_type: ValueType::Record(_),
            ..
        }
    ));
    assert!(matches!(
        right.as_ref(),
        Expr::IsTyped {
            negated: true,
            value_type: ValueType::ExactNumeric { .. },
            ..
        }
    ));
}

#[test]
fn parses_chunked_character_string_literals() {
    let program = parse("RETURN 'hello '\n'world' AS greeting, 'it''s ok' AS escaped").unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };
    let items = &query.body.result_clause.items;

    assert_eq!(
        items[0].expr,
        Expr::Literal(Literal::String("hello world".to_owned()))
    );
    assert_eq!(
        items[1].expr,
        Expr::Literal(Literal::String("it's ok".to_owned()))
    );
}

#[test]
fn parses_escaped_and_no_escape_character_string_literals() {
    let program = parse(
        r#"RETURN 'line\nnext' AS escaped, 'quote\'' AS quote, '\\' AS slash, '\u0041\U00005A' AS unicode, @'raw\ntext' AS raw, "double ""quote""" AS double_quoted, @"raw\ntext" AS raw_double"#,
    )
    .unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };
    let items = &query.body.result_clause.items;

    assert_eq!(
        items[0].expr,
        Expr::Literal(Literal::String("line\nnext".to_owned()))
    );
    assert_eq!(
        items[1].expr,
        Expr::Literal(Literal::String("quote'".to_owned()))
    );
    assert_eq!(
        items[2].expr,
        Expr::Literal(Literal::String("\\".to_owned()))
    );
    assert_eq!(
        items[3].expr,
        Expr::Literal(Literal::String("AZ".to_owned()))
    );
    assert_eq!(
        items[4].expr,
        Expr::Literal(Literal::String(r"raw\ntext".to_owned()))
    );
    assert_eq!(
        items[5].expr,
        Expr::Literal(Literal::String("double \"quote\"".to_owned()))
    );
    assert_eq!(
        items[6].expr,
        Expr::Literal(Literal::String(r"raw\ntext".to_owned()))
    );
}

#[test]
fn parses_typed_temporal_literals() {
    let program = parse(
        "SELECT DATE '2026-06-29' AS d, TIME '12:30:00' AS t, \
         DATETIME '2026-06-29T12:30:00' AS dt, \
         TIMESTAMP '2026-06-29T12:30:00' AS ts, DURATION 'P1D' AS dur, \
         INTERVAL '1' DAY AS one_day, DATE \"2026-06-30\" AS double_quoted_date, \
         DURATION @`P\\n1D` AS raw_accent_duration",
    )
    .unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };

    let items = &query.body.result_clause.items;
    assert!(matches!(items[0].expr, Expr::Literal(Literal::Date(_))));
    assert!(matches!(items[1].expr, Expr::Literal(Literal::Time(_))));
    assert!(matches!(items[2].expr, Expr::Literal(Literal::DateTime(_))));
    assert!(matches!(items[3].expr, Expr::Literal(Literal::DateTime(_))));
    assert!(matches!(items[4].expr, Expr::Literal(Literal::Duration(_))));
    assert!(matches!(
        items[5].expr,
        Expr::Literal(Literal::SqlInterval { .. })
    ));
    assert_eq!(
        items[6].expr,
        Expr::Literal(Literal::Date("2026-06-30".to_owned()))
    );
    assert_eq!(
        items[7].expr,
        Expr::Literal(Literal::Duration(r"P\n1D".to_owned()))
    );
}

#[test]
fn parses_sql_interval_literals() {
    let program = parse(
        "RETURN INTERVAL '1' DAY AS one_day, \
         INTERVAL '2-3' YEAR TO MONTH AS years_months, \
         INTERVAL -'4.500' SECOND(3) AS negative_seconds, \
         INTERVAL \"5\" HOUR AS double_quoted_hour, \
         INTERVAL '1 02:03:04' DAY(2) TO SECOND(6) AS day_to_second",
    )
    .unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };
    let items = &query.body.result_clause.items;

    let Expr::Literal(Literal::SqlInterval { value, qualifier }) = &items[0].expr else {
        panic!("expected sql interval literal");
    };
    assert_eq!(value, "1");
    assert_eq!(qualifier.start, ident("DAY"));
    assert_eq!(qualifier.start_precision, None);
    assert_eq!(qualifier.end, None);
    assert_eq!(qualifier.end_precision, None);

    let Expr::Literal(Literal::SqlInterval { value, qualifier }) = &items[1].expr else {
        panic!("expected sql interval literal");
    };
    assert_eq!(value, "2-3");
    assert_eq!(qualifier.start, ident("YEAR"));
    assert_eq!(qualifier.start_precision, None);
    assert_eq!(qualifier.end, Some(ident("MONTH")));
    assert_eq!(qualifier.end_precision, None);

    let Expr::Literal(Literal::SqlInterval { value, qualifier }) = &items[2].expr else {
        panic!("expected sql interval literal");
    };
    assert_eq!(value, "-4.500");
    assert_eq!(qualifier.start, ident("SECOND"));
    assert_eq!(qualifier.start_precision, Some(3));
    assert_eq!(qualifier.end, None);
    assert_eq!(qualifier.end_precision, None);

    assert!(matches!(
        &items[3].expr,
        Expr::Literal(Literal::SqlInterval { value, qualifier })
            if value == "5" && qualifier.start == ident("HOUR")
    ));

    let Expr::Literal(Literal::SqlInterval { value, qualifier }) = &items[4].expr else {
        panic!("expected sql interval literal");
    };
    assert_eq!(value, "1 02:03:04");
    assert_eq!(qualifier.start, ident("DAY"));
    assert_eq!(qualifier.start_precision, Some(2));
    assert_eq!(qualifier.end, Some(ident("SECOND")));
    assert_eq!(qualifier.end_precision, Some(6));
}

#[test]
fn parses_radix_and_separated_numeric_literals() {
    let program = parse(
        "RETURN 1_000 AS decimal, 0x_FF AS hex, 0o755 AS octal, 0b1010_0101 AS binary, \
         .5 AS leading_decimal, 1. AS trailing_decimal, 10M AS exact_decimal, \
         2.5F AS float_value, 1e3D AS double_value",
    )
    .unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };
    let items = &query.body.result_clause.items;

    assert_eq!(items[0].expr, Expr::Literal(Literal::Integer(1000)));
    assert_eq!(items[1].expr, Expr::Literal(Literal::Integer(255)));
    assert_eq!(items[2].expr, Expr::Literal(Literal::Integer(493)));
    assert_eq!(items[3].expr, Expr::Literal(Literal::Integer(165)));
    assert_eq!(items[4].expr, Expr::Literal(Literal::Decimal(0.5)));
    assert_eq!(items[5].expr, Expr::Literal(Literal::Decimal(1.0)));
    assert_eq!(items[6].expr, Expr::Literal(Literal::Decimal(10.0)));
    assert_eq!(items[7].expr, Expr::Literal(Literal::Decimal(2.5)));
    assert_eq!(items[8].expr, Expr::Literal(Literal::Decimal(1000.0)));
}

#[test]
fn parses_signed_numeric_expressions() {
    let program = parse("RETURN +1 AS positive, -2 AS negative, -+3 AS nested_sign").unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };
    let items = &query.body.result_clause.items;

    assert_eq!(
        items[0].expr,
        Expr::Unary {
            op: UnaryOp::Pos,
            expr: Box::new(Expr::Literal(Literal::Integer(1))),
        }
    );
    assert_eq!(
        items[1].expr,
        Expr::Unary {
            op: UnaryOp::Neg,
            expr: Box::new(Expr::Literal(Literal::Integer(2))),
        }
    );
    assert_eq!(
        items[2].expr,
        Expr::Unary {
            op: UnaryOp::Neg,
            expr: Box::new(Expr::Unary {
                op: UnaryOp::Pos,
                expr: Box::new(Expr::Literal(Literal::Integer(3))),
            }),
        }
    );
}

#[test]
fn parses_byte_string_literals() {
    let program = parse("RETURN X'0A ff 10' AS raw, x'' AS empty, X'CA'\n'FE' AS chunked").unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };

    let items = &query.body.result_clause.items;
    assert_eq!(
        items[0].expr,
        Expr::Literal(Literal::Bytes(vec![0x0a, 0xff, 0x10]))
    );
    assert_eq!(items[1].expr, Expr::Literal(Literal::Bytes(vec![])));
    assert_eq!(
        items[2].expr,
        Expr::Literal(Literal::Bytes(vec![0xca, 0xfe]))
    );
}

#[test]
fn parses_byte_string_value_functions() {
    let program = parse(
        "RETURN LEFT(X'0A0B0C', 2) AS prefix, RIGHT(X'0A0B0C', 1) AS suffix, \
         TRIM(BOTH X'00' FROM X'000A00') AS trimmed",
    )
    .unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };

    let items = &query.body.result_clause.items;
    let Expr::SubstringByLength {
        side,
        value,
        length,
    } = &items[0].expr
    else {
        panic!("expected left byte substring");
    };
    assert_eq!(*side, SubstringSide::Left);
    assert_eq!(
        value.as_ref(),
        &Expr::Literal(Literal::Bytes(vec![0x0a, 0x0b, 0x0c]))
    );
    assert_eq!(length.as_ref(), &Expr::Literal(Literal::Integer(2)));

    let Expr::SubstringByLength { side, value, .. } = &items[1].expr else {
        panic!("expected right byte substring");
    };
    assert_eq!(*side, SubstringSide::Right);
    assert!(matches!(value.as_ref(), Expr::Literal(Literal::Bytes(_))));

    let Expr::Trim {
        specification,
        trim_character,
        value,
    } = &items[2].expr
    else {
        panic!("expected byte trim expression");
    };
    assert_eq!(*specification, Some(TrimSpec::Both));
    assert_eq!(
        trim_character.as_deref(),
        Some(&Expr::Literal(Literal::Bytes(vec![0x00])))
    );
    assert_eq!(
        value.as_ref(),
        &Expr::Literal(Literal::Bytes(vec![0x00, 0x0a, 0x00]))
    );
}

#[test]
fn parses_duration_value_functions() {
    let program = parse(
        "RETURN DURATION('P1D') AS from_string, \
         DURATION(RECORD {days: 1, hours: 2}) AS from_record, \
         DURATION(\"P\"\"2D\") AS from_double_quoted_string, \
         ABS(DURATION 'P1D') AS absolute_duration, duration AS identifier",
    )
    .unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };
    let items = &query.body.result_clause.items;

    let Expr::DurationFunction { value } = &items[0].expr else {
        panic!("expected duration function");
    };
    assert_eq!(
        value.as_ref(),
        &Expr::Literal(Literal::String("P1D".to_owned()))
    );

    let Expr::DurationFunction { value } = &items[1].expr else {
        panic!("expected duration function");
    };
    assert!(matches!(
        value.as_ref(),
        Expr::Record {
            explicit: true,
            fields
        } if fields.entries.len() == 2
    ));

    let Expr::DurationFunction { value } = &items[2].expr else {
        panic!("expected duration function");
    };
    assert_eq!(
        value.as_ref(),
        &Expr::Literal(Literal::String("P\"2D".to_owned()))
    );

    let Expr::AbsoluteValue { expr } = &items[3].expr else {
        panic!("expected absolute duration function");
    };
    assert_eq!(
        expr.as_ref(),
        &Expr::Literal(Literal::Duration("P1D".to_owned()))
    );

    assert_eq!(items[4].expr, Expr::Identifier(ident("duration")));
}

#[test]
fn parses_duration_between_function() {
    let program = parse(
        "RETURN DURATION_BETWEEN(DATE '2026-06-29', DATE '2026-06-01') AS days_between, \
         DURATION_BETWEEN(DATETIME '2026-06-29T12:00:00', \
                          DATETIME '2026-06-29T10:30:00') AS time_between, \
         duration_between AS identifier",
    )
    .unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };
    let items = &query.body.result_clause.items;

    let Expr::DurationBetween { left, right } = &items[0].expr else {
        panic!("expected duration between function");
    };
    assert_eq!(
        left.as_ref(),
        &Expr::Literal(Literal::Date("2026-06-29".to_owned()))
    );
    assert_eq!(
        right.as_ref(),
        &Expr::Literal(Literal::Date("2026-06-01".to_owned()))
    );

    let Expr::DurationBetween { left, right } = &items[1].expr else {
        panic!("expected duration between function");
    };
    assert!(matches!(left.as_ref(), Expr::Literal(Literal::DateTime(_))));
    assert!(matches!(
        right.as_ref(),
        Expr::Literal(Literal::DateTime(_))
    ));

    assert_eq!(items[2].expr, Expr::Identifier(ident("duration_between")));
}

#[test]
fn parses_extract_as_regular_identifier() {
    let program = parse("RETURN extract AS identifier").unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };
    let items = &query.body.result_clause.items;

    assert_eq!(items[0].expr, Expr::Identifier(ident("extract")));
}

#[test]
fn parses_local_datetime_value_functions() {
    let program = parse(
        "RETURN CURRENT_DATE AS d, \
         CURRENT_TIME AS t, \
         CURRENT_TIMESTAMP AS ts, \
         CURRENT_USER AS current_user, \
         LOCAL_TIME AS lt, \
         LOCAL_TIMESTAMP AS lts, \
         localtime AS localtime_identifier, \
         localtimestamp AS localtimestamp_identifier",
    )
    .unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };
    let items = &query.body.result_clause.items;

    assert_eq!(items[0].expr, Expr::CurrentDate);
    assert_eq!(items[1].expr, Expr::CurrentTime { precision: None });
    assert_eq!(items[2].expr, Expr::CurrentTimestamp { precision: None });
    assert_eq!(items[3].expr, Expr::CurrentUser);
    assert_eq!(items[4].expr, Expr::LocalTime { precision: None });
    assert_eq!(items[5].expr, Expr::LocalTimestamp { precision: None });
    assert_eq!(items[6].expr, Expr::Identifier(ident("localtime")));
    assert_eq!(items[7].expr, Expr::Identifier(ident("localtimestamp")));
}

#[test]
fn parses_standard_datetime_value_functions() {
    let program = parse(
        "RETURN DATE() AS current_date, \
         DATE('2026-06-29') AS date_from_string, \
         DATE(\"2026-06-30\") AS date_from_double_quoted_string, \
         DATE(RECORD {year: 2026, month: 6, day: 29}) AS date_from_record, \
         DATE({year: 2026}) AS date_from_implicit_record, \
         ZONED_TIME('12:30:00Z') AS zoned_time, \
         ZONED_DATETIME('2026-06-29T12:30:00Z') AS zoned_datetime, \
         LOCAL_TIME() AS local_time, \
         LOCAL_DATETIME(RECORD {year: 2026, month: 6, day: 29, hour: 12}) AS local_datetime, \
         zoned_time AS identifier",
    )
    .unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };
    let items = &query.body.result_clause.items;

    assert_eq!(
        items[0].expr,
        Expr::DateTimeFunction {
            function: DateTimeFunctionKind::Date,
            value: None,
        }
    );
    assert!(matches!(
        items[1].expr,
        Expr::DateTimeFunction {
            function: DateTimeFunctionKind::Date,
            value: Some(ref value),
        } if value.as_ref() == &Expr::Literal(Literal::String("2026-06-29".to_owned()))
    ));
    assert!(matches!(
        items[2].expr,
        Expr::DateTimeFunction {
            function: DateTimeFunctionKind::Date,
            value: Some(ref value),
        } if value.as_ref() == &Expr::Literal(Literal::String("2026-06-30".to_owned()))
    ));
    assert!(matches!(
        items[3].expr,
        Expr::DateTimeFunction {
            function: DateTimeFunctionKind::Date,
            value: Some(ref value),
        } if matches!(value.as_ref(), Expr::Record { explicit: true, fields } if fields.entries.len() == 3)
    ));
    assert!(matches!(
        items[4].expr,
        Expr::DateTimeFunction {
            function: DateTimeFunctionKind::Date,
            value: Some(ref value),
        } if matches!(value.as_ref(), Expr::Record { explicit: false, fields } if fields.entries.len() == 1)
    ));
    assert!(matches!(
        items[5].expr,
        Expr::DateTimeFunction {
            function: DateTimeFunctionKind::ZonedTime,
            value: Some(_),
        }
    ));
    assert!(matches!(
        items[6].expr,
        Expr::DateTimeFunction {
            function: DateTimeFunctionKind::ZonedDateTime,
            value: Some(_),
        }
    ));
    assert_eq!(
        items[7].expr,
        Expr::DateTimeFunction {
            function: DateTimeFunctionKind::LocalTime,
            value: None,
        }
    );
    assert!(matches!(
        items[8].expr,
        Expr::DateTimeFunction {
            function: DateTimeFunctionKind::LocalDateTime,
            value: Some(ref value),
        } if matches!(value.as_ref(), Expr::Record { explicit: true, fields } if fields.entries.len() == 4)
    ));
    assert_eq!(items[9].expr, Expr::Identifier(ident("zoned_time")));
}

#[test]
fn parses_numeric_value_functions() {
    let program = parse(
        "RETURN ABS(-n.delta) AS magnitude, MOD(n.score, 10) AS bucket, \
         FLOOR(n.ratio) AS floored, CEIL(n.ratio) AS ceiled, \
         CEILING(n.ratio) AS ceilinged, SQRT(n.value) AS root, \
         POWER(n.value, 2) AS squared, LOG(10, n.value) AS log_base, \
         LOG10(n.value) AS common_log, LN(n.value) AS natural_log, \
         EXP(n.value) AS exponential, SIN(n.angle) AS s, COS(n.angle) AS c, \
         TAN(n.angle) AS t, COT(n.angle) AS cot, SINH(n.angle) AS sinh, \
         COSH(n.angle) AS cosh, TANH(n.angle) AS tanh, ASIN(n.ratio) AS asin, \
         ACOS(n.ratio) AS acos, ATAN(n.ratio) AS atan, DEGREES(n.angle) AS degrees, \
         RADIANS(n.degrees) AS radians, PATH_LENGTH(p) AS path_len, abs AS identifier",
    )
    .unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };
    let items = &query.body.result_clause.items;

    let Expr::AbsoluteValue { expr } = &items[0].expr else {
        panic!("expected absolute value function");
    };
    assert!(matches!(expr.as_ref(), Expr::Unary { .. }));

    let expected = [
        (NumericFunctionKind::Mod, 2),
        (NumericFunctionKind::Floor, 1),
        (NumericFunctionKind::Ceil, 1),
        (NumericFunctionKind::Ceil, 1),
        (NumericFunctionKind::Sqrt, 1),
        (NumericFunctionKind::Power, 2),
        (NumericFunctionKind::Log, 2),
        (NumericFunctionKind::Log10, 1),
        (NumericFunctionKind::Ln, 1),
        (NumericFunctionKind::Exp, 1),
        (NumericFunctionKind::Sin, 1),
        (NumericFunctionKind::Cos, 1),
        (NumericFunctionKind::Tan, 1),
        (NumericFunctionKind::Cot, 1),
        (NumericFunctionKind::Sinh, 1),
        (NumericFunctionKind::Cosh, 1),
        (NumericFunctionKind::Tanh, 1),
        (NumericFunctionKind::Asin, 1),
        (NumericFunctionKind::Acos, 1),
        (NumericFunctionKind::Atan, 1),
        (NumericFunctionKind::Degrees, 1),
        (NumericFunctionKind::Radians, 1),
        (NumericFunctionKind::PathLength, 1),
    ];

    for (index, (expected_function, expected_arity)) in expected.iter().enumerate() {
        let Expr::NumericFunction { function, args } = &items[index + 1].expr else {
            panic!("expected numeric function at item {index}");
        };
        assert_eq!(function, expected_function);
        assert_eq!(args.len(), *expected_arity);
    }

    assert_eq!(items[24].expr, Expr::Identifier(ident("abs")));
}

#[test]
fn parses_cast_expressions() {
    let program = parse(
        "RETURN CAST($age AS INTEGER) AS age, \
         CAST(['a', 'b'] AS LIST<STRING>) AS names",
    )
    .unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };

    let Expr::Cast { expr, value_type } = &query.body.result_clause.items[0].expr else {
        panic!("expected cast expression");
    };
    assert_eq!(expr.as_ref(), &Expr::Parameter("age".to_owned()));
    assert_eq!(
        *value_type,
        ValueType::ExactNumeric {
            kind: ExactNumericTypeKind::Integer,
            signed: Some(true),
            precision: None,
            scale: None,
        }
    );

    let Expr::Cast { expr, value_type } = &query.body.result_clause.items[1].expr else {
        panic!("expected cast expression");
    };
    assert!(matches!(expr.as_ref(), Expr::List(_)));
    assert!(matches!(value_type, ValueType::List(_)));
}

#[test]
fn parses_value_query_expression() {
    let program = parse(
        "RETURN VALUE { MATCH (n) RETURN n.name AS name LIMIT 1 } AS name, value AS identifier",
    )
    .unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };

    let Expr::ValueQuery { query: nested } = &query.body.result_clause.items[0].expr else {
        panic!("expected value query expression");
    };
    assert!(matches!(nested.body.clauses[0], QueryClause::Match(_)));
    assert_eq!(
        nested.body.limit,
        Some(UnsignedIntegerSpecification::Literal(1))
    );
    assert!(matches!(
        nested.body.result_clause.items[0].expr,
        Expr::Property { .. }
    ));
    assert_eq!(
        query.body.result_clause.items[1].expr,
        Expr::Identifier(ident("value"))
    );
}

#[test]
fn parses_let_value_expression() {
    let program = parse(
        "RETURN LET x = 1, VALUE y INTEGER = 2 IN x + y END AS total, \
         let AS identifier",
    )
    .unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };

    let Expr::Let { items, value } = &query.body.result_clause.items[0].expr else {
        panic!("expected let value expression");
    };
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].name, ident("x"));
    assert!(!items[0].typed);
    assert_eq!(items[0].value, Expr::Literal(Literal::Integer(1)));
    assert_eq!(items[1].name, ident("y"));
    assert!(items[1].value_type.is_some());
    assert_eq!(items[1].value, Expr::Literal(Literal::Integer(2)));
    assert!(matches!(
        value.as_ref(),
        Expr::Binary {
            op: BinaryOp::Add,
            ..
        }
    ));
    assert_eq!(
        query.body.result_clause.items[1].expr,
        Expr::Identifier(ident("let"))
    );
}

#[test]
fn parses_case_expressions() {
    let program = parse(
        "RETURN CASE WHEN n.age >= 18 THEN 'adult' ELSE 'minor' END AS searched, \
         CASE n.status WHEN 'active' THEN 1 ELSE 0 END AS simple, \
         CASE n.status WHEN 'new', 'open' THEN 'pending' ELSE 'closed' END AS operand_list, \
         CASE n.name WHEN COALESCE($name, 'unknown') THEN 'match' ELSE 'other' END AS primary_operand, \
         CASE n.age WHEN < 18, >= 65 THEN 'edge' ELSE 'middle' END AS comparison_parts, \
         CASE n.name WHEN IS NULL THEN 'missing' ELSE 'present' END AS null_part, \
         CASE r WHEN IS DIRECTED THEN 'directed' WHEN IS LABELED KNOWS THEN 'knows' ELSE 'other' END AS element_parts",
    )
    .unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };

    let searched = &query.body.result_clause.items[0].expr;
    let Expr::Case {
        operand,
        when_clauses,
        else_expr,
    } = searched
    else {
        panic!("expected searched case expression");
    };
    assert!(operand.is_none());
    assert_eq!(when_clauses.len(), 1);
    assert!(matches!(
        when_clauses[0].condition,
        Expr::Binary {
            op: BinaryOp::Ge,
            ..
        }
    ));
    assert!(matches!(
        when_clauses[0].result,
        Expr::Literal(Literal::String(_))
    ));
    assert!(else_expr.is_some());

    let simple = &query.body.result_clause.items[1].expr;
    let Expr::Case {
        operand,
        when_clauses,
        else_expr,
    } = simple
    else {
        panic!("expected simple case expression");
    };
    assert!(matches!(operand.as_deref(), Some(Expr::Property { .. })));
    assert_eq!(when_clauses.len(), 1);
    assert!(matches!(
        when_clauses[0].condition,
        Expr::Literal(Literal::String(_))
    ));
    assert!(when_clauses[0].additional_operands.is_empty());
    assert!(matches!(
        when_clauses[0].result,
        Expr::Literal(Literal::Integer(1))
    ));
    assert!(else_expr.is_some());

    let operand_list = &query.body.result_clause.items[2].expr;
    let Expr::Case {
        operand,
        when_clauses,
        else_expr,
    } = operand_list
    else {
        panic!("expected simple case expression with operand list");
    };
    assert!(matches!(operand.as_deref(), Some(Expr::Property { .. })));
    assert_eq!(when_clauses.len(), 1);
    assert_eq!(
        when_clauses[0].condition,
        Expr::Literal(Literal::String("new".to_owned()))
    );
    assert_eq!(
        when_clauses[0].additional_operands,
        vec![Expr::Literal(Literal::String("open".to_owned()))]
    );
    assert_eq!(
        when_clauses[0].result,
        Expr::Literal(Literal::String("pending".to_owned()))
    );
    assert!(else_expr.is_some());

    let primary_operand = &query.body.result_clause.items[3].expr;
    let Expr::Case {
        operand,
        when_clauses,
        else_expr,
    } = primary_operand
    else {
        panic!("expected simple case expression with primary operand");
    };
    assert!(matches!(operand.as_deref(), Some(Expr::Property { .. })));
    assert!(matches!(when_clauses[0].condition, Expr::Coalesce(_)));
    assert!(else_expr.is_some());

    let comparison_parts = &query.body.result_clause.items[4].expr;
    let Expr::Case {
        operand,
        when_clauses,
        else_expr,
    } = comparison_parts
    else {
        panic!("expected simple case expression with comparison predicate parts");
    };
    assert!(matches!(operand.as_deref(), Some(Expr::Property { .. })));
    assert_eq!(when_clauses.len(), 1);
    assert!(matches!(
        when_clauses[0].condition,
        Expr::Binary {
            op: BinaryOp::Lt,
            ..
        }
    ));
    assert!(matches!(
        when_clauses[0].additional_operands.as_slice(),
        [Expr::Binary {
            op: BinaryOp::Ge,
            ..
        }]
    ));
    assert!(else_expr.is_some());

    let null_part = &query.body.result_clause.items[5].expr;
    let Expr::Case {
        operand,
        when_clauses,
        ..
    } = null_part
    else {
        panic!("expected simple case expression with null predicate part");
    };
    assert!(matches!(operand.as_deref(), Some(Expr::Property { .. })));
    assert!(matches!(
        when_clauses[0].condition,
        Expr::IsNull { negated: false, .. }
    ));

    let element_parts = &query.body.result_clause.items[6].expr;
    let Expr::Case {
        operand,
        when_clauses,
        else_expr,
    } = element_parts
    else {
        panic!("expected simple case expression with element predicate parts");
    };
    assert_eq!(operand.as_deref(), Some(&Expr::Identifier(ident("r"))));
    assert_eq!(when_clauses.len(), 2);
    assert_eq!(
        when_clauses[0].condition,
        Expr::IsDirected {
            variable: ident("r"),
            negated: false
        }
    );
    assert_eq!(
        when_clauses[1].condition,
        Expr::IsLabeled {
            variable: ident("r"),
            negated: false,
            label_expression: LabelExpression::Label(ident("KNOWS"))
        }
    );
    assert!(else_expr.is_some());
}

#[test]
fn parses_case_abbreviation_expressions() {
    let program = parse(
        "RETURN COALESCE(n.name, 'unknown', $fallback) AS name, \
         NULLIF(n.status, 'deleted') AS status, nullif AS identifier",
    )
    .unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };

    let Expr::Coalesce(args) = &query.body.result_clause.items[0].expr else {
        panic!("expected coalesce expression");
    };
    assert_eq!(args.len(), 3);
    assert!(matches!(args[0], Expr::Property { .. }));
    assert_eq!(
        args[1],
        Expr::Literal(Literal::String("unknown".to_owned()))
    );
    assert_eq!(args[2], Expr::Parameter("fallback".to_owned()));

    let Expr::NullIf { left, right } = &query.body.result_clause.items[1].expr else {
        panic!("expected nullif expression");
    };
    assert!(matches!(left.as_ref(), Expr::Property { .. }));
    assert_eq!(
        right.as_ref(),
        &Expr::Literal(Literal::String("deleted".to_owned()))
    );

    assert_eq!(
        query.body.result_clause.items[2].expr,
        Expr::Identifier(ident("nullif"))
    );
}

#[test]
fn parses_standard_comparison_and_string_concat_operators() {
    let program =
        parse("RETURN 'hello' || ' ' || 'world' AS greeting, 1 <> 2 AS different").unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };

    let concat = &query.body.result_clause.items[0].expr;
    assert!(matches!(
        concat,
        Expr::Binary {
            op: BinaryOp::Concat,
            left,
            right,
        } if matches!(left.as_ref(), Expr::Binary { op: BinaryOp::Concat, .. })
            && matches!(right.as_ref(), Expr::Literal(Literal::String(value)) if value == "world")
    ));

    assert!(matches!(
        query.body.result_clause.items[1].expr,
        Expr::Binary {
            op: BinaryOp::Neq,
            ..
        }
    ));
}

#[test]
fn parses_standard_string_value_functions() {
    let program = parse(
        "RETURN LEFT(name, 2) AS prefix, \
         RIGHT(name, 3) AS suffix, \
         TRIM(BOTH 'x' FROM 'xxnamexx') AS trimmed, \
         TRIM(TRAILING FROM name) AS trailing_trimmed, \
         TRIM(name) AS simple_trim, \
         BTRIM(name, 'xy') AS both_trimmed, \
         LTRIM(name) AS left_trimmed, \
         RTRIM(name, 'z') AS right_trimmed, \
         trim AS trim_identifier, \
         btrim AS btrim_identifier, \
         left AS left_identifier, \
         right AS right_identifier, \
         position AS position_identifier, \
         overlay AS overlay_identifier, \
         placing AS placing_identifier",
    )
    .unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };
    let items = &query.body.result_clause.items;

    let Expr::SubstringByLength {
        side,
        value,
        length,
    } = &items[0].expr
    else {
        panic!("expected left substring expression");
    };
    assert_eq!(*side, SubstringSide::Left);
    assert_eq!(value.as_ref(), &Expr::Identifier(ident("name")));
    assert_eq!(length.as_ref(), &Expr::Literal(Literal::Integer(2)));

    let Expr::SubstringByLength {
        side,
        value,
        length,
    } = &items[1].expr
    else {
        panic!("expected right substring expression");
    };
    assert_eq!(*side, SubstringSide::Right);
    assert_eq!(value.as_ref(), &Expr::Identifier(ident("name")));
    assert_eq!(length.as_ref(), &Expr::Literal(Literal::Integer(3)));

    let Expr::Trim {
        specification,
        trim_character,
        value,
    } = &items[2].expr
    else {
        panic!("expected trim expression");
    };
    assert_eq!(*specification, Some(TrimSpec::Both));
    assert_eq!(
        trim_character.as_deref(),
        Some(&Expr::Literal(Literal::String("x".to_owned())))
    );
    assert_eq!(
        value.as_ref(),
        &Expr::Literal(Literal::String("xxnamexx".to_owned()))
    );

    assert!(matches!(
        items[3].expr,
        Expr::Trim {
            specification: Some(TrimSpec::Trailing),
            trim_character: None,
            ..
        }
    ));
    assert!(matches!(
        items[4].expr,
        Expr::Trim {
            specification: None,
            trim_character: None,
            ..
        }
    ));

    let Expr::MultiTrim {
        specification,
        value,
        trim_characters,
    } = &items[5].expr
    else {
        panic!("expected btrim expression");
    };
    assert_eq!(*specification, TrimSpec::Both);
    assert_eq!(value.as_ref(), &Expr::Identifier(ident("name")));
    assert_eq!(
        trim_characters.as_deref(),
        Some(&Expr::Literal(Literal::String("xy".to_owned())))
    );
    assert!(matches!(
        items[6].expr,
        Expr::MultiTrim {
            specification: TrimSpec::Leading,
            trim_characters: None,
            ..
        }
    ));
    assert!(matches!(
        items[7].expr,
        Expr::MultiTrim {
            specification: TrimSpec::Trailing,
            trim_characters: Some(_),
            ..
        }
    ));
    assert_eq!(items[8].expr, Expr::Identifier(ident("trim")));
    assert_eq!(items[9].expr, Expr::Identifier(ident("btrim")));
    assert_eq!(items[10].expr, Expr::Identifier(ident("left")));
    assert_eq!(items[11].expr, Expr::Identifier(ident("right")));
    assert_eq!(items[12].expr, Expr::Identifier(ident("position")));
    assert_eq!(items[13].expr, Expr::Identifier(ident("overlay")));
    assert_eq!(items[14].expr, Expr::Identifier(ident("placing")));
}

#[test]
fn parses_standard_string_fold_normalize_and_length_functions() {
    let program = parse(
        "RETURN UPPER(n.name) AS upper_name, LOWER(n.name) AS lower_name, \
         NORMALIZE(n.name, NFC) AS normalized, NORMALIZE(n.name, NFD) AS comma_normalized, \
         NORMALIZE(n.name) AS default_normalized, \
         CHAR_LENGTH(n.name) AS chars, CHARACTER_LENGTH(n.name) AS characters, \
         BYTE_LENGTH(n.raw) AS bytes, OCTET_LENGTH(n.raw) AS octets, normalize AS identifier",
    )
    .unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };
    let items = &query.body.result_clause.items;

    let Expr::Fold { upper, value } = &items[0].expr else {
        panic!("expected upper fold expression");
    };
    assert!(*upper);
    assert!(matches!(value.as_ref(), Expr::Property { .. }));

    let Expr::Fold { upper, value } = &items[1].expr else {
        panic!("expected lower fold expression");
    };
    assert!(!*upper);
    assert!(matches!(value.as_ref(), Expr::Property { .. }));

    let Expr::Normalize { value, normal_form } = &items[2].expr else {
        panic!("expected normalize expression");
    };
    assert!(matches!(value.as_ref(), Expr::Property { .. }));
    assert_eq!(normal_form.as_ref(), Some(&ident("NFC")));

    let Expr::Normalize { normal_form, .. } = &items[3].expr else {
        panic!("expected comma normalize expression");
    };
    assert_eq!(normal_form.as_ref(), Some(&ident("NFD")));

    let Expr::Normalize { normal_form, .. } = &items[4].expr else {
        panic!("expected normalize expression");
    };
    assert!(normal_form.is_none());

    let Expr::StringLength { unit, value } = &items[5].expr else {
        panic!("expected char length expression");
    };
    assert_eq!(*unit, StringLengthUnit::Characters);
    assert!(matches!(value.as_ref(), Expr::Property { .. }));

    let Expr::StringLength { unit, value } = &items[6].expr else {
        panic!("expected character length expression");
    };
    assert_eq!(*unit, StringLengthUnit::Characters);
    assert!(matches!(value.as_ref(), Expr::Property { .. }));

    let Expr::StringLength { unit, value } = &items[7].expr else {
        panic!("expected byte length expression");
    };
    assert_eq!(*unit, StringLengthUnit::Bytes);
    assert!(matches!(value.as_ref(), Expr::Property { .. }));

    let Expr::StringLength { unit, value } = &items[8].expr else {
        panic!("expected octet length expression");
    };
    assert_eq!(*unit, StringLengthUnit::Bytes);
    assert!(matches!(value.as_ref(), Expr::Property { .. }));

    assert_eq!(items[9].expr, Expr::Identifier(ident("normalize")));
}

#[test]
fn parses_list_value_functions() {
    let program = parse(
        "MATCH p = (a)-[e]->(b) \
         RETURN TRIM([1, 2, 3], 1) AS trimmed, ELEMENTS(p) AS path_elements, \
         elements AS identifier",
    )
    .unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };
    let items = &query.body.result_clause.items;

    let Expr::TrimList { value, count } = &items[0].expr else {
        panic!("expected list trim function");
    };
    assert!(matches!(value.as_ref(), Expr::List(values) if values.len() == 3));
    assert_eq!(count.as_ref(), &Expr::Literal(Literal::Integer(1)));

    let Expr::Elements { path } = &items[1].expr else {
        panic!("expected elements function");
    };
    assert_eq!(path.as_ref(), &Expr::Identifier(ident("p")));

    assert_eq!(items[2].expr, Expr::Identifier(ident("elements")));
}

#[test]
fn parses_typed_list_value_constructors() {
    let program = parse(
        "RETURN LIST [1, 2, 3] AS numbers, \
         ARRAY ['a', 'b'] AS names, \
         LIST [] AS empty_list, \
         list AS identifier, array AS array_identifier, group AS group_identifier",
    )
    .unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };
    let items = &query.body.result_clause.items;

    assert!(matches!(
        items[0].expr,
        Expr::TypedList {
            type_name: ListValueTypeName::List,
            ref values,
        } if values.len() == 3
    ));
    assert!(matches!(
        items[1].expr,
        Expr::TypedList {
            type_name: ListValueTypeName::Array,
            ref values,
        } if values.len() == 2
    ));
    assert_eq!(
        items[2].expr,
        Expr::TypedList {
            type_name: ListValueTypeName::List,
            values: Vec::new(),
        }
    );
    assert_eq!(items[3].expr, Expr::Identifier(ident("list")));
    assert_eq!(items[4].expr, Expr::Identifier(ident("array")));
    assert_eq!(items[5].expr, Expr::Identifier(ident("group")));
}

#[test]
fn parses_path_value_constructor() {
    let program =
        parse("RETURN PATH [n, e, m, e2, x] AS p, PATH [solo] AS single, path AS identifier")
            .unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };
    let items = &query.body.result_clause.items;

    let Expr::Path(values) = &items[0].expr else {
        panic!("expected path value constructor");
    };
    assert_eq!(
        values,
        &vec![
            Expr::Identifier(ident("n")),
            Expr::Identifier(ident("e")),
            Expr::Identifier(ident("m")),
            Expr::Identifier(ident("e2")),
            Expr::Identifier(ident("x")),
        ]
    );
    assert!(matches!(&items[1].expr, Expr::Path(values) if values.len() == 1));
    assert_eq!(items[2].expr, Expr::Identifier(ident("path")));
}

#[test]
fn parses_record_value_constructors() {
    let program = parse(
        "RETURN RECORD {name: n.name, age: n.age} AS person, \
         RECORD {} AS empty_record, {nested: RECORD {active: true}} AS nested",
    )
    .unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };
    let items = &query.body.result_clause.items;

    let Expr::Record { explicit, fields } = &items[0].expr else {
        panic!("expected explicit record constructor");
    };
    assert!(*explicit);
    assert_eq!(fields.entries.len(), 2);
    assert_eq!(fields.entries[0].0, ident("name"));
    assert!(matches!(fields.entries[0].1, Expr::Property { .. }));

    let Expr::Record { explicit, fields } = &items[1].expr else {
        panic!("expected empty record constructor");
    };
    assert!(*explicit);
    assert!(fields.entries.is_empty());

    let Expr::Record { explicit, fields } = &items[2].expr else {
        panic!("expected bare record constructor");
    };
    assert!(!*explicit);
    assert_eq!(fields.entries.len(), 1);
    assert!(matches!(
        fields.entries[0].1,
        Expr::Record { explicit: true, .. }
    ));
}

#[test]
fn preserves_expression_precedence() {
    let program = parse(
        "MATCH (n) RETURN 1 + 2 * 3 > 6 AND NOT false AS ok, \
         true OR false XOR unknown AS left_xor, \
         true XOR false OR unknown AS left_or",
    )
    .unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query statement");
    };

    assert_eq!(
        query.body.result_clause.items[0].expr,
        Expr::Binary {
            left: Box::new(Expr::Binary {
                left: Box::new(Expr::Binary {
                    left: Box::new(Expr::Literal(Literal::Integer(1))),
                    op: BinaryOp::Add,
                    right: Box::new(Expr::Binary {
                        left: Box::new(Expr::Literal(Literal::Integer(2))),
                        op: BinaryOp::Mul,
                        right: Box::new(Expr::Literal(Literal::Integer(3))),
                    }),
                }),
                op: BinaryOp::Gt,
                right: Box::new(Expr::Literal(Literal::Integer(6))),
            }),
            op: BinaryOp::And,
            right: Box::new(Expr::Unary {
                op: UnaryOp::Not,
                expr: Box::new(Expr::Literal(Literal::Boolean(false))),
            }),
        }
    );

    let Expr::Binary {
        left,
        op: BinaryOp::Xor,
        ..
    } = &query.body.result_clause.items[1].expr
    else {
        panic!("expected OR/XOR to be left-associative");
    };
    assert!(matches!(
        left.as_ref(),
        Expr::Binary {
            op: BinaryOp::Or,
            ..
        }
    ));

    let Expr::Binary {
        left,
        op: BinaryOp::Or,
        ..
    } = &query.body.result_clause.items[2].expr
    else {
        panic!("expected XOR/OR to be left-associative");
    };
    assert!(matches!(
        left.as_ref(),
        Expr::Binary {
            op: BinaryOp::Xor,
            ..
        }
    ));
}

#[test]
fn nested_query_primary_inherits_results_but_explicit_finish_does_not() {
    let program = parse("{ RETURN 1 AS x }; { RETURN 1 AS x } FINISH").unwrap();
    let Statement::Query(inherited) = &program.statements[0] else {
        panic!()
    };
    assert_eq!(inherited.body.result_clause.kind, ResultKind::Return);
    assert!(matches!(
        inherited.body.result_clause.items[0].expr,
        Expr::Wildcard
    ));
    let Statement::Query(finished) = &program.statements[1] else {
        panic!()
    };
    assert_eq!(finished.body.result_clause.kind, ResultKind::Finish);
}

#[test]
fn leading_parenthesized_paths_allow_following_factors_and_alternatives() {
    let program = parse("MATCH p = ((a)->(b)){1,2}->(c) | ((x)->(y)) RETURN p").unwrap();
    let Statement::Query(query) = &program.statements[0] else {
        panic!("query");
    };
    let pattern = &first_match(query).patterns[0];
    assert!(pattern.parenthesized.is_none());
    assert_eq!(pattern.factors.len(), 3);
    assert!(matches!(
        pattern.factors[0],
        PathPatternFactor::Parenthesized(_)
    ));
    assert_eq!(pattern.alternation, Some(PathPatternAlternation::Union));
    assert_eq!(pattern.alternatives.len(), 1);
    assert!(matches!(
        pattern.alternatives[0].factors[0],
        PathPatternFactor::Parenthesized(_)
    ));
    for invalid in [
        "MATCH (TRAIL ANY SHORTEST (a)->(b)) RETURN a",
        "MATCH (p = q = (a)->(b)) RETURN p",
    ] {
        assert!(parse(invalid).is_err(), "{invalid}");
    }
}
