use graphfusion_gql_parser::parse;

// Register each fixture separately so Cargo can filter it and report every failure.
macro_rules! valid_cases {
    ($($name:ident { input: $input:expr, statement_count: $expected:expr, })*) => {
        $(
            #[test]
            fn $name() {
                let input = $input;
                let program = parse(input)
                    .unwrap_or_else(|err| panic!("failed to parse {input:?}: {err}"));
                assert_eq!(program.statements.len(), $expected, "input: {input:?}");
            }
        )*
    };
}

valid_cases! {
    basic_match_return {
        input: "MATCH (n) RETURN n",
        statement_count: 1,
    }

    delimited_identifiers {
        input: "MATCH (\"select\":`Person Label` {\"display name\": 'Alice'}) RETURN \"select\".\"display name\" AS `display alias`",
        statement_count: 1,
    }

    match_yield_clause {
        input: "MATCH (n)-[e]->(m) WHERE n.active = true YIELD n, e RETURN n",
        statement_count: 1,
    }

    use_graph_optional_match {
        input: "USE GRAPH social OPTIONAL MATCH (a)<-[e:LIKES]-(b) RETURN a, b",
        statement_count: 1,
    }

    focused_linear_query_use_graph_clauses {
        input: "USE GRAPH social MATCH (a) USE GRAPH archive MATCH (b) RETURN a, b UNION USE GRAPH reports MATCH (r) RETURN r",
        statement_count: 1,
    }

    standard_use_graph_clauses {
        input: "USE social MATCH (n) RETURN n; RETURN 1 AS value UNION USE reports MATCH (r) RETURN r",
        statement_count: 2,
    }

    optional_match_statement_block {
        input: "OPTIONAL { MATCH (a)-[:KNOWS]->(b) OPTIONAL MATCH (b)-[:LIKES]->(c) } RETURN a, b, c",
        statement_count: 1,
    }

    current_and_home_graph_expressions {
        input: "USE PROPERTY GRAPH CURRENT_PROPERTY_GRAPH MATCH (n) RETURN n; SELECT n FROM HOME_GRAPH MATCH (n); SESSION SET PROPERTY GRAPH HOME_PROPERTY_GRAPH",
        statement_count: 3,
    }

    graph_reference_value_expressions {
        input: "RETURN GRAPH social AS g, PROPERTY GRAPH CURRENT_PROPERTY_GRAPH AS pg, graph AS identifier",
        statement_count: 1,
    }

    binding_table_reference_value_expressions {
        input: "RETURN TABLE $rows AS t, BINDING TABLE app.rows AS bt, table AS identifier, binding AS binding_identifier",
        statement_count: 1,
    }

    parameterized_graph_expressions {
        input: "USE GRAPH $active MATCH (n) RETURN n; SELECT n FROM $report MATCH (n); SESSION SET GRAPH $next",
        statement_count: 3,
    }

    delimited_and_extended_parameter_names {
        input: "USE GRAPH $\"active graph\" MATCH (n) RETURN $@\"raw\\nname\" AS raw, $123 AS ordinal",
        statement_count: 1,
    }

    schema_references {
        input: "AT SCHEMA CURRENT_SCHEMA MATCH (n) RETURN n; AT SCHEMA $tenant MATCH (n) RETURN n; SESSION SET SCHEMA HOME_SCHEMA",
        statement_count: 3,
    }

    match_modes {
        input: "MATCH REPEATABLE ELEMENTS (a)-[:KNOWS]->(b) RETURN a; MATCH DIFFERENT EDGE BINDINGS (a)-[e]->(b) RETURN e",
        statement_count: 2,
    }

    path_variable_declaration {
        input: "MATCH path = (a)-[:KNOWS]->(b), other = (b)-[:KNOWS]->(c) RETURN path, other",
        statement_count: 1,
    }

    path_mode_prefixes {
        input: "MATCH WALK (a)-[:KNOWS]->(b), TRAIL PATHS (b)-[:KNOWS]->(c) RETURN a",
        statement_count: 1,
    }

    path_search_prefixes {
        input: "MATCH ANY (a)-[:KNOWS]->(b), ALL SHORTEST PATHS (b)-[:KNOWS]->(c), SHORTEST 2 (c)-[:KNOWS]->(d), SHORTEST 2 TRAIL GROUPS (d)-[:KNOWS]->(e) RETURN a",
        statement_count: 1,
    }

    linear_catalog_modifying_statement {
        input: "CREATE GRAPH demo.temp ANY GRAPH DROP GRAPH demo.temp CREATE GRAPH demo.copy ANY GRAPH AS COPY OF demo.source",
        statement_count: 1,
    }

    call_prefixed_linear_catalog_modifying_statement {
        input: "CALL db.refresh() CREATE GRAPH demo.refreshed ANY GRAPH",
        statement_count: 1,
    }

    path_pattern_union {
        input: "MATCH (a)-[:KNOWS]->(b) | (a)-[:LIKES]->(b) RETURN a, b",
        statement_count: 1,
    }

    linear_query_clauses {
        input: "MATCH (n:Person) FILTER n.age > 21 LET decade = n.age / 10 RETURN decade",
        statement_count: 1,
    }

    typed_let_variable_definition {
        input: "MATCH (n) LET VALUE score :: TYPED INTEGER = n.score, label = n.name RETURN score, label",
        statement_count: 1,
    }

    filter_where_clause {
        input: "MATCH (n:Person) FILTER WHERE n.age > 21 RETURN n",
        statement_count: 1,
    }

    call_yield_for_query {
        input: "CALL graph.expand($start) YIELD node AS n FOR score IN [1, 2, 3] RETURN n, score",
        statement_count: 1,
    }

    inline_procedure_call {
        input: "CALL (n) { MATCH (n)-[:KNOWS]->(m) RETURN m } RETURN m",
        statement_count: 1,
    }

    inline_procedure_body_at_schema_with_definitions {
        input: "CALL { AT SCHEMA app.main VALUE fallback STRING = 'unknown' RETURN fallback } RETURN fallback",
        statement_count: 1,
    }

    optional_named_procedure_call {
        input: "OPTIONAL CALL graph.expand($start) YIELD node AS n RETURN n",
        statement_count: 1,
    }

    for_with_ordinality {
        input: "FOR item IN [10, 20, 30] WITH ORDINALITY ord RETURN item, ord",
        statement_count: 1,
    }

    let_return_star {
        input: "LET x = 1 RETURN *",
        statement_count: 1,
    }

    select_group_order_page {
        input: "MATCH (p:Person) SELECT p.city AS city, count(*) AS total GROUP BY city ORDER BY total DESC OFFSET 5 LIMIT 10",
        statement_count: 1,
    }

    aggregate_set_quantifiers {
        input: "MATCH (p:Person) SELECT count(DISTINCT p.city) AS cities, sum(ALL p.score) AS score",
        statement_count: 1,
    }

    standard_aggregate_function_names {
        input: "MATCH (p:Person) RETURN COUNT(*) AS total, AVG(p.score) AS avg_score, COLLECT_LIST(DISTINCT p.name) AS names, STDDEV_POP(p.score) AS spread, PERCENTILE_CONT(DISTINCT p.score, 0.5) AS median",
        statement_count: 1,
    }

    order_and_offset_synonyms {
        input: "RETURN n ORDER BY n.name ASCENDING, n.score DESCENDING SKIP 5 LIMIT 10",
        statement_count: 1,
    }

    linear_order_by_page_before_result {
        input: "MATCH (n) ORDER BY n.name ASC SKIP 5 LIMIT 10 RETURN n",
        statement_count: 1,
    }

    order_by_page_as_primitive_query_statement {
        input: "ORDER BY n.name ASC LIMIT 10 RETURN n",
        statement_count: 1,
    }

    select_group_having {
        input: "MATCH (p:Person) SELECT p.city AS city, count(*) AS total GROUP BY city HAVING total > 1 ORDER BY total DESC",
        statement_count: 1,
    }

    empty_grouping_set {
        input: "MATCH (p:Person) SELECT count(*) AS total GROUP BY () HAVING total > 0",
        statement_count: 1,
    }

    select_from_graph_match {
        input: "SELECT n.name AS name FROM social MATCH (n:Person) WHERE n.active = true ORDER BY name",
        statement_count: 1,
    }

    select_from_nested_query {
        input: "SELECT name FROM { MATCH (n:Person) RETURN n.name AS name LIMIT 5 } WHERE name IS NOT NULL",
        statement_count: 1,
    }

    select_from_graph_nested_query {
        input: "SELECT name FROM social { MATCH (n:Person) RETURN n.name AS name LIMIT 5 }",
        statement_count: 1,
    }

    linear_nested_query_specifications {
        input: "{ MATCH (n) RETURN n }; USE social { MATCH (n) RETURN n }; RETURN 1 AS value UNION USE reports { MATCH (r) RETURN r }",
        statement_count: 3,
    }

    catalog_and_transaction_batch {
        input: "CREATE SCHEMA IF NOT EXISTS demo; CREATE GRAPH IF NOT EXISTS demo.graph ANY GRAPH; START TRANSACTION READ ONLY; COMMIT;",
        statement_count: 4,
    }

    create_graph_type_and_source_forms {
        input: "CREATE PROPERTY GRAPH IF NOT EXISTS social ANY PROPERTY GRAPH AS COPY OF HOME_GRAPH; \
                    CREATE OR REPLACE GRAPH reports LIKE HOME_GRAPH AS COPY OF $seed; \
                    CREATE GRAPH typed :: TYPED app.social_type; \
                    CREATE GRAPH nested GRAPH { NODE Person {name STRING} }",
        statement_count: 4,
    }

    graph_type_with_value_types {
        input: "CREATE GRAPH TYPE social AS { NODE Person {name STRING, tags LIST<STRING>, meta RECORD {score INTEGER}}, DIRECTED EDGE Knows FROM Person TO Person {weights ARRAY<INTEGER>} }",
        statement_count: 1,
    }

    anonymous_graph_type_patterns {
        input: "CREATE GRAPH TYPE anonymous AS { (:Person {name STRING}), (:Person)-[:KNOWS {since INTEGER}]->(:Company) }",
        statement_count: 1,
    }

    abbreviated_graph_type_edge_patterns {
        input: "CREATE GRAPH TYPE abbreviated AS { NODE Person, NODE Company, (Person)->(Company), (Company)<-(Person), (Person)~(Person), DIRECTED EDGE Visits CONNECTING (Person)->(Company) }",
        statement_count: 1,
    }

    field_type_typed_markers {
        input: "CREATE GRAPH TYPE typed_fields AS { NODE Person {name TYPED STRING, age :: INTEGER, meta RECORD {score TYPED INTEGER, tags :: LIST<STRING>}} }",
        statement_count: 1,
    }

    not_null_value_types {
        input: "CREATE GRAPH TYPE strict AS { NODE Person {name STRING NOT NULL, tags LIST<STRING NOT NULL> NOT NULL} }; \
                    RETURN CAST($age AS INTEGER NOT NULL) AS age; \
                    MATCH (n) WHERE n.tags IS TYPED LIST<STRING> NOT NULL RETURN n",
        statement_count: 3,
    }

    list_value_type_postfix_and_max_length {
        input: "CREATE GRAPH TYPE sized AS { NODE Person {aliases STRING LIST[5], scores ARRAY<INTEGER>[3], tags STRING NOT NULL LIST NOT NULL} }; \
                    RETURN CAST($names AS STRING LIST) AS names; \
                    MATCH (n) WHERE n.scores IS TYPED INTEGER ARRAY[3] RETURN n",
        statement_count: 3,
    }

    path_and_list_value_types {
        input: "CREATE GRAPH TYPE paths AS { NODE Route {route PATH, nodes LIST<NODE>[4], edges EDGE ARRAY NOT NULL} }; \
                    RETURN CAST($path AS PATH NOT NULL) AS path; \
                    MATCH (n) WHERE n.nodes IS TYPED LIST<NODE> RETURN n",
        statement_count: 3,
    }

    record_value_type_forms {
        input: "CREATE GRAPH TYPE records AS { NODE Person {open RECORD, any_open ANY RECORD NOT NULL, closed {score INTEGER}, explicit RECORD {score TYPED INTEGER}} }; \
                    RETURN CAST($payload AS {name STRING, tags STRING LIST}) AS payload; \
                    MATCH (n) WHERE n.meta IS TYPED RECORD NOT NULL RETURN n",
        statement_count: 3,
    }

    character_string_value_types {
        input: "CREATE GRAPH TYPE strings AS { NODE Person {name STRING, handle VARCHAR(32), required STRING(255) NOT NULL} }; \
                    RETURN CAST($name AS VARCHAR(64)) AS name; \
                    MATCH (n) WHERE n.name IS TYPED STRING(128) RETURN n",
        statement_count: 3,
    }

    boolean_value_types {
        input: "CREATE GRAPH TYPE flags AS { NODE Feature {enabled BOOL, visible BOOLEAN NOT NULL} }; \
                    RETURN CAST($enabled AS BOOLEAN) AS enabled; \
                    MATCH (n) WHERE n.enabled IS TYPED BOOL RETURN n",
        statement_count: 3,
    }

    temporal_value_types {
        input: "CREATE GRAPH TYPE events AS { NODE Event {created ZONED DATETIME, observed TIMESTAMP WITH TIME ZONE, local_created LOCAL DATETIME, stored TIMESTAMP WITHOUT TIME ZONE, day DATE, zoned_at ZONED TIME, local_at TIME WITHOUT TIME ZONE, elapsed DURATION NOT NULL} }; \
                    RETURN CAST($created AS TIMESTAMP) AS created; \
                    MATCH (n) WHERE n.local_at IS TYPED LOCAL TIME RETURN n",
        statement_count: 3,
    }

    dynamic_union_value_types {
        input: "CREATE GRAPH TYPE dynamic AS { NODE Item {payload ANY, required ANY VALUE NOT NULL, prop PROPERTY VALUE, any_prop ANY PROPERTY VALUE NOT NULL, unioned STRING | INTEGER, required_union STRING NOT NULL | INTEGER NOT NULL, closed ANY VALUE<STRING | INTEGER>, required_closed ANY VALUE<STRING NOT NULL | INTEGER NOT NULL>, nested LIST<STRING | INTEGER>} }; \
                    RETURN CAST($payload AS ANY<STRING | INTEGER>) AS payload; \
                    RETURN CAST($required AS ANY<STRING NOT NULL | INTEGER NOT NULL>) AS required; \
                    MATCH (n) WHERE n.payload IS TYPED PROPERTY VALUE RETURN n",
        statement_count: 4,
    }

    reference_value_types {
        input: "CREATE GRAPH TYPE refs AS { NODE Holder {g ANY GRAPH, pg ANY PROPERTY GRAPH NOT NULL, closed_graph GRAPH { NODE Person {name STRING} }, n ANY NODE, cn NODE Person {name STRING}, e ANY EDGE, ce (:Person)-[:KNOWS {since INTEGER}]->(:Person)} }; \
                    RETURN CAST($g AS PROPERTY GRAPH { NODE City {name STRING} }) AS g; \
                    MATCH (n) WHERE n IS TYPED ANY NODE RETURN n",
        statement_count: 3,
    }

    binding_table_reference_value_types {
        input: "CREATE GRAPH TYPE table_refs AS { NODE Holder {rows TABLE {id INTEGER, name STRING}, bindings BINDING TABLE {node ANY NODE, score INTEGER} NOT NULL} }; \
                    RETURN CAST($rows AS TABLE {id INTEGER}) AS rows; \
                    MATCH (n) WHERE $rows IS TYPED BINDING TABLE {node ANY NODE} RETURN n",
        statement_count: 3,
    }

    byte_string_value_types {
        input: "CREATE GRAPH TYPE bytes AS { NODE Blob {raw BYTES, bounded BYTES(2, 16), digest BINARY(32), chunk VARBINARY(1024) NOT NULL} }; \
                    RETURN CAST($raw AS BYTES(16)) AS raw; \
                    MATCH (n) WHERE n.raw IS TYPED VARBINARY(512) RETURN n",
        statement_count: 3,
    }

    precision_scale_numeric_value_types {
        input: "CREATE GRAPH TYPE numbers AS { NODE Measurement {price DECIMAL(12, 2), ratio DEC(10), estimate FLOAT(24, 4), score FLOAT NOT NULL} }; \
                    RETURN CAST($price AS DECIMAL(8, 2)) AS price; \
                    MATCH (n) WHERE n.score IS TYPED FLOAT(32) RETURN n",
        statement_count: 3,
    }

    approximate_numeric_value_types {
        input: "CREATE GRAPH TYPE measurements AS { NODE Measurement {half FLOAT16, precise FLOAT256, real_value REAL, double_value DOUBLE PRECISION NOT NULL} }; \
                    RETURN CAST($score AS DOUBLE) AS score; \
                    MATCH (n) WHERE n.score IS TYPED FLOAT64 RETURN n",
        statement_count: 3,
    }

    binary_exact_numeric_value_types {
        input: "CREATE GRAPH TYPE ints AS { NODE Measurement {tiny INT8, unsigned16 UINT16, regular UNSIGNED INTEGER(32), small SIGNED SMALL INTEGER, big BIGINT NOT NULL} }; \
                    RETURN CAST($value AS UNSIGNED BIG INTEGER) AS value; \
                    MATCH (n) WHERE n.value IS TYPED INT(64) RETURN n",
        statement_count: 3,
    }

    multi_word_value_types {
        input: "CREATE GRAPH TYPE typed AS { NODE Measurement {value DOUBLE PRECISION, label CHARACTER VARYING(40), captured_at TIMESTAMP WITH TIME ZONE} }; \
                    RETURN CAST($started AS TIMESTAMP WITHOUT TIME ZONE) AS started; \
                    MATCH (n) WHERE n.duration IS TYPED TIME WITH TIME ZONE RETURN n",
        statement_count: 3,
    }

    graph_type_source_forms {
        input: "CREATE OR REPLACE PROPERTY GRAPH TYPE copied AS COPY OF app.base_type; \
                    CREATE GRAPH TYPE derived LIKE HOME_GRAPH; \
                    CREATE GRAPH TYPE external AS COPY OF 'https://example.com/types/social#g'; \
                    CREATE GRAPH TYPE nested { NODE Person {name STRING} }",
        statement_count: 4,
    }

    data_modifying_batch {
        input: "INSERT (:Person {name: $name}); SET n.age = 42, m = {name: 'Alice'}, n IS Active; REMOVE n:Temporary, n IS Active, n.old; DETACH DELETE n; NODETACH DELETE e;",
        statement_count: 5,
    }

    linear_data_modifying_statement_starts_with_insert {
        input: "INSERT (:Seen) SET n.flag = true RETURN n",
        statement_count: 1,
    }

    insert_can_repeat_node_variable_without_label_or_property_set {
        input: "INSERT (n:Person)-[:KNOWS]->(n)",
        statement_count: 1,
    }

    next_statement_chain {
        input: "RETURN 1 AS n NEXT YIELD n AS m RETURN m",
        statement_count: 2,
    }

    standalone_optional_call_statement {
        input: "OPTIONAL CALL db.refresh('social')",
        statement_count: 1,
    }

    binding_variable_definition_block {
        input: "VALUE limit INTEGER = 10 GRAPH work = HOME_GRAPH BINDING TABLE rows = app.rows RETURN limit",
        statement_count: 1,
    }

    session_context_commands {
        input: "SESSION SET SCHEMA app.main; SESSION SET GRAPH social; SESSION RESET GRAPH; SESSION CLOSE;",
        statement_count: 4,
    }

    standard_session_commands {
        input: "SESSION SET SCHEMA CURRENT_SCHEMA; SESSION SET TIME ZONE 'UTC'; SESSION SET PROPERTY GRAPH HOME_PROPERTY_GRAPH; SESSION RESET; SESSION RESET ALL PARAMETERS; SESSION RESET TIME ZONE; SESSION RESET PARAMETER $limit; SESSION CLOSE",
        statement_count: 8,
    }

    session_value_parameter_commands {
        input: "SESSION SET VALUE IF NOT EXISTS $limit INTEGER = 10; SESSION SET VALUE timezone = 'UTC'; SESSION SET VALUE $flag = true",
        statement_count: 3,
    }

    session_graph_and_binding_table_parameter_commands {
        input: "SESSION SET GRAPH $active = HOME_GRAPH; SESSION SET PROPERTY GRAPH IF NOT EXISTS $pg ANY PROPERTY GRAPH = HOME_PROPERTY_GRAPH; SESSION SET BINDING TABLE $rows BINDING TABLE {id INTEGER} = app.rows",
        statement_count: 3,
    }

    standalone_call {
        input: "CALL db.refresh('social')",
        statement_count: 1,
    }

    value_type_predicate {
        input: "MATCH (n) WHERE n.tags IS TYPED LIST<STRING> AND n.payload IS NOT TYPED RECORD {score INTEGER} AND n.age IS :: INTEGER RETURN n",
        statement_count: 1,
    }

    chunked_character_string_literal {
        input: "RETURN 'hello '\n'world' AS greeting",
        statement_count: 1,
    }

    double_quoted_character_string_literal {
        input: "RETURN \"hello \"\"world\"\"\" AS greeting",
        statement_count: 1,
    }

    typed_temporal_literals {
        input: "SELECT DATE '2026-06-29' AS d, TIME '12:30:00' AS t, DATETIME '2026-06-29T12:30:00' AS dt, TIMESTAMP '2026-06-29T12:30:00' AS ts, DURATION 'P1D' AS dur, INTERVAL '1' DAY AS one_day",
        statement_count: 1,
    }

    sql_interval_literal_range_qualifier {
        input: "RETURN INTERVAL '1 02:03:04' DAY(2) TO SECOND(6) AS day_to_second",
        statement_count: 1,
    }

    radix_and_separated_numeric_literals {
        input: "RETURN 1_000 AS decimal, 0x_FF AS hex, 0o755 AS octal, 0b1010_0101 AS binary, .5 AS leading_decimal, 1. AS trailing_decimal, 10M AS exact_decimal, 2.5F AS float_value, 1e3D AS double_value",
        statement_count: 1,
    }

    signed_numeric_expressions {
        input: "RETURN +1 AS positive, -2 AS negative, -+3 AS nested_sign",
        statement_count: 1,
    }

    byte_string_literals {
        input: "RETURN X'0A ff 10' AS raw, x'' AS empty, X'CA'\n'FE' AS chunked",
        statement_count: 1,
    }

    duration_value_functions {
        input: "RETURN DURATION('P1D') AS from_string, DURATION(RECORD {days: 1, hours: 2}) AS from_record, ABS(DURATION 'P1D') AS absolute_duration, duration AS identifier",
        statement_count: 1,
    }

    duration_between_function {
        input: "RETURN DURATION_BETWEEN(DATE '2026-06-29', DATE '2026-06-01') AS days_between, DURATION_BETWEEN(DATETIME '2026-06-29T12:00:00', DATETIME '2026-06-29T10:30:00') AS time_between, duration_between AS identifier",
        statement_count: 1,
    }

    local_datetime_value_functions {
        input: "RETURN CURRENT_DATE AS d, CURRENT_TIME AS t, CURRENT_TIMESTAMP AS ts, CURRENT_USER AS user, LOCAL_TIME AS lt, LOCAL_TIMESTAMP AS lts",
        statement_count: 1,
    }

    standard_datetime_value_functions {
        input: "RETURN DATE() AS current_date, DATE('2026-06-29') AS date_from_string, DATE(RECORD {year: 2026, month: 6, day: 29}) AS date_from_record, DATE({year: 2026}) AS date_from_implicit_record, ZONED_TIME('12:30:00Z') AS zoned_time, ZONED_DATETIME('2026-06-29T12:30:00Z') AS zoned_datetime, LOCAL_TIME() AS local_time, LOCAL_TIMESTAMP AS local_timestamp, LOCAL_DATETIME(RECORD {year: 2026, month: 6, day: 29, hour: 12}) AS local_datetime, zoned_time AS identifier",
        statement_count: 1,
    }

    numeric_value_functions {
        input: "RETURN ABS(-n.delta) AS magnitude, MOD(n.score, 10) AS bucket, FLOOR(n.ratio) AS floored, CEIL(n.ratio) AS ceiled, CEILING(n.ratio) AS ceilinged, SQRT(n.value) AS root, POWER(n.value, 2) AS squared, LOG(10, n.value) AS log_base, LOG10(n.value) AS common_log, LN(n.value) AS natural_log, EXP(n.value) AS exponential, SIN(n.angle) AS s, COS(n.angle) AS c, TAN(n.angle) AS t, COT(n.angle) AS cot, SINH(n.angle) AS sinh, COSH(n.angle) AS cosh, TANH(n.angle) AS tanh, ASIN(n.ratio) AS asin, ACOS(n.ratio) AS acos, ATAN(n.ratio) AS atan, DEGREES(n.angle) AS degrees, RADIANS(n.degrees) AS radians, PATH_LENGTH(p) AS path_len, abs AS identifier",
        statement_count: 1,
    }

    path_value_constructor {
        input: "RETURN PATH [start_node, edge_ref, end_node] AS p, path AS identifier",
        statement_count: 1,
    }

    cast_expressions {
        input: "RETURN CAST($age AS INTEGER) AS age, CAST(['a', 'b'] AS LIST<STRING>) AS names",
        statement_count: 1,
    }

    value_query_expression {
        input: "RETURN VALUE { MATCH (n) RETURN n.name AS name LIMIT 1 } AS name, value AS identifier",
        statement_count: 1,
    }

    value_query_aggregate_expression {
        input: "RETURN VALUE { MATCH (n) RETURN COUNT(*) AS total } AS total",
        statement_count: 1,
    }

    aggregate_value_query_with_named_call {
        input: "RETURN COUNT(VALUE { CALL graph.values() YIELD value RETURN value LIMIT 1 }) AS total",
        statement_count: 1,
    }

    let_value_expression {
        input: "RETURN LET x = 1, VALUE y INTEGER = 2 IN x + y END AS total, let AS identifier",
        statement_count: 1,
    }

    element_id_function {
        input: "MATCH (n) RETURN ELEMENT_ID(n) AS id",
        statement_count: 1,
    }

    case_expressions {
        input: "MATCH (n) RETURN CASE WHEN n.age >= 18 THEN 'adult' ELSE 'minor' END AS bucket",
        statement_count: 1,
    }

    simple_case_predicate_operands {
        input: "MATCH (n)-[r]->(m) RETURN CASE n.age WHEN < 18 THEN 'minor' ELSE 'adult' END AS age_bucket, CASE r WHEN IS DIRECTED THEN 'directed' ELSE 'other' END AS edge_kind",
        statement_count: 1,
    }

    case_abbreviation_expressions {
        input: "RETURN COALESCE(n.name, 'unknown', $fallback) AS name, NULLIF(n.status, 'deleted') AS status, coalesce AS identifier",
        statement_count: 1,
    }

    standard_comparison_and_string_concatenation_operators {
        input: "RETURN 'hello' || ' ' || 'world' AS greeting, 1 <> 2 AS different",
        statement_count: 1,
    }

    standard_string_value_functions {
        input: "RETURN LEFT(name, 2) AS prefix, RIGHT(name, 3) AS suffix, TRIM(BOTH 'x' FROM 'xxnamexx') AS trimmed, TRIM(name) AS simple_trim, BTRIM(name, 'xy') AS both_trimmed, LTRIM(name) AS left_trimmed, RTRIM(name, 'z') AS right_trimmed",
        statement_count: 1,
    }

    standard_string_fold_normalize_and_length_functions {
        input: "RETURN UPPER(n.name) AS upper_name, LOWER(n.name) AS lower_name, NORMALIZE(n.name, NFC) AS normalized, NORMALIZE(n.name, NFD) AS comma_normalized, CHAR_LENGTH(n.name) AS chars, CHARACTER_LENGTH(n.name) AS characters, BYTE_LENGTH(n.raw) AS bytes, OCTET_LENGTH(n.raw) AS octets, upper AS identifier",
        statement_count: 1,
    }

    list_value_functions {
        input: "MATCH p = (a)-[e]->(b) RETURN TRIM([1, 2, 3], 1) AS trimmed, ELEMENTS(p) AS path_elements, elements AS identifier",
        statement_count: 1,
    }

    typed_list_value_constructors {
        input: "RETURN LIST [1, 2, 3] AS numbers, ARRAY ['a', 'b'] AS names, LIST [] AS empty_list, list AS identifier, array AS array_identifier",
        statement_count: 1,
    }

    record_value_constructors {
        input: "RETURN RECORD {name: n.name, age: n.age} AS person, RECORD {} AS empty_record, {nested: RECORD {active: true}} AS nested",
        statement_count: 1,
    }

    null_predicates_and_null_ordering {
        input: "MATCH (n) WHERE n.deleted_at IS NULL OR n.name IS NOT NULL RETURN n ORDER BY n.name ASC NULLS LAST",
        statement_count: 1,
    }

    unknown_truth_value_predicates {
        input: "RETURN TRUE AS t, FALSE AS f, UNKNOWN AS u, $flag IS TRUE AS truthy, $flag IS NOT FALSE AS not_false, $flag IS UNKNOWN AS maybe, $flag IS NOT UNKNOWN AS known",
        statement_count: 1,
    }

    property_exists_predicate {
        input: "MATCH (n) WHERE PROPERTY_EXISTS(n, name) RETURN n",
        statement_count: 1,
    }

    exists_graph_pattern_predicate {
        input: "MATCH (n) WHERE EXISTS { (n)-[:KNOWS]->(:Person) } RETURN n",
        statement_count: 1,
    }

    exists_predicate_variants {
        input: "MATCH (n) WHERE EXISTS ((n)-[:KNOWS]->(:Person)) OR EXISTS { MATCH (m) RETURN m LIMIT 1 } RETURN n",
        statement_count: 1,
    }

    exists_match_statement_block {
        input: "MATCH (n) WHERE EXISTS { MATCH (n)-[:KNOWS]->(m) OPTIONAL MATCH (m)-[:LIKES]->(x) } AND EXISTS (MATCH (n)-[:FOLLOWS]->(f)) RETURN n",
        statement_count: 1,
    }

    same_predicate {
        input: "MATCH (a), (b), (c) WHERE SAME(a, b, c) RETURN a",
        statement_count: 1,
    }

    all_different_predicate {
        input: "MATCH (a), (b), (c) WHERE ALL_DIFFERENT(a, b, c) RETURN a",
        statement_count: 1,
    }

    source_destination_predicates {
        input: "MATCH (a)-[e]->(b) WHERE a IS SOURCE OF e AND b IS NOT DESTINATION OF e RETURN e",
        statement_count: 1,
    }

    normalized_predicates {
        input: "RETURN 'cafe' IS NORMALIZED AS plain, 'cafe' IS NOT NFC NORMALIZED AS nfc",
        statement_count: 1,
    }

    directed_predicates {
        input: "MATCH ()-[e]->() WHERE e IS DIRECTED AND e IS NOT DIRECTED RETURN e",
        statement_count: 1,
    }

    labeled_predicates {
        input: "MATCH (n) WHERE n IS NOT LABELED Person|Admin OR n:Employee RETURN n",
        statement_count: 1,
    }

    element_pattern_where_predicates {
        input: "MATCH (n WHERE n.age > 21)-[r WHERE r.since >= 2020]->(m) RETURN n, r, m",
        statement_count: 1,
    }

    query_union_all {
        input: "MATCH (n) RETURN n UNION ALL MATCH (m) RETURN m",
        statement_count: 1,
    }

    query_otherwise {
        input: "RETURN 1 AS value OTHERWISE RETURN 2 AS value",
        statement_count: 1,
    }

    select_all_quantifier {
        input: "SELECT ALL n",
        statement_count: 1,
    }

    finish_query {
        input: "MATCH (n) FINISH",
        statement_count: 1,
    }
}
