use super::*;
use datafusion::{
    arrow::datatypes::DataType,
    common::ScalarValue,
    logical_expr::{cast, lit, ExprSchemable},
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
        let mut element_targets: BTreeMap<String, Vec<String>> = BTreeMap::new();
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
            if source_scope.elements.contains_key(name) {
                // Branches can store different properties for one element. Align
                // them here; copying the first branch's physical fields crashes
                // when a later branch does not have the same column.
                let element = unified_element(&branches, name, scope.fresh("union_element"))?;
                let layout = element_layout(&element);
                let relation = datafusion::common::TableReference::bare(element.alias.clone());
                element_targets.insert(
                    name.clone(),
                    layout.iter().map(|(field, _)| field.clone()).collect(),
                );
                for (field, data_type) in &layout {
                    target.push(Column::new(Some(relation.clone()), field));
                    types.push(data_type.clone());
                }
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
                            values.extend(element_values(source, target_binding)?);
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

fn unified_element(branches: &[Built], name: &str, alias: String) -> Result<graph::ElementBinding> {
    let elements: Vec<_> = branches
        .iter()
        .filter_map(|(_, branch, _)| branch.elements.get(name))
        .collect();
    let Some(first) = elements.first().copied() else {
        return Err(Error::InvalidQuery(
            "incompatible path-alternative binding types".into(),
        ));
    };
    let mut properties = BTreeMap::new();
    let mut label_names = BTreeSet::new();
    let mut source_labels = Vec::new();
    let mut seen_labels = BTreeSet::new();
    let mut constrained = true;
    for element in elements {
        graph::check_binding(element, first.graph, first.kind)?;
        constrained &= element.constrained;
        for labels in &element.source_labels {
            if seen_labels.insert(labels.clone()) {
                source_labels.push(labels.clone());
            }
        }
        label_names.extend(element.labels.keys().cloned());
        for (property, data_type) in &element.properties {
            match properties.get(property).cloned() {
                None => {
                    properties.insert(property.clone(), data_type.clone());
                }
                Some(previous) => {
                    let unified = crate::graph::unify_property_type(&previous, data_type)
                        .ok_or_else(|| {
                            Error::InvalidQuery(
                                "incompatible path-alternative binding types".into(),
                            )
                        })?;
                    properties.insert(property.clone(), unified);
                }
            }
        }
    }
    let present = properties
        .keys()
        .enumerate()
        .map(|(index, property)| (property.clone(), format!("__gf_present_{index}")))
        .collect();
    let labels = label_names
        .into_iter()
        .enumerate()
        .map(|(index, label)| (label, format!("__gf_label_{index}")))
        .collect();
    Ok(graph::ElementBinding {
        alias,
        graph: first.graph,
        kind: first.kind,
        properties,
        labels,
        present,
        constrained: constrained && !source_labels.is_empty(),
        source_labels,
        tombstones: BTreeSet::new(),
        prior_id: None,
    })
}

fn element_layout(element: &graph::ElementBinding) -> Vec<(String, DataType)> {
    let mut fields = vec![(crate::graph::ID.to_string(), DataType::UInt64)];
    if element.kind == graph::ElementKind::Edge {
        fields.extend([
            (crate::graph::SOURCE.to_string(), DataType::UInt64),
            (crate::graph::DESTINATION.to_string(), DataType::UInt64),
            ("__gf_directed".to_string(), DataType::Boolean),
            (crate::graph::FROM.to_string(), DataType::UInt64),
            (crate::graph::TO.to_string(), DataType::UInt64),
        ]);
    }
    for (name, data_type) in &element.properties {
        fields.push((name.clone(), data_type.clone()));
        fields.push((element.present[name].clone(), DataType::Boolean));
    }
    for column in element.labels.values() {
        fields.push((column.clone(), DataType::Boolean));
    }
    fields
}

fn element_values(
    source: &graph::ElementBinding,
    unified: &graph::ElementBinding,
) -> Result<Vec<Expr>> {
    let mut values = vec![source.column(crate::graph::ID)];
    if unified.kind == graph::ElementKind::Edge {
        values.extend([
            source.column(crate::graph::SOURCE),
            source.column(crate::graph::DESTINATION),
            source.column("__gf_directed"),
            source.column(crate::graph::FROM),
            source.column(crate::graph::TO),
        ]);
    }
    for (name, data_type) in &unified.properties {
        values.push(match source.properties.get(name) {
            Some(actual) if actual == data_type => source.column(name),
            Some(_) => cast(source.column(name), data_type.clone()),
            None => lit(ScalarValue::try_from(data_type)?),
        });
        values.push(match source.present.get(name) {
            Some(column) => source.column(column),
            None => lit(false),
        });
    }
    for label in unified.labels.keys() {
        values.push(match source.labels.get(label) {
            Some(column) => source.column(column),
            None => lit(false),
        });
    }
    Ok(values)
}
