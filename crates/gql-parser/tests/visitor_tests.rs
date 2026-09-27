use std::convert::Infallible;
use std::ops::ControlFlow;

use gql_parser::visit::{self, AstNode, Field, Scalar, Visitor};
use gql_parser::{parse, Expr, Identifier, Literal};

#[derive(Default)]
struct Recorder<'ast> {
    stack: Vec<String>,
    kinds: Vec<&'static str>,
    parameters: Vec<&'ast str>,
    identifiers: Vec<&'ast Identifier>,
    leaves: Vec<String>,
}

impl<'ast> Visitor<'ast> for Recorder<'ast> {
    type Break = Infallible;

    fn enter_node(&mut self, node: AstNode<'ast>) -> ControlFlow<Self::Break> {
        self.kinds.push(node.kind());
        self.stack.push(node.kind().into());
        ControlFlow::Continue(())
    }

    fn leave_node(&mut self, node: AstNode<'ast>) -> ControlFlow<Self::Break> {
        assert_eq!(self.stack.pop().as_deref(), Some(node.kind()));
        ControlFlow::Continue(())
    }

    fn enter_field(&mut self, field: Field) -> ControlFlow<Self::Break> {
        self.stack.push(field.to_string());
        ControlFlow::Continue(())
    }

    fn leave_field(&mut self, field: Field) -> ControlFlow<Self::Break> {
        assert_eq!(self.stack.pop(), Some(field.to_string()));
        ControlFlow::Continue(())
    }

    fn enter_list(&mut self, _len: usize) -> ControlFlow<Self::Break> {
        self.stack.push("list".into());
        ControlFlow::Continue(())
    }

    fn leave_list(&mut self) -> ControlFlow<Self::Break> {
        assert_eq!(self.stack.pop().as_deref(), Some("list"));
        ControlFlow::Continue(())
    }

    fn visit_scalar(&mut self, scalar: Scalar<'ast>) -> ControlFlow<Self::Break> {
        self.leaves
            .push(format!("{}: {scalar}", self.stack.join("/")));
        ControlFlow::Continue(())
    }

    fn visit_expr(&mut self, expr: &'ast Expr) -> ControlFlow<Self::Break> {
        if let Expr::Parameter(name) = expr {
            self.parameters.push(name);
        }
        visit::walk_expr(self, expr)
    }

    fn visit_identifier(&mut self, identifier: &'ast Identifier) -> ControlFlow<Self::Break> {
        self.identifiers.push(identifier);
        visit::walk_identifier(self, identifier)
    }
}

#[test]
fn traverses_map_keys_values_and_lists_in_order_through_a_trait_object() {
    let ast = parse("RETURN RECORD {first: $a, second: [$b, $c]} AS result").unwrap();
    let mut recorder = Recorder::default();
    let visitor: &mut dyn Visitor<'_, Break = Infallible> = &mut recorder;
    let ControlFlow::Continue(()) = visitor.visit_program(&ast);
    assert_eq!(recorder.parameters, ["a", "b", "c"]);
    assert_eq!(
        recorder
            .identifiers
            .iter()
            .map(|id| id.value.as_str())
            .collect::<Vec<_>>(),
        ["first", "second", "result"]
    );
    assert!(recorder.stack.is_empty());
    assert!(recorder
        .leaves
        .iter()
        .any(|path| path.contains("entries/list/[0]/key/Identifier/value")));
}

#[test]
fn typed_override_can_skip_a_subtree_and_continue_with_siblings() {
    #[derive(Default)]
    struct SkipLists(Vec<String>);
    impl<'ast> Visitor<'ast> for SkipLists {
        type Break = Infallible;
        fn visit_expr(&mut self, expr: &'ast Expr) -> ControlFlow<Self::Break> {
            match expr {
                Expr::List(_) => return ControlFlow::Continue(()),
                Expr::Parameter(name) => self.0.push(name.clone()),
                _ => {}
            }
            visit::walk_expr(self, expr)
        }
    }
    let ast = parse("RETURN $first AS a, [$hidden] AS b, $last AS c").unwrap();
    let mut visitor = SkipLists::default();
    let ControlFlow::Continue(()) = visitor.visit_program(&ast);
    assert_eq!(visitor.0, ["first", "last"]);
}

