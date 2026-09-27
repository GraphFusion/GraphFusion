//! Deterministic tree rendering built on the public AST visitor.
//!
//! All stored fields are printed, including defaults, absent options, and empty
//! lists. Field labels distinguish otherwise similar children such as a filter
//! predicate and a result expression. Strings are quoted and escaped so each
//! tree node occupies one physical line. This is a diagnostic format, not a
//! serialization protocol: deliberate AST/schema changes can change the output.

use std::convert::Infallible;
use std::ops::ControlFlow;

use crate::ast::Program;
use crate::visit::{AstNode, Field, Scalar, Visitor};

/// Format a complete program as a tree, ending with a newline.
///
/// ```
/// use gql_parser::{format_ast, parse};
/// let ast = parse("RETURN 1 + 2 * 3 AS value").unwrap();
/// let tree = format_ast(&ast);
/// assert!(tree.starts_with("Program\n"));
/// assert!(tree.contains("op: BinaryOp::Mul"));
/// ```
pub fn format_ast(program: &Program) -> String {
    let mut printer = AstTreePrinter::new();
    let ControlFlow::Continue(()) = printer.visit_program(program);
    printer.finish()
}

#[derive(Debug)]
struct Tree {
    label: String,
    children: Vec<Tree>,
}

/// A visitor that renders programs or individual AST subtrees.
///
/// Visit one or more roots, then consume the printer with [`Self::finish`].
/// Node order is deterministic and follows the visitor's structural order.
///
/// ```
/// use std::ops::ControlFlow;
/// use gql_parser::{AstTreePrinter, Expr};
/// use gql_parser::visit::Visitor;
/// let mut printer = AstTreePrinter::new();
/// let ControlFlow::Continue(()) = printer.visit_expr(&Expr::Wildcard);
/// assert_eq!(printer.finish(), "Expr::Wildcard\n");
/// ```
#[derive(Debug, Default)]
pub struct AstTreePrinter {
    roots: Vec<Tree>,
    stack: Vec<Tree>,
}

impl AstTreePrinter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Finish rendering. An unused printer returns an empty string.
    ///
    /// # Panics
    /// Panics if entry/exit hooks were manually called without balancing them.
    /// Normal typed `visit_*` calls always balance the hooks.
    pub fn finish(self) -> String {
        assert!(self.stack.is_empty(), "unfinished AST traversal");
        let mut output = String::new();
        for root in &self.roots {
            output.push_str(&root.label);
            output.push('\n');
            // Rendering uses an explicit stack rather than recursing a second
            // time through deeply nested expression trees.
            let mut pending = Vec::new();
            for (index, child) in root.children.iter().enumerate().rev() {
                pending.push((child, String::new(), index + 1 == root.children.len()));
            }
            while let Some((tree, prefix, last)) = pending.pop() {
                output.push_str(&prefix);
                output.push_str(if last { "└── " } else { "├── " });
                output.push_str(&tree.label);
                output.push('\n');
                let child_prefix = prefix + if last { "    " } else { "│   " };
                for (index, child) in tree.children.iter().enumerate().rev() {
                    pending.push((
                        child,
                        child_prefix.clone(),
                        index + 1 == tree.children.len(),
                    ));
                }
            }
        }
        output
    }

    fn push(&mut self, label: String) {
        self.stack.push(Tree {
            label,
            children: Vec::new(),
        });
    }

    fn append(&mut self, tree: Tree) {
        if let Some(parent) = self.stack.last_mut() {
            parent.children.push(tree);
        } else {
            self.roots.push(tree);
        }
    }

    fn pop(&mut self, field: bool) {
        let mut tree = self.stack.pop().expect("balanced AST traversal");
        if field && tree.children.len() == 1 {
            let child = tree.children.pop().expect("one child");
            tree.label.push_str(": ");
            tree.label.push_str(&child.label);
            tree.children = child.children;
        }
        self.append(tree);
    }
}

impl<'ast> Visitor<'ast> for AstTreePrinter {
    type Break = Infallible;

    fn enter_node(&mut self, node: AstNode<'ast>) -> ControlFlow<Self::Break> {
        self.push(node.kind().to_owned());
        ControlFlow::Continue(())
    }

    fn leave_node(&mut self, _node: AstNode<'ast>) -> ControlFlow<Self::Break> {
        self.pop(false);
        ControlFlow::Continue(())
    }

    fn enter_field(&mut self, field: Field) -> ControlFlow<Self::Break> {
        self.push(field.to_string());
        ControlFlow::Continue(())
    }

    fn leave_field(&mut self, _field: Field) -> ControlFlow<Self::Break> {
        self.pop(true);
        ControlFlow::Continue(())
    }

    fn enter_list(&mut self, len: usize) -> ControlFlow<Self::Break> {
        self.push(if len == 0 {
            "[]".to_owned()
        } else {
            format!("List (len={len})")
        });
        ControlFlow::Continue(())
    }

    fn leave_list(&mut self) -> ControlFlow<Self::Break> {
        self.pop(false);
        ControlFlow::Continue(())
    }

    fn visit_scalar(&mut self, value: Scalar<'ast>) -> ControlFlow<Self::Break> {
        self.append(Tree {
            label: value.to_string(),
            children: Vec::new(),
        });
        ControlFlow::Continue(())
    }
}
