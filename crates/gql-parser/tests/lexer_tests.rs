use graphfusion_gql_parser::{
    lexer::Lexer,
    token::{Token, TokenKind},
    Error,
};

macro_rules! token_cases {
    ($($name:ident { input: $input:expr, tokens: [$(($kind:ident, $text:expr, $offset:expr)),* $(,)?], })*) => {
        $(
            #[test]
            fn $name() {
                let input = $input;
                let actual = Lexer::new(input).collect::<Result<Vec<_>, _>>()
                    .unwrap_or_else(|err| panic!("failed to lex {input:?}: {err}"));
                let expected: Vec<Token> = vec![$(Token::new(TokenKind::$kind, $text, $offset)),*];
                assert_eq!(actual, expected, "input: {input:?}");
            }
        )*
    };
}

token_cases! {
    empty_input {
        input: "",
        tokens: [],
    }
    whitespace_only {
        input: " \t\r\n\u{3000}",
        tokens: [],
    }
    comments_only {
        input: "/* 中 */ // line\n-- final line",
        tokens: [],
    }
    basic_query {
        input: "MATCH (n) RETURN n",
        tokens: [
            (Match, "MATCH", 0), (LParen, "(", 6), (Identifier, "n", 7),
            (RParen, ")", 8), (Return, "RETURN", 10), (Identifier, "n", 17),
        ],
    }
    keywords_are_case_insensitive_and_require_word_boundaries {
        input: "mAtCh matchbox RETURNING return",
        tokens: [
            (Match, "mAtCh", 0), (Identifier, "matchbox", 6),
            (Identifier, "RETURNING", 15), (Return, "return", 25),
        ],
    }
    unicode_identifiers_use_byte_offsets {
        input: "节点 café _n2",
        tokens: [(Identifier, "节点", 0), (Identifier, "café", 7), (Identifier, "_n2", 13)],
    }
    block_comments_separate_adjacent_identifiers {
        input: "a/* 中 */b",
        tokens: [(Identifier, "a", 0), (Identifier, "b", 10)],
    }
    line_comments_handle_lf_and_crlf {
        input: "a // skip\nb -- skip\r\nc",
        tokens: [(Identifier, "a", 0), (Identifier, "b", 10), (Identifier, "c", 21)],
    }
    overlapping_operators_use_longest_match {
        input: ":::|||!!=<><=<- ->>=",
        tokens: [
            (DoubleColon, "::", 0), (Colon, ":", 2),
            (DoublePipe, "||", 3), (Pipe, "|", 5),
            (Bang, "!", 6), (Neq, "!=", 7), (Neq, "<>", 9),
            (Le, "<=", 11), (ArrowLeft, "<-", 13),
            (ArrowRight, "->", 16), (Ge, ">=", 18),
        ],
    }
    dot_is_distinct_from_decimal_and_property_access {
        input: "n.x .5 1. 1..2",
        tokens: [
            (Identifier, "n", 0), (Dot, ".", 1), (Identifier, "x", 2),
            (Decimal, ".5", 4), (Decimal, "1.", 7),
            (Integer, "1", 10), (Dot, ".", 11), (Decimal, ".2", 12),
        ],
    }
    signs_are_separate_from_numeric_literals {
        input: "-12 +3",
        tokens: [(Minus, "-", 0), (Integer, "12", 1), (Plus, "+", 4), (Integer, "3", 5)],
    }
    numeric_separators_and_radices_are_normalized {
        input: "1_000 0x_FF 0o755 0b1010_0101",
        tokens: [
            (Integer, "1000", 0), (Integer, "255", 6),
            (Integer, "493", 12), (Integer, "165", 18),
        ],
    }
    numeric_suffixes_and_exponents {
        input: "10M 2.5F 1E-3D",
        tokens: [(Decimal, "10", 0), (Decimal, "2.5", 4), (Decimal, "1e-3", 9)],
    }
    largest_radix_integer {
        input: "0x7fff_ffff_ffff_ffff",
        tokens: [(Integer, "9223372036854775807", 0)],
    }
    strings_decode_escapes_and_doubled_quotes {
        input: r"'it''s\n\u0041\U01F600'",
        tokens: [(String, "it's\nA😀", 0)],
    }
    raw_strings_preserve_backslashes {
        input: r"@'it''s\n'",
        tokens: [(String, r"it's\n", 0)],
    }
    comment_markers_inside_strings_are_text {
        input: "'/* text */ // --'",
        tokens: [(String, "/* text */ // --", 0)],
    }
    string_chunks_join_across_newlines {
        input: "'a'\r\n@'b\\n'\n'c\\t'",
        tokens: [(String, "ab\\nc\t", 0)],
    }
    string_chunks_do_not_join_across_spaces {
        input: "'a' 'b'",
        tokens: [(String, "a", 0), (String, "b", 4)],
    }
    delimited_identifiers_preserve_kind_and_decode_quotes {
        input: "\"a\"\"b\" `c``d`",
        tokens: [(DoubleQuotedString, "a\"b", 0), (QuotedString, "c`d", 7)],
    }
    raw_delimited_identifiers_preserve_backslashes {
        input: r#"@"a\n" @`b\t`"#,
        tokens: [(DoubleQuotedString, r"a\n", 0), (QuotedString, r"b\t", 7)],
    }
    regular_and_ordinal_parameters {
        input: "$name $123 $节点",
        tokens: [(Parameter, "$name", 0), (Parameter, "$123", 6), (Parameter, "$节点", 11)],
    }
    delimited_parameters_decode_their_names {
        input: r#"$"a\n" $@`b\t`"#,
        tokens: [(Parameter, "a\n", 0), (Parameter, r"b\t", 7)],
    }
    byte_string_chunks {
        input: "X'CA'\n'FE 00'",
        tokens: [(ByteString, "CAFE00", 0)],
    }
}