#[test]
fn break_returns_a_borrowed_value_without_visiting_siblings_or_exit_hooks() {
    #[derive(Default)]
    struct FindFirst {
        visited: usize,
        exited: usize,
    }
    impl<'ast> Visitor<'ast> for FindFirst {
        type Break = &'ast str;
        fn visit_expr(&mut self, expr: &'ast Expr) -> ControlFlow<Self::Break> {
            self.visited += 1;
            if let Expr::Parameter(name) = expr {
                return ControlFlow::Break(name);
            }
            visit::walk_expr(self, expr)
        }
        fn leave_node(&mut self, _node: AstNode<'ast>) -> ControlFlow<Self::Break> {
            self.exited += 1;
            ControlFlow::Continue(())
        }
    }
    let expr = Expr::List(vec![
        Expr::Parameter("first".into()),
        Expr::Parameter("second".into()),
    ]);
    let mut visitor = FindFirst::default();
    assert_eq!(visitor.visit_expr(&expr), ControlFlow::Break("first"));
    assert_eq!(visitor.visited, 2);
    assert_eq!(visitor.exited, 0);
}

#[test]
fn scalar_hook_can_stop_inside_a_container() {
    struct FindInteger;
    impl<'ast> Visitor<'ast> for FindInteger {
        type Break = i64;
        fn visit_scalar(&mut self, value: Scalar<'ast>) -> ControlFlow<Self::Break> {
            if let Scalar::Signed(value) = value {
                ControlFlow::Break(value)
            } else {
                ControlFlow::Continue(())
            }
        }
    }
    let expr = Expr::List(vec![Expr::Literal(Literal::Integer(42)), Expr::Wildcard]);
    assert_eq!(FindInteger.visit_expr(&expr), ControlFlow::Break(42));
}

#[test]
fn reaches_nested_queries_casts_and_definition_initializers() {
    let ast = parse("VALUE limit INTEGER = $initial RETURN CAST($input AS LIST<STRING>) AS items, VALUE { RETURN $nested AS value LIMIT 1 } AS nested").unwrap();
    let mut recorder = Recorder::default();
    let ControlFlow::Continue(()) = recorder.visit_program(&ast);
    assert_eq!(recorder.parameters, ["initial", "input", "nested"]);
    for kind in [
        "ValueVariableDefinition",
        "ValueType::List",
        "ValueType::CharacterString",
        "Expr::ValueQuery",
    ] {
        assert!(recorder.kinds.contains(&kind), "missing {kind}");
    }
    assert!(recorder.stack.is_empty());
}

#[test]
fn visits_all_stored_path_representations() {
    let ast = parse("MATCH (n:Person {score: $score})-[e:KNOWS]->(m) RETURN n").unwrap();
    let mut recorder = Recorder::default();
    let ControlFlow::Continue(()) = recorder.visit_program(&ast);
    assert!(recorder
        .leaves
        .iter()
        .any(|path| path.contains("PathPattern/start/NodePattern")));
    assert!(recorder
        .leaves
        .iter()
        .any(|path| path.contains("PathPattern/chains/list/[0]/PathPatternChain")));
    assert!(recorder
        .leaves
        .iter()
        .any(|path| path.contains("PathPattern/factors/list/[0]/PathPatternFactor::Node")));
    assert!(recorder.stack.is_empty());
}

#[test]
fn balanced_hooks_cover_catalog_types_sessions_and_transactions() {
    for input in [
        "CREATE GRAPH TYPE social AS { NODE Person {name STRING NOT NULL, meta RECORD {tags LIST<STRING>}} }",
        "SESSION SET VALUE $limit INTEGER = 10; SESSION RESET PARAMETER $limit; SESSION CLOSE",
        "START TRANSACTION READ ONLY; COMMIT",
        "CALL () { RETURN 1 AS value } RETURN value",
        "MATCH p = ((a)->(b)){1,2}->(c) | ((x)->(y)) RETURN p",
    ] {
        let ast = parse(input).unwrap();
        let mut recorder = Recorder::default();
        let ControlFlow::Continue(()) = recorder.visit_program(&ast);
        assert!(recorder.stack.is_empty(), "{input}");
        assert_eq!(recorder.kinds.first(), Some(&"Program"));
    }
}
