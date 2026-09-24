use super::*;
use datafusion::{
    arrow::datatypes::DataType,
    common::ScalarValue,
    datasource::{cte_worktable::CteWorkTable, provider_as_source},
    functions::core::expr_fn::coalesce,
    functions_nested::expr_fn::{array_concat, make_array},
    logical_expr::{lit, ExprSchemable},
};
use std::sync::Arc;

impl Compiler<'_> {
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn repeat(
        &mut self,
        plan: Plan,
        scope: &mut Bindings,
        group: &ast::ParenthesizedPathPatternExpression,
        state: &State,
        mode: ast::PathMode,
        finite: Option<u64>,
    ) -> Result<(Plan, State)> {
        let hint = narrower(
            finite,
            paths::prefix(group.prefix.as_ref(), self.session).map(|p| self.capacity(p.0))?,
        );
        let (minimum, _) = self.validate(&group.pattern, hint)?;
        if minimum == 0 {
            return Err(Error::InvalidQuery(
                "a quantified path primary must have positive minimum length".into(),
            ));
        }
        let (min, max) = raw_bounds(group.quantifier.as_ref().expect("quantified group"))?;
        let max = narrower(max, finite.map(|n| n / minimum))
            .ok_or_else(|| super::super::unsupported("unbounded repeatable WALK group"))?;
        // Exposed declarations inside a quantified primary are fresh group variables.
        let mut declared = Vec::new();
        declarations(&group.pattern, &mut declared);
        if let Some(name) = &group.variable {
            declared.push(name.value.clone());
        }
        if declared.iter().any(|n| scope.contains(n)) {
            return Err(Error::InvalidQuery(
                "quantified group variable is already bound".into(),
            ));
        }
        let row = scope.fresh("group_row");
        let input = execution::ordinal(plan, self.ctx, self.trace, &row).await?;
        let incoming = input.schema().columns();
        let mut local = scope.clone();
        let (step, step_state) =
            Box::pin(self.group_once(input.clone(), &mut local, group, None, finite)).await?;
        scope.advance(&local);
        let new_names: Vec<_> = local
            .order
            .iter()
            .filter(|n| !scope.contains(n))
            .cloned()
            .collect();
        let step_row = scope.fresh("step_row");
        let step_nodes = scope.fresh("step_nodes");
        let step_edges = scope.fresh("step_edges");
        let mut projections = vec![
            Expr::Column(Column::new_unqualified(&row)).alias(&step_row),
            step_state.nodes.alias(&step_nodes),
            step_state.edges.alias(&step_edges),
        ];
        let mut values = Vec::new();
        for name in &new_names {
            let value = if let Some(element) = local.elements.get(name) {
                let ids = make_array(vec![element.column(ID)]);
                super::super::path_values::references(
                    lit(element.graph),
                    (element.kind == ElementKind::Node).then_some(ids.clone()),
                    (element.kind == ElementKind::Edge).then_some(ids),
                )
            } else {
                let expr = Expr::Column(local.scalars.get(name).expect("local scalar").clone());
                if local.groups.contains(name) {
                    expr
                } else {
                    make_array(vec![expr])
                }
            };
            let DataType::List(field) = value.get_type(step.schema())? else {
                return Err(Error::InvalidQuery("group value must be a list".into()));
            };
            let empty = lit(ScalarValue::List(ScalarValue::new_list(
                &[],
                field.data_type(),
                true,
            )));
            let step_name = scope.fresh("step_value");
            let accumulator = scope.fresh("group_value");
            projections.push(value.alias(&step_name));
            values.push((step_name, accumulator, empty));
        }
        // This step relation may itself contain recursion or union. Materialize it
        // before constructing the outer recursive term, which has one work-table scan.
        let step = execution::freeze(step.project(projections)?, self.ctx, self.trace, None)
            .await?
            .build()?;
        let work_name = scope.fresh("group_work");
        let input_names: Vec<_> = incoming
            .iter()
            .map(|_| scope.fresh("group_input"))
            .collect();
        let nodes = scope.fresh("group_nodes");
        let edges = scope.fresh("group_edges");
        let depth = scope.fresh("group_depth");
        let mut anchor: Vec<_> = incoming
            .iter()
            .zip(&input_names)
            .map(|(c, n)| Expr::Column(c.clone()).alias(n))
            .collect();
        anchor.extend([
            state.nodes.clone().alias(&nodes),
            state.edges.clone().alias(&edges),
            lit(0_u64).alias(&depth),
        ]);
        anchor.extend(
            values
                .iter()
                .map(|(_, acc, empty)| empty.clone().alias(acc)),
        );
        let anchor = input.project(anchor)?;
        let work = Plan::scan(
            &work_name,
            provider_as_source(Arc::new(CteWorkTable::new(
                &work_name,
                Arc::new(anchor.schema().as_arrow().clone()),
            ))),
            None,
        )?;
        let col = |n: &str| Expr::Column(Column::new_unqualified(n));
        let mut restore: Vec<_> = incoming
            .iter()
            .zip(&input_names)
            .map(|(c, n)| {
                execution::col(&work_name, n).alias_qualified(c.relation.clone(), &c.name)
            })
            .collect();
        restore.extend([&nodes, &edges, &depth].map(|n| execution::col(&work_name, n).alias(n)));
        restore.extend(
            values
                .iter()
                .map(|(_, acc, _)| execution::col(&work_name, acc).alias(acc)),
        );
        let current = State {
            nodes: col(&nodes),
            edges: col(&edges),
        };
        let step_state = State {
            nodes: col(&step_nodes),
            edges: col(&step_edges),
        };
        let mut recursive = work
            .project(restore)?
            .filter(col(&depth).lt(lit(max)))?
            .join_on(
                step,
                JoinType::Inner,
                [
                    col(&row).eq(col(&step_row)),
                    current.end().eq(step_state.start()),
                ],
            )?;
        let next = current.concat(&step_state);
        recursive = recursive.filter(next.valid(mode, self.different))?;
        let mut output: Vec<_> = incoming
            .iter()
            .zip(&input_names)
            .map(|(c, n)| Expr::Column(c.clone()).alias(n))
            .collect();
        output.extend([
            next.nodes.alias(&nodes),
            next.edges.alias(&edges),
            (col(&depth) + lit(1_u64)).alias(&depth),
        ]);
        output.extend(values.iter().map(|(value, acc, empty)| {
            array_concat(vec![col(acc), coalesce(vec![col(value), empty.clone()])]).alias(acc)
        }));
        let plan = anchor
            .to_recursive_query(work_name, recursive.project(output)?.build()?, false)?
            .filter(col(&depth).gt_eq(lit(min)))?;
        let mut output: Vec<_> = incoming
            .iter()
            .zip(&input_names)
            .map(|(c, n)| col(n).alias_qualified(c.relation.clone(), &c.name))
            .collect();
        output.extend([col(&nodes), col(&edges)]);
        for (name, (_, acc, _)) in new_names.iter().zip(&values) {
            let column = scope.scalar(name);
            scope.groups.insert(name.clone());
            output.push(col(acc).alias(column.name));
        }
        Ok((plan.project(output)?, current))
    }
}
fn declarations(path: &ast::PathPattern, names: &mut Vec<String>) {
    if let Some(name) = &path.variable {
        names.push(name.value.clone());
    }
    fn factors(fs: &[ast::PathPatternFactor], names: &mut Vec<String>) {
        for f in fs {
            match f {
                ast::PathPatternFactor::Node(n) => {
                    if let Some(n) = &n.variable {
                        names.push(n.value.clone());
                    }
                }
                ast::PathPatternFactor::Relationship(e) => {
                    if let Some(n) = &e.variable {
                        names.push(n.value.clone());
                    }
                }
                ast::PathPatternFactor::Parenthesized(g) => {
                    if let Some(n) = &g.variable {
                        names.push(n.value.clone());
                    }
                    declarations(&g.pattern, names);
                }
                ast::PathPatternFactor::Alternation { alternatives, .. } => {
                    for a in alternatives {
                        factors(a, names);
                    }
                }
            }
        }
    }
    factors(&path.factors, names);
    for a in &path.alternatives {
        factors(&a.factors, names);
    }
}