macro_rules! error_cases {
    ($($name:ident { input: $input:expr, error: $error:expr, })*) => {
        $(
            #[test]
            fn $name() {
                let input = $input;
                let err = Lexer::new(input).collect::<Result<Vec<_>, _>>()
                    .expect_err(&format!("unexpectedly lexed {input:?}"));
                assert_eq!(err, $error, "input: {input:?}");
            }
        )*
    };
}

error_cases! {
    unrecognized_token_after_unicode {
        input: "中 #",
        error: Error::UnrecognizedToken { offset: 4, token_text: "#".into() },
    }
    unterminated_string {
        input: " 'unfinished",
        error: Error::UnterminatedString { offset: 1, token_text: "'unfinished".into() },
    }
    unterminated_double_quoted_identifier {
        input: " \"unfinished",
        error: Error::UnterminatedString { offset: 1, token_text: "\"unfinished".into() },
    }
    unterminated_backtick_identifier {
        input: " `unfinished",
        error: Error::UnterminatedString { offset: 1, token_text: "`unfinished".into() },
    }
    unterminated_block_comment_after_unicode {
        input: "中 /* unfinished",
        error: Error::UnterminatedBlockComment { offset: 4, token_text: "/* unfinished".into() },
    }
    invalid_escape_after_unicode {
        input: " '中\\q'",
        error: Error::Message { offset: 5, message: "invalid string escape '\\q'".into() },
    }
    unicode_escape_rejects_surrogate {
        input: r" '\uD800'",
        error: Error::Message { offset: 2, message: "invalid string escape".into() },
    }
    unicode_escape_rejects_out_of_range_scalar {
        input: r" '\U110000'",
        error: Error::Message { offset: 2, message: "invalid string escape".into() },
    }
    unicode_escape_rejects_missing_digits {
        input: r" '\u12'",
        error: Error::Message { offset: 2, message: "invalid string escape".into() },
    }
    number_rejects_repeated_separator {
        input: " 1__2",
        error: Error::BadNumber { offset: 1, token_text: "1__".into() },
    }
    number_rejects_trailing_separator {
        input: " 1_",
        error: Error::BadNumber { offset: 1, token_text: "1_".into() },
    }
    exponent_requires_digits {
        input: " 1e+",
        error: Error::BadNumber { offset: 1, token_text: "1e+".into() },
    }
    radix_requires_digits {
        input: " 0x",
        error: Error::BadNumber { offset: 1, token_text: "0x".into() },
    }
    radix_integer_rejects_overflow {
        input: " 0x8000000000000000",
        error: Error::BadNumber { offset: 1, token_text: "0x8000000000000000".into() },
    }
    parameter_requires_name {
        input: " $ ",
        error: Error::UnrecognizedToken { offset: 1, token_text: "$".into() },
    }
    byte_string_requires_hex_digits {
        input: " X'0G'",
        error: Error::Message { offset: 4, message: "invalid byte string literal character 'G'".into() },
    }
    byte_string_requires_complete_bytes {
        input: " X'0'",
        error: Error::Message { offset: 1, message: "invalid byte string literal 'X'0''".into() },
    }
}

#[test]
fn iterator_stays_exhausted_after_trailing_trivia() {
    let mut lexer = Lexer::new("n /* trailing */\n");
    assert_eq!(
        lexer.next(),
        Some(Ok(Token::new(TokenKind::Identifier, "n", 0)))
    );
    assert_eq!(lexer.next(), None);
    assert_eq!(lexer.next(), None);
}
