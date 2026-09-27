use gql_parser::{parse, token::TokenKind, Error};

// These cases check the structured diagnostic, including UTF-8 byte offsets,
// independently of the broader message-substring fixtures.
macro_rules! error_cases {
    ($($name:ident { input: $input:expr, error: $error:expr, })*) => {
        $(
            #[test]
            fn $name() {
                let input = $input;
                let err = parse(input).expect_err(&format!("unexpectedly parsed {input:?}"));
                assert_eq!(err, $error, "input: {input:?}");
            }
        )*
    };
}

error_cases! {
    forwards_lexer_error_after_complete_statement {
        input: "RETURN 1 AS n #",
        error: Error::UnrecognizedToken { offset: 14, token_text: "#".into() },
    }
    forwards_trailing_unterminated_comment {
        input: "RETURN 1 AS n /* bad",
        error: Error::UnterminatedBlockComment { offset: 14, token_text: "/* bad".into() },
    }
    forwards_unterminated_string_in_expression {
        input: "RETURN '中",
        error: Error::UnterminatedString { offset: 7, token_text: "'中".into() },
    }
    forwards_escape_error_with_byte_offset {
        input: "RETURN '中\\q' AS x",
        error: Error::Message { offset: 11, message: "invalid string escape '\\q'".into() },
    }
    forwards_numeric_lexer_error {
        input: "RETURN 0x8000000000000000 AS x",
        error: Error::BadNumber { offset: 7, token_text: "0x8000000000000000".into() },
    }
    unexpected_token_after_unicode_identifier {
        input: "MATCH (中] RETURN 中",
        error: Error::UnexpectedToken {
            offset: 10,
            expected: vec![TokenKind::RParen],
            got: TokenKind::RBracket,
            token_text: "]".into(),
        },
    }
    alias_requires_identifier {
        input: "RETURN 1 AS ]",
        error: Error::UnexpectedToken {
            offset: 12,
            expected: vec![TokenKind::Identifier],
            got: TokenKind::RBracket,
            token_text: "]".into(),
        },
    }
    incomplete_node_pattern {
        input: "MATCH (",
        error: Error::UnexpectedToken {
            offset: 7,
            expected: vec![TokenKind::RParen],
            got: TokenKind::Eof,
            token_text: "".into(),
        },
    }
    incomplete_parenthesized_expression {
        input: "RETURN (1 + 2",
        error: Error::UnexpectedEof,
    }
    duplicate_unicode_field_points_to_second_name {
        input: "RETURN {中: 1, 中: 2} AS x",
        error: Error::Message { offset: 16, message: "duplicate field '中'".into() },
    }
}
