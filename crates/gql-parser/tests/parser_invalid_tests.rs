use graphfusion_gql_parser::parse;

// Register each fixture separately so Cargo can filter it and report every failure.
macro_rules! invalid_cases {
    ($($name:ident { input: $input:expr, error_contains: $expected:expr, })*) => {
        $(
            #[test]
            fn $name() {
                let input = $input;
                let err = parse(input)
                    .expect_err(&format!("unexpectedly parsed {input:?}"));
                assert!(
                    err.to_string().contains($expected),
                    "input: {input:?}; expected error containing {:?}, got {err:?}",
                    $expected,
                );
            }
        )*
    };
}

invalid_cases! {
    unterminated_block_comment {
        input: "MATCH (n) RETURN n /* unfinished",
        error_contains: "unterminated block comment",
    }

    unterminated_delimited_identifier {
        input: "RETURN \"unfinished",
        error_contains: "unterminated string",
    }

    delimited_identifier_rejects_unknown_escape {
        input: r#"RETURN "bad\q""#,
        error_contains: "invalid string escape",
    }

    missing_result_clause {
        input: "MATCH (n)",
        error_contains: "expected",
    }

    label_expression_rejects_keyword_not {
        input: "MATCH (n) WHERE n IS LABELED NOT Person RETURN n",
        error_contains: "label negation uses !",
    }

    node_label_expression_rejects_keyword_not {
        input: "MATCH (n IS NOT Person) RETURN n",
        error_contains: "label negation uses !",
    }

    composite_query_cannot_mix_conjunctions {
        input: "RETURN 1 AS value UNION RETURN 2 AS value EXCEPT RETURN 3 AS value",
        error_contains: "mixed query conjunctions",
    }

    optional_match_block_requires_match_statement {
        input: "OPTIONAL { RETURN n } RETURN n",
        error_contains: "expected",
    }

    bad_transaction_isolation {
        input: "START TRANSACTION ISOLATION LEVEL WRITE",
        error_contains: "ISOLATION LEVEL",
    }

    start_transaction_rejects_isolation_level {
        input: "START TRANSACTION READ ONLY, ISOLATION LEVEL SERIALIZABLE",
        error_contains: "ISOLATION LEVEL",
    }

    start_transaction_rejects_duplicate_access_mode {
        input: "START TRANSACTION READ ONLY, READ WRITE",
        error_contains: "exactly one access mode",
    }

    set_all_properties_requires_property_map {
        input: "SET n = 1",
        error_contains: "expected",
    }

    set_property_item_rejects_duplicate_target {
        input: "SET n.age = 1, n.AGE = 2",
        error_contains: "duplicate SET assignment",
    }

    set_all_properties_rejects_duplicate_variable {
        input: "SET n = {age: 1}, N = {name: 'Alice'}",
        error_contains: "duplicate SET all-properties assignment",
    }

    set_all_properties_rejects_later_property_assignment {
        input: "SET n = {age: 1}, n.age = 2",
        error_contains: "all-properties assignment conflicts",
    }

    set_property_rejects_later_all_properties_assignment {
        input: "SET n.age = 1, n = {name: 'Alice'}",
        error_contains: "all-properties assignment conflicts",
    }

    remove_item_requires_property_or_label {
        input: "REMOVE n",
        error_contains: "expected",
    }

    repeated_insert_node_variable_rejects_labels {
        input: "INSERT (n:Person)-[:KNOWS]->(n:Company)",
        error_contains: "repeated insert element variable",
    }

    repeated_insert_node_variable_rejects_properties {
        input: "INSERT (n {name: 'Alice'}), (n {name: 'Bob'})",
        error_contains: "repeated insert element variable",
    }

    insert_edge_variable_cannot_duplicate_node_variable {
        input: "INSERT (n)-[n]->(m)",
        error_contains: "insert edge variable",
    }

    next_statement_requires_statement {
        input: "RETURN n NEXT",
        error_contains: "expected",
    }

    next_yield_requires_item {
        input: "RETURN n NEXT YIELD , RETURN n",
        error_contains: "expected",
    }

    limit_requires_unsigned_integer_specification {
        input: "RETURN n LIMIT 1 + 1",
        error_contains: "expected",
    }

    inline_procedure_requires_statement {
        input: "CALL { } RETURN n",
        error_contains: "expected",
    }

    inline_procedure_variable_scope_requires_closing_paren {
        input: "CALL (n { RETURN n } RETURN n",
        error_contains: "expected",
    }

    inline_procedure_variable_scope_rejects_duplicates {
        input: "CALL (n, n) { RETURN n }",
        error_contains: "duplicate variable",
    }

    inline_procedure_variable_scope_rejects_equivalent_duplicates {
        input: "CALL (n, N) { RETURN n }",
        error_contains: "duplicate variable",
    }

    binding_variable_definitions_reject_duplicate_value_variables {
        input: "CALL { VALUE limit = 1 VALUE LIMIT = 2 RETURN limit }",
        error_contains: "duplicate binding variable",
    }

    binding_variable_definitions_reject_duplicate_graph_and_table_variables {
        input: "CALL { GRAPH rows = HOME_GRAPH TABLE ROWS = app.rows RETURN rows }",
        error_contains: "duplicate binding variable",
    }

    value_variable_definition_requires_initializer {
        input: "VALUE limit INTEGER RETURN limit",
        error_contains: "expected",
    }

    binding_table_variable_type_requires_fields {
        input: "BINDING TABLE rows BINDING TABLE = app.rows RETURN rows",
        error_contains: "expected",
    }

    typed_let_variable_definition_requires_initializer {
        input: "MATCH (n) LET VALUE score INTEGER RETURN score",
        error_contains: "expected",
    }

    let_statement_rejects_duplicate_variables {
        input: "MATCH (n) LET score = n.score, score = n.rank RETURN score",
        error_contains: "duplicate let variable",
    }

    let_value_expression_requires_in {
        input: "RETURN LET x = 1 END",
        error_contains: "expected",
    }

    let_value_expression_rejects_duplicate_variables {
        input: "RETURN LET x = 1, x = 2 IN x END",
        error_contains: "duplicate let variable",
    }

    for_ordinality_variable_cannot_duplicate_item_alias {
        input: "FOR item IN [1, 2] WITH ORDINALITY item RETURN item",
        error_contains: "duplicates item alias",
    }

    for_offset_variable_cannot_duplicate_item_alias {
        input: "FOR item IN [1, 2] WITH OFFSET ITEM RETURN item",
        error_contains: "duplicates item alias",
    }

    case_without_when {
        input: "RETURN CASE ELSE 1 END",
        error_contains: "expected",
    }

    simple_case_when_operand_rejects_full_expression {
        input: "RETURN CASE n WHEN 1 + 2 THEN 'sum' END",
        error_contains: "expected",
    }

    simple_case_when_operand_rejects_parenthesized_expression {
        input: "RETURN CASE n WHEN (1) THEN 'one' END",
        error_contains: "non-parenthesized value primary",
    }

    coalesce_requires_two_expressions {
        input: "RETURN COALESCE(n.name)",
        error_contains: "expected",
    }

    nullif_requires_two_expressions {
        input: "RETURN NULLIF(n.status)",
        error_contains: "expected",
    }

    nulls_without_ordering_direction {
        input: "RETURN n ORDER BY n NULLS",
        error_contains: "expected",
    }

    property_exists_missing_comma {
        input: "RETURN PROPERTY_EXISTS(n name)",
        error_contains: "expected",
    }

    exists_predicate_requires_graph_pattern_or_query {
        input: "RETURN EXISTS ()",
        error_contains: "expected",
    }

    exists_match_block_rejects_trailing_result {
        input: "RETURN EXISTS (MATCH (n) RETURN n)",
        error_contains: "expected",
    }

    same_predicate_requires_two_variables {
        input: "RETURN SAME(n)",
        error_contains: "expected",
    }

    all_different_predicate_requires_two_variables {
        input: "RETURN ALL_DIFFERENT(n)",
        error_contains: "expected",
    }

    source_predicate_requires_of {
        input: "RETURN n IS SOURCE e",
        error_contains: "expected",
    }

    is_predicate_rejects_arbitrary_value_expression {
        input: "RETURN n IS m",
        error_contains: "expected standard IS predicate tail",
    }

    is_not_predicate_rejects_arbitrary_value_expression {
        input: "RETURN n IS NOT m",
        error_contains: "expected standard IS predicate tail",
    }

    path_union_requires_right_term {
        input: "MATCH (a)-[:KNOWS]->(b) | RETURN a",
        error_contains: "expected",
    }

    parenthesized_path_where_rejects_local_path_variable {
        input: "MATCH path = (sub = (a)-[:KNOWS]->(b) WHERE sub IS NOT NULL) RETURN path",
        error_contains: "cannot reference path variable 'sub'",
    }

    function_quantifier_requires_argument {
        input: "RETURN count(DISTINCT)",
        error_contains: "expected",
    }

    cast_requires_value_type {
        input: "RETURN CAST(n AS)",
        error_contains: "expected",
    }

    value_query_requires_nested_query_result {
        input: "RETURN VALUE { MATCH (n) }",
        error_contains: "expected",
    }

    value_query_requires_one_return_item {
        input: "RETURN VALUE { MATCH (n) RETURN n.name AS name, n.age AS age LIMIT 1 }",
        error_contains: "exactly one item",
    }

    value_query_requires_limit_or_aggregate {
        input: "RETURN VALUE { MATCH (n) RETURN n.name AS name }",
        error_contains: "LIMIT 1 or an aggregate result",
    }

    value_query_limit_must_be_one {
        input: "RETURN VALUE { MATCH (n) RETURN n.name AS name LIMIT 2 }",
        error_contains: "LIMIT 1 or an aggregate result",
    }

    value_query_limit_cannot_be_parameterized {
        input: "RETURN VALUE { MATCH (n) RETURN n.name AS name LIMIT $limit }",
        error_contains: "LIMIT 1 or an aggregate result",
    }

    value_query_aggregate_alternative_rejects_group_by {
        input: "RETURN VALUE { MATCH (n) SELECT COUNT(*) AS total GROUP BY city }",
        error_contains: "cannot contain GROUP BY",
    }

    select_nested_query_requires_result {
        input: "SELECT n FROM { MATCH (n) }",
        error_contains: "expected",
    }

    select_where_requires_condition {
        input: "SELECT name FROM { RETURN 1 AS name } WHERE",
        error_contains: "incomplete input",
    }

    float_precision_requires_number {
        input: "RETURN CAST($score AS FLOAT(, 2)) AS score",
        error_contains: "expected",
    }

    signed_integer_type_requires_valid_integer_name {
        input: "RETURN CAST($score AS SIGNED INTENSITY) AS score",
        error_contains: "expected",
    }

    element_id_requires_variable_reference {
        input: "RETURN ELEMENT_ID(n.name)",
        error_contains: "expected",
    }

    property_subscript_expression_is_not_standard_gql {
        input: "RETURN (n.tags[0]) AS first_tag",
        error_contains: "expected",
    }

    list_subscript_expression_is_not_standard_gql {
        input: "RETURN ([1, 2, 3][0]) AS selected",
        error_contains: "expected",
    }

    substring_requires_from {
        input: "RETURN SUBSTRING('abc' 1)",
        error_contains: "expected",
    }

    sql_substring_syntax_is_not_a_gql_string_function {
        input: "RETURN SUBSTRING('abcdef' FROM 2)",
        error_contains: "expected",
    }

    substring_function_name_is_not_standard_gql {
        input: "RETURN SUBSTRING('abcdef', 2)",
        error_contains: "not a standard GQL value function",
    }

    unknown_scalar_function_is_not_standard_gql {
        input: "RETURN custom_score(n.score)",
        error_contains: "not a standard GQL value function",
    }

    left_substring_requires_length {
        input: "RETURN LEFT(name)",
        error_contains: "expected",
    }

    position_requires_in {
        input: "RETURN POSITION('a' name)",
        error_contains: "expected",
    }

    sql_position_syntax_is_not_a_gql_string_function {
        input: "RETURN POSITION('a' IN name)",
        error_contains: "expected",
    }

    overlay_requires_placing {
        input: "RETURN OVERLAY('abcdef' 'ZZ' FROM 2)",
        error_contains: "expected",
    }

    sql_overlay_syntax_is_not_a_gql_string_function {
        input: "RETURN OVERLAY('abcdef' PLACING 'ZZ' FROM 2)",
        error_contains: "expected",
    }

    multi_character_trim_requires_source {
        input: "RETURN BTRIM()",
        error_contains: "expected",
    }

    normalize_normal_form_requires_identifier {
        input: "RETURN NORMALIZE(name AS)",
        error_contains: "expected",
    }

    normalize_does_not_accept_as_normal_form_syntax {
        input: "RETURN NORMALIZE(name AS NFC)",
        error_contains: "expected",
    }

    normalize_comma_normal_form_requires_identifier {
        input: "RETURN NORMALIZE(name,)",
        error_contains: "expected",
    }

    normalize_rejects_invalid_normal_form {
        input: "RETURN NORMALIZE(name, FORM)",
        error_contains: "invalid normal form",
    }

    normalized_predicate_rejects_invalid_normal_form {
        input: "RETURN 'cafe' IS FORM NORMALIZED",
        error_contains: "invalid normal form",
    }

    char_length_requires_closing_paren {
        input: "RETURN CHAR_LENGTH(name",
        error_contains: "incomplete input",
    }

    list_trim_requires_count_expression {
        input: "RETURN TRIM([1, 2, 3],)",
        error_contains: "expected",
    }

    elements_requires_path_expression {
        input: "RETURN ELEMENTS()",
        error_contains: "expected",
    }

    typed_list_constructor_requires_closing_bracket {
        input: "RETURN LIST [1, 2",
        error_contains: "incomplete input",
    }

    group_list_constructor_is_not_user_visible {
        input: "RETURN GROUP LIST [n, m]",
        error_contains: "not user-visible",
    }

    group_array_constructor_is_not_user_visible {
        input: "RETURN GROUP ARRAY []",
        error_contains: "not user-visible",
    }

    path_value_constructor_requires_start_element {
        input: "RETURN PATH []",
        error_contains: "expected",
    }

    path_value_constructor_requires_edge_node_pairs {
        input: "RETURN PATH [n, e]",
        error_contains: "expected",
    }

    field_typed_marker_requires_value_type {
        input: "CREATE GRAPH TYPE bad AS { NODE Person {name TYPED} }",
        error_contains: "expected",
    }

    graph_type_rejects_duplicate_node_type_names {
        input: "CREATE GRAPH TYPE bad AS { NODE Person, NODE person }",
        error_contains: "duplicate node type name",
    }

    graph_type_rejects_duplicate_edge_type_names {
        input: "CREATE GRAPH TYPE bad AS { NODE Person, DIRECTED EDGE Knows CONNECTING (Person TO Person), DIRECTED EDGE knows CONNECTING (Person TO Person) }",
        error_contains: "duplicate edge type name",
    }

    node_property_type_set_rejects_duplicate_property_names {
        input: "CREATE GRAPH TYPE bad AS { NODE Person {name STRING, NAME INTEGER} }",
        error_contains: "duplicate property",
    }

    edge_property_type_set_rejects_duplicate_property_names {
        input: "CREATE GRAPH TYPE bad AS { NODE Person, DIRECTED EDGE Knows FROM Person TO Person {since INTEGER, SINCE STRING} }",
        error_contains: "duplicate property",
    }

    graph_type_rejects_undefined_source_endpoint_node_type {
        input: "CREATE GRAPH TYPE bad AS { NODE Person, DIRECTED EDGE Knows FROM Missing TO Person }",
        error_contains: "endpoint node type",
    }

    graph_type_rejects_undefined_destination_endpoint_node_type {
        input: "CREATE GRAPH TYPE bad AS { NODE Person, DIRECTED EDGE Knows FROM Person TO Missing }",
        error_contains: "endpoint node type",
    }

    directed_edge_type_rejects_undirected_endpoint_pair {
        input: "CREATE GRAPH TYPE bad AS { NODE Person, DIRECTED EDGE Knows CONNECTING (Person ~ Person) }",
        error_contains: "DIRECTED edge type",
    }

    undirected_edge_type_rejects_directed_abbreviated_endpoint_pair {
        input: "CREATE GRAPH TYPE bad AS { NODE Person, UNDIRECTED EDGE Knows CONNECTING (Person)->(Person) }",
        error_contains: "UNDIRECTED edge type",
    }

    graph_type_body_edge_type_phrase_requires_edge_kind {
        input: "CREATE GRAPH TYPE bad AS { NODE Person, EDGE Knows CONNECTING (Person TO Person) }",
        error_contains: "requires DIRECTED or UNDIRECTED",
    }

    graph_type_body_edge_type_phrase_requires_edge_type_name {
        input: "CREATE GRAPH TYPE bad AS { NODE Person, DIRECTED EDGE :KNOWS CONNECTING (Person TO Person) }",
        error_contains: "requires an edge type name",
    }

    closed_edge_reference_rejects_edge_type_name {
        input: "RETURN CAST($edge AS EDGE Knows FROM Person TO Person)",
        error_contains: "edge type name",
    }

    closed_edge_reference_rejects_endpoint_node_type_names {
        input: "RETURN CAST($edge AS (Person)->(Company))",
        error_contains: "endpoint node type names",
    }

    list_value_type_max_length_requires_integer {
        input: "CREATE GRAPH TYPE bad AS { NODE Person {tags STRING LIST[]} }",
        error_contains: "expected",
    }

    list_value_type_max_length_must_be_positive {
        input: "CREATE GRAPH TYPE bad AS { NODE Person {tags STRING LIST[0]} }",
        error_contains: "must be greater than or equal to 1",
    }

    group_list_value_type_is_not_user_visible {
        input: "RETURN CAST($names AS GROUP LIST<STRING>)",
        error_contains: "not user-visible",
    }

    group_array_value_type_is_not_user_visible {
        input: "RETURN CAST($names AS STRING GROUP ARRAY)",
        error_contains: "not user-visible",
    }

    group_list_value_type_requires_list_synonym {
        input: "RETURN CAST($names AS GROUP<STRING>)",
        error_contains: "expected",
    }

    character_string_length_requires_integer {
        input: "RETURN CAST($name AS VARCHAR())",
        error_contains: "expected",
    }

    character_string_max_length_must_be_positive {
        input: "RETURN CAST($name AS STRING(0))",
        error_contains: "must be greater than or equal to 1",
    }

    temporal_type_requires_full_time_zone_phrase {
        input: "RETURN CAST($time AS TIME WITH ZONE)",
        error_contains: "expected",
    }

    record_field_type_requires_value_type {
        input: "RETURN CAST($x AS {name})",
        error_contains: "expected",
    }

    record_field_type_list_rejects_duplicate_field_names {
        input: "RETURN CAST($x AS {name STRING, NAME INTEGER})",
        error_contains: "duplicate field",
    }

    binding_table_field_type_list_rejects_duplicate_field_names {
        input: "RETURN CAST($rows AS BINDING TABLE {node ANY NODE, NODE ANY EDGE})",
        error_contains: "duplicate field",
    }

    dynamic_union_requires_component_after_pipe {
        input: "RETURN CAST($x AS ANY<STRING |>)",
        error_contains: "expected",
    }

    dynamic_union_rejects_mixed_component_nullability {
        input: "RETURN CAST($x AS STRING | INTEGER NOT NULL)",
        error_contains: "matching nullability",
    }

    any_value_dynamic_union_rejects_mixed_component_nullability {
        input: "RETURN CAST($x AS ANY<STRING NOT NULL | INTEGER>)",
        error_contains: "matching nullability",
    }

    closed_graph_reference_type_requires_nested_body {
        input: "RETURN CAST($g AS PROPERTY GRAPH)",
        error_contains: "expected",
    }

    binding_table_type_requires_fields {
        input: "RETURN CAST($rows AS BINDING TABLE)",
        error_contains: "expected",
    }

    bytes_type_requires_length_before_comma {
        input: "RETURN CAST($raw AS BYTES(, 16))",
        error_contains: "expected",
    }

    bytes_max_length_must_be_positive {
        input: "RETURN CAST($raw AS BYTES(0))",
        error_contains: "must be greater than or equal to 1",
    }

    bytes_min_length_cannot_exceed_max_length {
        input: "RETURN CAST($raw AS BYTES(16, 2))",
        error_contains: "min length exceeds max length",
    }

    decimal_type_requires_precision_before_comma {
        input: "RETURN CAST($value AS DECIMAL(, 2))",
        error_contains: "expected",
    }

    decimal_precision_must_be_positive {
        input: "RETURN CAST($value AS DECIMAL(0, 2))",
        error_contains: "must be greater than or equal to 1",
    }

    decimal_scale_cannot_exceed_precision {
        input: "RETURN CAST($value AS DECIMAL(2, 5))",
        error_contains: "scale exceeds precision",
    }

    integer_precision_must_be_positive {
        input: "RETURN CAST($value AS INT(0))",
        error_contains: "must be greater than or equal to 1",
    }

    integer_suffix_must_use_a_standard_width {
        input: "RETURN CAST($value AS INT7)",
        error_contains: "integer type suffix",
    }

    unsigned_integer_suffix_must_use_a_standard_width {
        input: "RETURN CAST($value AS UINT9)",
        error_contains: "integer type suffix",
    }

    float_precision_must_be_positive {
        input: "RETURN CAST($value AS FLOAT(0))",
        error_contains: "must be greater than or equal to 1",
    }

    float_precision_must_be_at_least_two {
        input: "RETURN CAST($value AS FLOAT(1))",
        error_contains: "greater than or equal to 2",
    }

    float_suffix_must_use_a_standard_width {
        input: "RETURN CAST($value AS FLOAT17)",
        error_contains: "FLOAT type suffix",
    }

    unsigned_type_requires_integer_type {
        input: "RETURN CAST($value AS UNSIGNED STRING)",
        error_contains: "expected",
    }

    record_constructor_requires_field_name {
        input: "RETURN RECORD {name: 'Alice', 1: 'bad'}",
        error_contains: "expected",
    }

    record_constructor_field_requires_value {
        input: "RETURN RECORD {name:}",
        error_contains: "expected",
    }

    record_constructor_rejects_duplicate_fields {
        input: "RETURN RECORD {name: 'Alice', NAME: 'Bob'}",
        error_contains: "duplicate field",
    }

    bare_record_constructor_rejects_duplicate_fields {
        input: "RETURN {id: 1, id: 2}",
        error_contains: "duplicate field",
    }

    element_property_map_rejects_duplicate_fields {
        input: "MATCH (n {id: 1, ID: 2}) RETURN n",
        error_contains: "duplicate field",
    }

    extract_function_name_is_not_standard_gql {
        input: "RETURN EXTRACT(n)",
        error_contains: "not a standard GQL value function",
    }

    localtime_function_name_is_not_standard_gql {
        input: "RETURN LOCALTIME('bad')",
        error_contains: "not a standard GQL value function",
    }

    localtimestamp_function_name_is_not_standard_gql {
        input: "RETURN LOCALTIMESTAMP()",
        error_contains: "not a standard GQL value function",
    }

    current_time_does_not_accept_parameters {
        input: "RETURN CURRENT_TIME(3)",
        error_contains: "CURRENT_TIME does not accept parameters",
    }

    current_timestamp_does_not_accept_parameters {
        input: "RETURN CURRENT_TIMESTAMP('bad')",
        error_contains: "CURRENT_TIMESTAMP does not accept parameters",
    }

    current_role_is_reserved_but_not_a_predefined_value {
        input: "RETURN CURRENT_ROLE AS role",
        error_contains: "expected",
    }

    local_timestamp_does_not_accept_parameters {
        input: "RETURN LOCAL_TIMESTAMP(6)",
        error_contains: "LOCAL_TIMESTAMP does not accept parameters",
    }

    date_function_rejects_empty_leading_comma {
        input: "RETURN DATE(,)",
        error_contains: "DATE requires a string literal or record value constructor parameter",
    }

    date_function_rejects_arbitrary_expression_parameter {
        input: "MATCH (n) RETURN DATE(n.created)",
        error_contains: "DATE requires a string literal or record value constructor parameter",
    }

    zoned_time_function_rejects_arbitrary_expression_parameter {
        input: "MATCH (n) RETURN ZONED_TIME(n.time)",
        error_contains: "ZONED_TIME requires a string literal or record value constructor parameter",
    }

    local_datetime_function_rejects_parameter_reference {
        input: "RETURN LOCAL_DATETIME($dt)",
        error_contains: "LOCAL_DATETIME requires a string literal or record value constructor parameter",
    }

    duration_function_rejects_arbitrary_expression_parameter {
        input: "MATCH (n) RETURN DURATION(n.duration)",
        error_contains: "DURATION requires a string literal or record value constructor parameter",
    }

    zoned_time_function_requires_closing_paren {
        input: "RETURN ZONED_TIME('12:00:00Z'",
        error_contains: "incomplete input",
    }

    duration_function_requires_argument {
        input: "RETURN DURATION()",
        error_contains: "DURATION requires a string literal or record value constructor parameter",
    }

    duration_function_requires_closing_paren {
        input: "RETURN DURATION('P1D'",
        error_contains: "incomplete input",
    }

    duration_between_requires_two_expressions {
        input: "RETURN DURATION_BETWEEN(DATE '2026-06-29')",
        error_contains: "expected",
    }

    duration_between_requires_closing_paren {
        input: "RETURN DURATION_BETWEEN(DATE '2026-06-29', DATE '2026-06-01'",
        error_contains: "incomplete input",
    }

    sql_interval_literal_requires_qualifier {
        input: "RETURN INTERVAL '1'",
        error_contains: "incomplete input",
    }

    sql_interval_literal_rejects_cross_group_range {
        input: "RETURN INTERVAL '1' YEAR TO DAY",
        error_contains: "invalid SQL interval qualifier range",
    }

    sql_interval_literal_rejects_reversed_range {
        input: "RETURN INTERVAL '1' SECOND TO YEAR",
        error_contains: "invalid SQL interval qualifier range",
    }

    sql_interval_literal_rejects_invalid_end_precision {
        input: "RETURN INTERVAL '1' DAY TO HOUR(2)",
        error_contains: "only SECOND end interval field can specify fractional precision",
    }

    between_predicate_is_not_standard_gql {
        input: "MATCH (n) WHERE n.age BETWEEN 18 AND 30 RETURN n",
        error_contains: "expected",
    }

    in_predicate_is_not_standard_gql {
        input: "MATCH (n) WHERE n.status IN ['blocked'] RETURN n",
        error_contains: "expected",
    }

    like_predicate_is_not_standard_gql {
        input: "MATCH (n) WHERE n.name LIKE 'A%' RETURN n",
        error_contains: "expected",
    }

    character_string_literal_rejects_unknown_escape {
        input: r"RETURN '\q'",
        error_contains: "invalid string escape",
    }

    character_string_literal_rejects_short_unicode_escape {
        input: r"RETURN '\u12'",
        error_contains: "invalid string escape",
    }

    numeric_literal_rejects_consecutive_separators {
        input: "RETURN 1__2",
        error_contains: "bad number",
    }

    hex_literal_requires_digits {
        input: "RETURN 0x",
        error_contains: "bad number",
    }

    binary_literal_rejects_non_binary_digits {
        input: "RETURN 0b102",
        error_contains: "bad number",
    }

    numeric_literal_rejects_extra_suffix_text {
        input: "RETURN 1FF",
        error_contains: "bad number",
    }

    byte_string_literal_requires_hex_pairs {
        input: "RETURN X'0'",
        error_contains: "invalid byte string literal",
    }

    byte_string_literal_rejects_non_hex_digits {
        input: "RETURN X'0G'",
        error_contains: "invalid byte string literal",
    }

    abs_requires_argument {
        input: "RETURN ABS()",
        error_contains: "expected",
    }

    power_requires_two_arguments {
        input: "RETURN POWER(n.value)",
        error_contains: "expected",
    }

    log_requires_two_arguments {
        input: "RETURN LOG(n.value)",
        error_contains: "expected",
    }

    sin_accepts_one_argument {
        input: "RETURN SIN(n.angle, 1)",
        error_contains: "expected",
    }

    numeric_expression_rejects_percent_modulus_operator {
        input: "RETURN (5 % 2) AS remainder",
        error_contains: "expected",
    }

    mod_requires_closing_paren {
        input: "RETURN MOD(n.value, 10",
        error_contains: "incomplete input",
    }

    count_aggregate_requires_argument {
        input: "RETURN COUNT()",
        error_contains: "aggregate function COUNT",
    }

    count_star_cannot_use_quantifier {
        input: "RETURN COUNT(DISTINCT *)",
        error_contains: "COUNT(*) cannot specify a set quantifier",
    }

    avg_aggregate_requires_argument {
        input: "RETURN AVG()",
        error_contains: "aggregate function AVG",
    }

    avg_aggregate_rejects_wildcard {
        input: "RETURN AVG(*)",
        error_contains: "aggregate function AVG",
    }

    percentile_aggregate_requires_two_arguments {
        input: "RETURN PERCENTILE_CONT(n.score)",
        error_contains: "aggregate function PERCENTILE_CONT",
    }

    aggregate_argument_rejects_procedure_body {
        input: "RETURN COUNT(VALUE { CALL { RETURN 1 AS value } RETURN value LIMIT 1 }) AS total",
        error_contains: "cannot contain a procedure body",
    }

    percentile_aggregate_rejects_extra_argument {
        input: "RETURN PERCENTILE_DISC(n.score, 0.5, 0.9)",
        error_contains: "aggregate function PERCENTILE_DISC",
    }

    property_graph_keyword_requires_graph {
        input: "USE PROPERTY CURRENT_GRAPH MATCH (n) RETURN n",
        error_contains: "expected",
    }

    session_time_requires_zone {
        input: "SESSION SET TIME 'UTC'",
        error_contains: "expected",
    }

    set_schema_shorthand_is_not_standard_gql {
        input: "SET SCHEMA app.main",
        error_contains: "expected",
    }

    set_property_graph_shorthand_is_not_standard_gql {
        input: "SET PROPERTY GRAPH HOME_PROPERTY_GRAPH",
        error_contains: "expected",
    }

    reset_graph_shorthand_is_not_standard_gql {
        input: "RESET GRAPH",
        error_contains: "expected",
    }

    close_session_shorthand_is_not_standard_gql {
        input: "CLOSE SESSION",
        error_contains: "expected",
    }

    session_reset_all_requires_supported_target {
        input: "SESSION RESET ALL GRAPH",
        error_contains: "expected",
    }

    session_reset_session_is_not_standard_gql {
        input: "SESSION RESET SESSION",
        error_contains: "expected",
    }

    session_value_parameter_typed_initializer_requires_equals {
        input: "SESSION SET VALUE $limit INTEGER 10",
        error_contains: "expected",
    }

    session_value_parameter_requires_initializer {
        input: "SESSION SET VALUE $flag",
        error_contains: "incomplete input",
    }

    session_binding_table_parameter_requires_initializer {
        input: "SESSION SET TABLE $rows",
        error_contains: "incomplete input",
    }

    bare_subscript_expression_is_not_standard_gql {
        input: "RETURN n.tags[0",
        error_contains: "non-binding result item requires AS alias",
    }

    binding_table_reference_requires_table_expression {
        input: "RETURN BINDING TABLE",
        error_contains: "incomplete input",
    }

    return_rejects_duplicate_implicit_outputs {
        input: "RETURN n, N",
        error_contains: "duplicate result output",
    }

    return_rejects_duplicate_aliases {
        input: "RETURN n.name AS name, n.full_name AS NAME",
        error_contains: "duplicate result output",
    }

    select_rejects_duplicate_aliases {
        input: "SELECT n.name AS name, n.full_name AS NAME",
        error_contains: "duplicate result output",
    }

    graph_source_requires_copy_of {
        input: "CREATE GRAPH broken AS HOME_GRAPH",
        error_contains: "expected",
    }

    create_graph_requires_graph_type {
        input: "CREATE GRAPH broken",
        error_contains: "expected",
    }

    create_graph_type_nested_source_does_not_take_graph_type_keywords {
        input: "CREATE GRAPH TYPE bad AS GRAPH TYPE { NODE Person }",
        error_contains: "expected",
    }

    commit_command_does_not_take_transaction_keyword {
        input: "COMMIT TRANSACTION",
        error_contains: "expected",
    }

    rollback_command_does_not_take_transaction_keyword {
        input: "ROLLBACK TRANSACTION",
        error_contains: "expected",
    }

    procedure_yield_does_not_take_no_bindings {
        input: "CALL graph.expand() YIELD NO BINDINGS",
        error_contains: "expected",
    }

    procedure_yield_does_not_take_star {
        input: "CALL graph.expand() YIELD *",
        error_contains: "expected",
    }

    procedure_yield_rejects_duplicate_outputs {
        input: "CALL graph.expand() YIELD node AS n, other AS n",
        error_contains: "duplicate yield output",
    }

    procedure_yield_rejects_equivalent_duplicate_outputs {
        input: "CALL graph.expand() YIELD node AS n, other AS N",
        error_contains: "duplicate yield output",
    }

    procedure_yield_rejects_equivalent_implicit_outputs {
        input: "CALL graph.expand() YIELD node, NODE",
        error_contains: "duplicate yield output",
    }

    next_yield_does_not_take_no_bindings {
        input: "RETURN 1 AS n NEXT YIELD NO BINDINGS RETURN n",
        error_contains: "expected",
    }

    graph_pattern_yield_no_bindings_is_not_user_visible {
        input: "MATCH (n) YIELD NO BINDINGS RETURN n",
        error_contains: "not declared",
    }

    graph_pattern_yield_does_not_take_star {
        input: "MATCH (n) YIELD * RETURN *",
        error_contains: "expected",
    }

    graph_pattern_yield_rejects_duplicate_variables {
        input: "MATCH (n)-[e]->(m) YIELD n, n RETURN n",
        error_contains: "duplicate yield output",
    }

    graph_pattern_yield_rejects_equivalent_duplicate_variables {
        input: "MATCH (n)-[e]->(m) YIELD n, N RETURN n",
        error_contains: "duplicate yield output",
    }

    graph_pattern_yield_rejects_undeclared_variable {
        input: "MATCH (n)-[e]->(m) YIELD n, missing RETURN n",
        error_contains: "graph pattern yield variable 'missing' is not declared",
    }

    graph_pattern_yield_rejects_temporary_variable {
        input: "MATCH (TEMP n)-[e]->(m) YIELD n RETURN n",
        error_contains: "TEMP element variable declarations are not user-visible",
    }

    temporary_element_variable_is_not_user_visible {
        input: "MATCH (TEMP a:Person)-[TEMP r:KNOWS]->(b) RETURN a, r",
        error_contains: "TEMP element variable declarations are not user-visible",
    }

    non_binding_return_item_requires_alias {
        input: "RETURN 1 + 2",
        error_contains: "requires AS alias",
    }

    return_star_requires_non_unit_input {
        input: "RETURN *",
        error_contains: "non-unit incoming working table",
    }

    return_star_rejects_use_graph_only_input {
        input: "USE GRAPH social RETURN *",
        error_contains: "non-unit incoming working table",
    }

    return_star_rejects_filter_only_input {
        input: "FILTER true RETURN *",
        error_contains: "non-unit incoming working table",
    }

    return_star_rejects_page_only_input {
        input: "LIMIT 1 RETURN *",
        error_contains: "non-unit incoming working table",
    }

    return_star_rejects_insert_only_input {
        input: "INSERT (:Seen) RETURN *",
        error_contains: "non-unit incoming working table",
    }

    return_no_bindings_is_not_user_visible {
        input: "MATCH (n) RETURN NO BINDINGS",
        error_contains: "not user-visible standard GQL syntax",
    }

    select_star_requires_from_clause {
        input: "SELECT *",
        error_contains: "requires a FROM",
    }

    return_star_cannot_group {
        input: "MATCH (n) RETURN * GROUP BY n",
        error_contains: "cannot contain GROUP BY",
    }

    group_by_rejects_property_reference {
        input: "MATCH (n) RETURN n.name AS name GROUP BY n.name",
        error_contains: "GROUP BY elements must be binding variable references",
    }

    group_by_rejects_value_expression {
        input: "MATCH (n) RETURN n.age + 1 AS age GROUP BY age + 1",
        error_contains: "GROUP BY elements must be binding variable references",
    }

    return_rejects_having_clause {
        input: "MATCH (n) RETURN count(*) AS total GROUP BY () HAVING total > 0",
        error_contains: "HAVING is only valid in SELECT statements",
    }

    external_object_reference_requires_colon {
        input: "CREATE GRAPH TYPE bad AS COPY OF 'relative/path'",
        error_contains: "external object reference",
    }
}
