use gql_parser::{lexer::Lexer, parse, Error, Expr, Literal, Statement};

fn parse_expression(expression: &str) -> Expr {
    let input = format!("RETURN {expression} AS value");
    let program = parse(&input).unwrap_or_else(|err| panic!("failed to parse {input:?}: {err}"));
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected a query for {input:?}");
    };
    query.body.result_clause.items[0].expr.clone()
}

#[test]
fn integer_radices_produce_the_same_value() {
    for value in (0..=255_i64).chain([256, 65535, i64::MAX]) {
        for input in [
            value.to_string(),
            format!("0x{value:x}"),
            format!("0o{value:o}"),
            format!("0b{value:b}"),
        ] {
            assert_eq!(
                parse_expression(&input),
                Expr::Literal(Literal::Integer(value)),
                "literal: {input:?}",
            );
        }
    }
}

#[test]
fn escaped_and_raw_strings_round_trip() {
    let fragments = [
        "", "text", "'", "\"", "`", "\\", "\n", "\r", "\t", "中", "😀",
    ];
    for left in fragments {
        for right in fragments {
            let value = format!("{left}{right}");
            let escaped = value
                .replace('\\', "\\\\")
                .replace('\'', "''")
                .replace('\n', "\\n")
                .replace('\r', "\\r")
                .replace('\t', "\\t");
            let raw = value.replace('\'', "''");
            for input in [format!("'{escaped}'"), format!("@'{raw}'")] {
                assert_eq!(
                    parse_expression(&input),
                    Expr::Literal(Literal::String(value.clone())),
                    "literal: {input:?}",
                );
            }
        }
    }
}

#[test]
fn trivia_between_tokens_preserves_the_entire_ast() {
    let queries: &[&[&str]] = &[
        &[
            "MATCH", "(", "节点", ":", "Person", ")", "WHERE", "节点", ".", "age", ">=", "18",
            "RETURN", "节点",
        ],
        &["RETURN", "1", "+", "2", "*", "3", "AS", "value"],
        &[
            "INSERT", "(", ":", "Person", "{", "name", ":", "'Alice'", "}", ")",
        ],
    ];
    for tokens in queries {
        let expected = parse(&tokens.join(" ")).unwrap();
        for separator in [
            " ",
            "\t",
            "\r\n",
            "\u{3000}",
            "/**/",
            "/* 中 */",
            "// comment\n",
            "-- comment\r\n",
        ] {
            let input = format!("{separator}{}{separator}", tokens.join(separator));
            let actual =
                parse(&input).unwrap_or_else(|err| panic!("failed to parse {input:?}: {err}"));
            assert_eq!(actual, expected, "input: {input:?}");
        }
    }
}

#[test]
fn keyword_case_preserves_the_entire_ast() {
    let expected = parse("MATCH (n:Person) WHERE n.age >= 18 RETURN n").unwrap();
    for match_keyword in ["MATCH", "match", "MaTcH"] {
        for where_keyword in ["WHERE", "where", "WhErE"] {
            for return_keyword in ["RETURN", "return", "ReTuRn"] {
                let input = format!(
                    "{match_keyword} (n:Person) {where_keyword} n.age >= 18 {return_keyword} n"
                );
                assert_eq!(parse(&input).unwrap(), expected, "input: {input:?}");
            }
        }
    }
}

fn assert_error_offset(input: &str, err: &Error) {
    let offset = match err {
        Error::UnrecognizedToken { offset, .. }
        | Error::UnterminatedString { offset, .. }
        | Error::UnterminatedBlockComment { offset, .. }
        | Error::BadNumber { offset, .. }
        | Error::UnexpectedToken { offset, .. }
        | Error::Message { offset, .. } => *offset,
        Error::UnexpectedEof => return,
    };
    assert!(
        input.is_char_boundary(offset),
        "error offset must be within the input on a UTF-8 boundary: {input:?}: {err:?}",
    );
}

#[test]
fn truncated_inputs_do_not_panic_and_keep_offsets_on_utf8_boundaries() {
    // Exercise every character boundary of a small, deterministic corpus. Some
    // prefixes are valid programs; others must produce an error without panicking.
    let corpus = [
        "MATCH (节点:Person {name: '中😀'})-[e:KNOWS]->(m) RETURN 节点",
        r"RETURN '\u0041\U01F600' AS value",
        r#"RETURN $@"中\n" AS value"#,
        "RETURN 0x7fff_ffff AS value, 1.25e-3 AS fraction",
        "RETURN X'CA'\n'FE' AS bytes",
        "RETURN (1 + 2) * 3 AS value /* 中😀 */",
        "CALL { MATCH (n) RETURN n } RETURN n",
        "CREATE GRAPH TYPE social AS { (person:Person {name STRING}) }",
    ];
    for input in corpus {
        for end in input
            .char_indices()
            .map(|(offset, _)| offset)
            .chain(std::iter::once(input.len()))
        {
            let prefix = &input[..end];
            let mut previous_offset = None;
            for (count, result) in Lexer::new(prefix).enumerate() {
                assert!(count < prefix.len(), "lexer did not advance for {prefix:?}");
                match result {
                    Ok(token) => {
                        assert!(
                            token.offset < prefix.len() && prefix.is_char_boundary(token.offset)
                        );
                        assert!(previous_offset.is_none_or(|previous| token.offset > previous));
                        previous_offset = Some(token.offset);
                    }
                    Err(err) => {
                        assert_error_offset(prefix, &err);
                        break;
                    }
                }
            }
            let parsed = std::panic::catch_unwind(|| parse(prefix))
                .unwrap_or_else(|_| panic!("parser panicked for {prefix:?}"));
            if let Err(err) = parsed {
                assert_error_offset(prefix, &err);
            }
        }
    }
}
