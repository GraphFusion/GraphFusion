use super::*;
use datafusion::{
    common::ScalarValue,
    logical_expr::{lit, ExprSchemable},
};
use std::collections::{BTreeMap, BTreeSet};

impl Compiler<'_> {
    /// Align exposed bindings, dropping anonymous implementation columns before
    /// union deduplication. Incoming ordinals preserve duplicate input records.
    pub(super) async fn merge(
        &mut self,
        input: Plan,
        scope: &mut Bindings,
        branches: Vec<Built>,
        alternation: ast::PathPatternAlternation,
    ) -> Result<(Plan, State)> {
        let incoming = input.schema().columns();
        let mut names = Vec::new();
        for (_, branch, _) in &branches {
            for name in &branch.order {
                if !scope.contains(name) && !names.contains(name) {
                    names.push(name.clone());
                }
            }
        }
        let mut target = incoming.clone();
        let mut types = incoming
            .iter()
            .map(|c| Ok(Expr::Column(c.clone()).get_type(input.schema())?))
            .collect::<Result<Vec<_>>>()?;
        let mut element_targets = BTreeMap::new();
        for name in &names {
            let mut domain = super::super::references::Domain::new();
            let mut known = true;
            for (_, branch, _) in &branches {
                if !branch.contains(name) {
                    continue;
                }
                if let Some(source) = super::super::references::domain_for_name(name, branch) {
                    domain.extend(source);
                } else {
                    known = false;
                }
            }
            if known {
                scope.domains.insert(name.clone(), domain);
            }
            let (source_plan, source_scope, _) = branches
                .iter()
                .find(|(_, b, _)| b.contains(name))
                .expect("declared name");
            if let Some(source) = source_scope.elements.get(name) {
                let mut element = source.clone();
                element.alias = scope.fresh("union_element");
                let mut columns = Vec::new();
                for (qualifier, field) in source_plan.schema().iter() {
                    if qualifier.is_some_and(|q| q.table() == source.alias) {
                        columns.push(field.name().clone());
                        target.push(Column::new(
                            Some(datafusion::common::TableReference::bare(
                                element.alias.clone(),
                            )),
                            field.name(),
                        ));
                        types.push(field.data_type().clone());
                    }
                }
                element_targets.insert(name.clone(), columns);
                graph::declare(scope, name, &element)?;
            } else {
                let source = source_scope.scalars.get(name).expect("scalar binding");
                let data_type = Expr::Column(source.clone()).get_type(source_plan.schema())?;
                let column = scope.scalar(name);
                target.push(column.clone());
                types.push(data_type);
            }
            let count = branches.iter().filter(|(_, b, _)| b.contains(name)).count();
            if count != branches.len()
                || branches
                    .iter()
                    .any(|(_, b, _)| b.conditional.contains(name))
            {
                scope.conditional.insert(name.clone());
            }
            let groups: BTreeSet<_> = branches
                .iter()
                .filter(|(_, b, _)| b.contains(name))
                .map(|(_, b, _)| b.groups.contains(name))
                .collect();
            if groups.len() > 1 {
                return Err(Error::InvalidQuery(format!(
                    "variable {name} has inconsistent group/singleton exposure"
                )));
            }
            if groups.contains(&true) {
                scope.groups.insert(name.clone());
            }
        }
        let node_name = scope.fresh("union_nodes");
        let edge_name = scope.fresh("union_edges");
        let output_names: Vec<_> = target.iter().map(|_| scope.fresh("union_value")).collect();
        let mut plans = Vec::new();
        for (plan, branch, state) in branches {
            let mut values: Vec<_> = incoming.iter().cloned().map(Expr::Column).collect();
            for name in &names {
                if let Some(fields) = element_targets.get(name) {
                    let target_binding = scope.elements.get(name).expect("element target");
                    match branch.elements.get(name) {
                        Some(source) => {
                            graph::check_binding(
                                source,
                                target_binding.graph,
                                target_binding.kind,
                            )?;
                            values.extend(fields.iter().map(|f| source.column(f)));
                        }
                        None if branch.scalars.contains_key(name) => {
                            return Err(Error::InvalidQuery(format!(
                                "variable {name} has inconsistent path alternative types"
                            )))
                        }
                        None => {
                            for _ in fields {
                                values.push(lit(ScalarValue::Null));
                            }
                        }
                    }
                } else {
                    if branch.elements.contains_key(name) {
                        return Err(Error::InvalidQuery(format!(
                            "variable {name} has inconsistent path alternative types"
                        )));
                    }
                    values.push(
                        branch
                            .scalars
                            .get(name)
                            .map_or_else(|| lit(ScalarValue::Null), |c| Expr::Column(c.clone())),
                    );
                }
            }
            let mut projections = Vec::new();
            for ((value, ty), name) in values.into_iter().zip(&types).zip(&output_names) {
                let actual = value.get_type(plan.schema())?;
                if actual != *ty && actual != datafusion::arrow::datatypes::DataType::Null {
                    return Err(Error::InvalidQuery(
                        "incompatible path-alternative binding types".into(),
                    ));
                }
                let value = if actual == datafusion::arrow::datatypes::DataType::Null {
                    lit(ScalarValue::try_from(ty)?)
                } else {
                    value
                };
                projections.push(value.alias(name));
            }
            projections.extend([state.nodes.alias(&node_name), state.edges.alias(&edge_name)]);
            plans.push(plan.project(projections)?.build()?);
        }
        let mut iter = plans.into_iter();
        let mut plan = Plan::from(
            iter.next()
                .ok_or_else(|| Error::InvalidQuery("empty path alternation".into()))?,
        );
        for branch in iter {
            plan = plan.union(branch)?;
        }
        if alternation == ast::PathPatternAlternation::Union {
            plan = plan.distinct()?;
        }
        let mut output: Vec<_> = target
            .iter()
            .zip(&output_names)
            .map(|(c, n)| {
                Expr::Column(Column::new_unqualified(n))
                    .alias_qualified(c.relation.clone(), &c.name)
            })
            .collect();
        let state = State {
            nodes: Expr::Column(Column::new_unqualified(node_name)),
            edges: Expr::Column(Column::new_unqualified(edge_name)),
        };
        output.extend([state.nodes.clone(), state.edges.clone()]);
        let plan = execution::freeze(plan.project(output)?, self.ctx, self.trace, None).await?;
        Ok((plan, state))
    }
}
