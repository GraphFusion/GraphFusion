use std::ops::ControlFlow;

use gql_parser::visit::Visitor;
use gql_parser::{format_ast, parse, AstTreePrinter, Expr, InlineProcedureCall, Literal};

macro_rules! snapshots {
    ($($name:ident),* $(,)?) => {
        $(#[test]
        fn $name() {
            let input = include_str!(concat!("fixtures/ast/", stringify!($name), ".gql"));
            let expected = include_str!(concat!("fixtures/ast/", stringify!($name), ".ast"));
            let ast = parse(input).unwrap();
            assert_eq!(format_ast(&ast), expected, "AST snapshot for {}", stringify!($name));
        })*
    };
}

snapshots! { precedence, match_query, graph_type, nested_call, session_commands, path_alternation }

#[test]
fn prints_individual_expressions_with_correct_branch_connectors() {
    let expr = Expr::List(vec![
        Expr::Literal(Literal::Integer(1)),
        Expr::Literal(Literal::String("中\n\"\\".into())),
    ]);
    let mut printer = AstTreePrinter::new();
    let ControlFlow::Continue(()) = printer.visit_expr(&expr);
    assert_eq!(
        printer.finish(),
        concat!(
            "Expr::List\n",
            "└── value: List (len=2)\n",
            "    ├── [0]: Expr::Literal\n",
            "    │   └── value: Literal::Integer\n",
            "    │       └── value: 1\n",
            "    └── [1]: Expr::Literal\n",
            "        └── value: Literal::String\n",
            "            └── value: \"中\\n\\\"\\\\\"\n",
        )
    );
}

#[test]
fn preserves_the_difference_between_missing_and_empty_optional_lists() {
    let body = Box::new(parse("RETURN 1 AS value").unwrap());
    let mut printer = AstTreePrinter::new();
    let ControlFlow::Continue(()) = printer.visit_inline_procedure_call(&InlineProcedureCall {
        variable_scope: None,
        body: body.clone(),
    });
    let missing = printer.finish();
    let mut printer = AstTreePrinter::new();
    let ControlFlow::Continue(()) = printer.visit_inline_procedure_call(&InlineProcedureCall {
        variable_scope: Some(Vec::new()),
        body,
    });
    let empty = printer.finish();
    assert!(missing.contains("variable_scope: None\n"));
    assert!(empty.contains("variable_scope: []\n"));
    assert_ne!(missing, empty);
}

#[test]
fn empty_printer_and_multiple_roots_are_well_defined() {
    assert_eq!(AstTreePrinter::new().finish(), "");
    let mut printer = AstTreePrinter::new();
    let ControlFlow::Continue(()) = printer.visit_expr(&Expr::Wildcard);
    let ControlFlow::Continue(()) = printer.visit_literal(&Literal::Null);
    assert_eq!(printer.finish(), "Expr::Wildcard\nLiteral::Null\n");
}
