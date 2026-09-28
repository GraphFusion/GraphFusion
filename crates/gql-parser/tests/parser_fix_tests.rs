use gql_parser::{ast::*, parse};

fn returned(input: &str) -> Expr {
    let program = parse(input).unwrap_or_else(|err| panic!("parse {input:?}: {err}"));
    let Statement::Query(query) = &program.statements[0] else {
        panic!("expected query");
    };
    query.body.result_clause.items[0].expr.clone()
}

#[test]
fn not_binds_looser_than_comparison() {
    assert_eq!(
        returned("RETURN NOT 1 = 2 AS x"),
        Expr::Unary {
            op: UnaryOp::Not,
            expr: Box::new(Expr::Binary {
                left: Box::new(Expr::Literal(Literal::Integer(1))),
                op: BinaryOp::Eq,
                right: Box::new(Expr::Literal(Literal::Integer(2))),
            }),
        }
    );
}

#[test]
fn most_negative_integer_literals_parse() {
    for input in [
        "RETURN -9223372036854775808 AS x",
        "RETURN -0x8000000000000000 AS x",
    ] {
        assert_eq!(
            returned(input),
            Expr::Literal(Literal::Integer(i64::MIN)),
            "{input}"
        );
    }
}

#[test]
fn decimal_exponents_parse() {
    assert_eq!(
        returned("RETURN .5e3 AS x"),
        Expr::Literal(Literal::Decimal(500.0))
    );
    assert_eq!(
        returned("RETURN 1.e5 AS x"),
        Expr::Literal(Literal::Decimal(100_000.0))
    );
    let err = parse("RETURN 1.exp AS x").unwrap_err();
    assert!(err.to_string().contains("bad number"), "{err}");
}

#[test]
fn quoted_parameter_keeps_a_leading_dollar() {
    assert_eq!(
        returned(r#"RETURN $"$a" AS x"#),
        Expr::Parameter("$a".into())
    );
}

#[test]
fn lexer_error_is_not_replaced_by_a_later_parse_error() {
    let err = parse("RETURN n IS § AS x").unwrap_err();
    let text = err.to_string();
    assert!(
        text.contains('§') || text.contains("unrecognized"),
        "{text}"
    );
}

#[test]
fn long_but_shallow_addition_parses_and_drops() {
    let input = format!("RETURN {} AS x", ["1"].repeat(120).join("+"));
    let program = parse(&input).unwrap();
    let _ = gql_parser::format_ast(&program);
}
