//! Preserve shared projections when optimizing nested COALESCE expressions.
use crate::gql as ast;
use ast::visit::Visitor;
use datafusion::optimizer::{Optimizer, OptimizerRule};
use std::{ops::ControlFlow, sync::Arc};

pub(super) fn needs_shared(chain: &[(Option<ast::YieldClause>, ast::QueryStatement)]) -> bool {
    struct Shared(bool);
    impl<'ast> Visitor<'ast> for Shared {
        type Break = ();
        fn visit_expr(&mut self, expr: &'ast ast::Expr) -> ControlFlow<()> {
            if super::native::needs_shared_coalesce(expr) {
                self.0 = true;
                ControlFlow::Break(())
            } else {
                ast::visit::walk_expr(self, expr)
            }
        }
    }
    chain.iter().any(|(_, query)| {
        let mut visitor = Shared(false);
        let _ = visitor.visit_query_statement(query);
        visitor.0
    })
}

// DF55 filter rewrites and outer-join analysis inline definitions without
// retaining sharing. Keep these logical and physical filters above shared
// projections; native scan predicates are applied before these optimizers.
pub(super) fn logical_rules(shared: bool) -> Vec<Arc<dyn OptimizerRule + Send + Sync>> {
    Optimizer::new()
        .rules
        .into_iter()
        .filter(|rule| {
            !shared || !matches!(rule.name(), "push_down_filter" | "eliminate_outer_join")
        })
        .collect()
}

pub(super) fn physical_rules(
    shared: bool,
) -> Vec<Arc<dyn datafusion::physical_optimizer::PhysicalOptimizerRule + Send + Sync>> {
    datafusion::physical_optimizer::optimizer::PhysicalOptimizer::new()
        .rules
        .into_iter()
        .filter(|rule| !shared || !matches!(rule.name(), "FilterPushdown" | "FilterPushdown(Post)"))
        .collect()
}
