//! GQL writes are relational DataFusion plans, published by the graph coordinator.
use super::{
    execution::{batch, col, freeze, memory, Trace},
    expressions::Binder,
    graph::{self, Bindings, ElementBinding, ElementKind},
};
use crate::{
    catalog::ObjectId,
    gql as ast,
    graph::{EdgeTable, GraphData, NodeTable, Table, DESTINATION, ID, SOURCE},
    transaction::StatementTxn,
    Error, Result, SessionState,
};
use datafusion::{
    arrow::{
        array::{Array, UInt64Array},
        datatypes::{DataType, Schema},
        record_batch::RecordBatch,
    },
    common::{Column, TableReference},
    datasource::provider_as_source,
    execution::context::SessionContext,
    logical_expr::{Expr, JoinType, LogicalPlan, LogicalPlanBuilder},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

#[derive(Default)]
pub(super) struct Writes {
    pub graphs: BTreeMap<ObjectId, GraphData>,
    pub affected: usize,
}
impl Writes {
    pub fn data(&self, tx: &mut StatementTxn, id: ObjectId) -> Result<Arc<GraphData>> {
        match self.graphs.get(&id) {
            Some(data) => Ok(Arc::new(data.clone())),
            None => tx.graph_data(id),
        }
    }
}
pub(super) fn is_mutation(clause: &ast::QueryClause) -> bool {
    matches!(
        clause,
        ast::QueryClause::Insert(_)
            | ast::QueryClause::Set(_)
            | ast::QueryClause::Remove(_)
            | ast::QueryClause::Delete(_)
    )
}
async fn table(
    plan: LogicalPlanBuilder,
    labels: BTreeSet<String>,
    edge: bool,
    ctx: &SessionContext,
    trace: &mut Trace,
) -> Result<Option<Table>> {
    table_with_empty(plan, labels, edge, ctx, trace, false).await
}

async fn table_with_empty(
    plan: LogicalPlanBuilder,
    labels: BTreeSet<String>,
    edge: bool,
    ctx: &SessionContext,
    trace: &mut Trace,
    keep_empty: bool,
) -> Result<Option<Table>> {
    let (schema, batches) = trace.collect(ctx, &plan.build()?).await?;
    if !keep_empty && batches.iter().all(|b| b.num_rows() == 0) {
        return Ok(None);
    }
    let schema = Arc::new(Schema::new(
        schema
            .fields()
            .iter()
            .map(|f| {
                if [ID, SOURCE, DESTINATION].contains(&f.name().as_str()) {
                    Arc::new(f.as_ref().clone().with_nullable(false))
                } else {
                    f.clone()
                }
            })
            .collect::<Vec<_>>(),
    ));
    let batches = batches
        .into_iter()
        .map(|b| batch(schema.clone(), b.columns().to_vec(), b.num_rows()))
        .collect::<Result<_>>()?;
    Ok(Some(Table::try_new(
        labels.into_iter().collect(),
        schema,
        batches,
        if edge {
            &[ID, SOURCE, DESTINATION]
        } else {
            &[ID]
        },
    )?))
}
fn source(table: &Table) -> Result<LogicalPlanBuilder> {
    Ok(LogicalPlanBuilder::scan(
        "__gf_old",
        provider_as_source(table.provider.clone()),
        None,
    )?)
}
fn properties(
    map: Option<&ast::MapLiteral>,
    session: &SessionState,
    plan: &LogicalPlanBuilder,
    scope: &Bindings,
) -> Result<Vec<(String, Expr)>> {
    let mut result = Vec::new();
    if let Some(map) = map {
        for (name, value) in &map.entries {
            if name.value.starts_with("__gf_") {
                return Err(Error::InvalidDefinition(
                    "reserved graph property name".into(),
                ));
            }
            if result.iter().any(|(n, _)| n == &name.value) {
                return Err(Error::InvalidDefinition("duplicate property".into()));
            }
            let value = Binder::with_bindings(session, plan.schema(), scope).bind(value)?;
            result.push((name.value.clone(), value));
        }
    }
    Ok(result)
}

fn validate_insert_labels(expression: Option<&ast::LabelExpression>) -> Result<()> {
    match expression {
        None | Some(ast::LabelExpression::Label(_)) => Ok(()),
        Some(ast::LabelExpression::And(left, right)) => {
            validate_insert_labels(Some(left))?;
            validate_insert_labels(Some(right))
        }
        Some(ast::LabelExpression::Parenthesized(inner)) => validate_insert_labels(Some(inner)),
        _ => Err(super::unsupported(
            "INSERT labels must be concrete names joined by &",
        )),
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn apply(
    mut plan: LogicalPlanBuilder,
    scope: &mut Bindings,
    session: &SessionState,
    id: ObjectId,
    clause: &ast::QueryClause,
    tx: &mut StatementTxn,
    ctx: &SessionContext,
    writes: &mut Writes,
    trace: &mut Trace,
) -> Result<LogicalPlanBuilder> {
    let affected_before = writes.affected;
    let mut data = (*writes.data(tx, id)?).clone();
    // Durable providers contain no resident Arrow batches. Materialize their immutable
    // snapshot through DataFusion before validating or rewriting the graph.
    for node in &mut data.nodes {
        if node
            .0
            .batches
            .iter()
            .map(RecordBatch::num_rows)
            .sum::<usize>()
            != node.0.row_count
        {
            node.0 = table(source(&node.0)?, node.0.labels.clone(), false, ctx, trace)
                .await?
                .ok_or_else(|| Error::Corrupt("graph row count changed".into()))?;
        }
    }
    for edge in &mut data.edges {
        if edge
            .table
            .batches
            .iter()
            .map(RecordBatch::num_rows)
            .sum::<usize>()
            != edge.table.row_count
        {
            edge.table = table(
                source(&edge.table)?,
                edge.table.labels.clone(),
                true,
                ctx,
                trace,
            )
            .await?
            .ok_or_else(|| Error::Corrupt("graph row count changed".into()))?;
        }
    }
    plan = freeze(plan, ctx, trace, None).await?;
    match clause {
        ast::QueryClause::Insert(insert) => {
            for path in &insert.patterns {
                let (next, mut left) = insert_node(
                    plan,
                    scope,
                    session,
                    id,
                    &path.start,
                    &mut data,
                    tx,
                    ctx,
                    trace,
                    &mut writes.affected,
                )
                .await?;
                plan = next;
                for chain in &path.chains {
                    let (next, right) = insert_node(
                        plan,
                        scope,
                        session,
                        id,
                        &chain.node,
                        &mut data,
                        tx,
                        ctx,
                        trace,
                        &mut writes.affected,
                    )
                    .await?;
                    plan = next;
                    let edge = &chain.relationship;
                    validate_insert_labels(edge.label_expression.as_ref())?;
                    let (from, to, directed) = match edge.direction {
                        ast::Direction::Right => (left.column(ID), right.column(ID), true),
                        ast::Direction::Left => (right.column(ID), left.column(ID), true),
                        ast::Direction::Undirected => (left.column(ID), right.column(ID), false),
                        _ => return Err(super::unsupported("ambiguous INSERT edge direction")),
                    };
                    let values = properties(edge.properties.as_ref(), session, &plan, scope)?;
                    let (next, _) = insert_element(
                        plan,
                        scope,
                        id,
                        ElementKind::Edge,
                        edge.variable.as_ref(),
                        edge.labels.iter().map(|l| l.value.clone()).collect(),
                        values,
                        Some((from, to, directed)),
                        &mut data,
                        tx,
                        ctx,
                        trace,
                        &mut writes.affected,
                    )
                    .await?;
                    plan = next;
                    left = right;
                }
            }
        }
        ast::QueryClause::Set(set) => {
            for item in &set.items {
                let (name, mut edit, values) = match item {
                    ast::SetItem::Property { target, value } => {
                        let (name, property) = property_target(target)?;
                        let expr =
                            Binder::with_bindings(session, plan.schema(), scope).bind(value)?;
                        (name, Edit::default(), vec![(property, expr)])
                    }
                    ast::SetItem::AllProperties {
                        variable,
                        properties: map,
                    } => (
                        variable.value.clone(),
                        Edit {
                            replace: true,
                            ..Edit::default()
                        },
                        properties(Some(map), session, &plan, scope)?,
                    ),
                    ast::SetItem::Label { variable, label } => (
                        variable.value.clone(),
                        Edit {
                            add_label: Some(label.value.clone()),
                            ..Edit::default()
                        },
                        vec![],
                    ),
                };
                let binding = target(scope, &name, id)?;
                let patch = patch(&plan, &binding, values, &mut edit, ctx, trace).await?;
                if let Some((patch, count)) = patch {
                    data = edit_graph(&data, binding.kind, &patch, &edit, ctx, trace).await?;
                    writes.affected += count;
                    plan = refresh(plan, scope, id, &data)?;
                    plan = freeze(plan, ctx, trace, None).await?;
                }
            }
        }
        ast::QueryClause::Remove(remove) => {
            for item in &remove.items {
                let (name, edit) = match item {
                    ast::RemoveItem::Property(expr) => {
                        let (name, property) = property_target(expr)?;
                        (
                            name,
                            Edit {
                                remove: Some(property),
                                ..Edit::default()
                            },
                        )
                    }
                    ast::RemoveItem::Label { variable, label } => (
                        variable.value.clone(),
                        Edit {
                            remove_label: Some(label.value.clone()),
                            ..Edit::default()
                        },
                    ),
                };
                let binding = target(scope, &name, id)?;
                let mut edit = edit;
                if let Some((patch, count)) =
                    patch(&plan, &binding, vec![], &mut edit, ctx, trace).await?
                {
                    data = edit_graph(&data, binding.kind, &patch, &edit, ctx, trace).await?;
                    writes.affected += count;
                    plan = refresh(plan, scope, id, &data)?;
                    plan = freeze(plan, ctx, trace, None).await?;
                }
            }
        }
        ast::QueryClause::Delete(delete) => {
            let (next, affected) =
                delete_graph(&plan, scope, id, &data, delete, ctx, trace).await?;
            data = next;
            writes.affected += affected;
            // Keep references to surviving elements. Detect deleted aliases as well as
            // explicitly named targets, so later expressions cannot read stale properties.
            let bindings: Vec<_> = scope
                .elements
                .iter()
                .filter(|(_, b)| b.graph == id)
                .map(|(name, binding)| (name.clone(), binding.clone()))
                .collect();
            let mut removed = BTreeSet::new();
            for (name, binding) in bindings {
                let (scan, live) = graph::scan(scope, id, &data, binding.kind, None)?;
                let missing = plan
                    .clone()
                    .filter(binding.column(ID).is_not_null())?
                    .join_on(
                        scan,
                        JoinType::LeftAnti,
                        [binding.column(ID).eq(live.column(ID))],
                    )?
                    .limit(0, Some(1))?
                    .build()?;
                let (_, rows) = trace.collect(ctx, &missing).await?;
                if rows.iter().any(|batch| batch.num_rows() != 0) {
                    removed.insert(name);
                }
            }
            scope.elements.retain(|name, _| !removed.contains(name));
            scope.order.retain(|name| !removed.contains(name));
            plan = refresh(plan, scope, id, &data)?;
        }
        _ => unreachable!("mutation dispatch"),
    }
    if writes.affected != affected_before {
        writes.graphs.insert(id, data);
    }
    Ok(plan)
}

#[allow(clippy::too_many_arguments)]
async fn insert_node(
    plan: LogicalPlanBuilder,
    scope: &mut Bindings,
    session: &SessionState,
    graph: ObjectId,
    node: &ast::NodePattern,
    data: &mut GraphData,
    tx: &mut StatementTxn,
    ctx: &SessionContext,
    trace: &mut Trace,
    affected: &mut usize,
) -> Result<(LogicalPlanBuilder, ElementBinding)> {
    validate_insert_labels(node.label_expression.as_ref())?;
    if let Some(name) = &node.variable {
        if scope.contains(&name.value) {
            let binding = target(scope, &name.value, graph)?;
            if binding.kind != ElementKind::Node
                || !node.labels.is_empty()
                || node.properties.is_some()
            {
                return Err(Error::InvalidQuery(
                    "bound INSERT node must be a bare node reference".into(),
                ));
            }
            return Ok((plan, binding));
        }
    }
    let values = properties(node.properties.as_ref(), session, &plan, scope)?;
    insert_element(
        plan,
        scope,
        graph,
        ElementKind::Node,
        node.variable.as_ref(),
        node.labels.iter().map(|l| l.value.clone()).collect(),
        values,
        None,
        data,
        tx,
        ctx,
        trace,
        affected,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn insert_element(
    mut plan: LogicalPlanBuilder,
    scope: &mut Bindings,
    graph_id: ObjectId,
    kind: ElementKind,
    variable: Option<&ast::Identifier>,
    labels: BTreeSet<String>,
    values: Vec<(String, Expr)>,
    endpoints: Option<(Expr, Expr, bool)>,
    data: &mut GraphData,
    tx: &mut StatementTxn,
    ctx: &SessionContext,
    trace: &mut Trace,
    affected: &mut usize,
) -> Result<(LogicalPlanBuilder, ElementBinding)> {
    if variable.is_some_and(|v| scope.contains(&v.value)) {
        return Err(Error::InvalidQuery(
            "INSERT edge variable already bound".into(),
        ));
    }
    // Count a frozen input with DataFusion. ID allocation is a storage-coordinator operation.
    let (_, batches) = trace.collect(ctx, &plan.clone().build()?).await?;
    let rows = batches.iter().map(RecordBatch::num_rows).sum();
    let ids = tx.allocate_element_ids(graph_id, rows)?;
    let identity = scope.fresh("insert_id");
    plan = freeze(plan, ctx, trace, Some((&identity, ids))).await?;
    let mut expressions = vec![Expr::Column(Column::new_unqualified(&identity)).alias(ID)];
    let mut directed = false;
    if let Some((from, to, is_directed)) = endpoints {
        directed = is_directed;
        expressions.extend([from.alias(SOURCE), to.alias(DESTINATION)]);
    }
    use datafusion::logical_expr::ExprSchemable;
    for (name, value) in values {
        if value.get_type(plan.schema())? != DataType::Null {
            expressions.push(value.alias(name));
        }
    }
    let mut next = data.clone();
    if let Some(table) = table_with_empty(
        plan.clone().project(expressions)?,
        labels,
        kind == ElementKind::Edge,
        ctx,
        trace,
        true,
    )
    .await?
    {
        if kind == ElementKind::Node {
            next.nodes.push(NodeTable(table));
        } else {
            next.edges.push(EdgeTable { table, directed });
        }
    }
    let next = GraphData::try_new(next.nodes, next.edges)?;
    let (scan, binding) = graph::scan(scope, graph_id, &next, kind, None)?;
    // Empty input still needs typed bindings, but must not publish a new layout.
    if rows != 0 {
        *data = next;
        *affected += rows;
    }
    plan = plan.join_on(
        scan,
        JoinType::Inner,
        [Expr::Column(Column::new_unqualified(&identity)).eq(binding.column(ID))],
    )?;
    if let Some(name) = variable {
        graph::declare(scope, &name.value, &binding)?;
    }
    Ok((plan, binding))
}

fn target(scope: &Bindings, name: &str, graph: ObjectId) -> Result<ElementBinding> {
    let binding = scope.element(name)?.clone();
    if binding.graph != graph {
        return Err(Error::InvalidQuery(
            "mutation target belongs to another working graph".into(),
        ));
    }
    Ok(binding)
}
fn property_target(expr: &ast::Expr) -> Result<(String, String)> {
    if let ast::Expr::Property { base, key } = expr {
        if let ast::Expr::Identifier(name) = base.as_ref() {
            if key.value.starts_with("__gf_") {
                return Err(Error::InvalidDefinition(
                    "reserved graph property name".into(),
                ));
            }
            return Ok((name.value.clone(), key.value.clone()));
        }
    }
    Err(Error::InvalidQuery(
        "property mutation requires an element variable".into(),
    ))
}

#[derive(Default)]
struct Edit {
    values: BTreeMap<String, Option<String>>,
    replace: bool,
    remove: Option<String>,
    add_label: Option<String>,
    remove_label: Option<String>,
}
async fn patch(
    plan: &LogicalPlanBuilder,
    binding: &ElementBinding,
    values: Vec<(String, Expr)>,
    edit: &mut Edit,
    ctx: &SessionContext,
    trace: &mut Trace,
) -> Result<Option<(LogicalPlan, usize)>> {
    use datafusion::logical_expr::ExprSchemable;
    let mut expressions = vec![binding.column(ID).alias(ID)];
    for (i, (name, value)) in values.into_iter().enumerate() {
        if value.get_type(plan.schema())? == DataType::Null {
            edit.values.insert(name, None);
        } else {
            let column = format!("__gf_value_{i}");
            expressions.push(value.alias(&column));
            edit.values.insert(name, Some(column));
        }
    }
    let patch = plan
        .clone()
        .filter(binding.column(ID).is_not_null())?
        .project(expressions)?
        .distinct()?
        .build()?;
    let (schema, batches) = trace.collect(ctx, &patch).await?;
    let mut ids = BTreeSet::new();
    for batch in &batches {
        let column = batch
            .column(0)
            .as_any()
            .downcast_ref::<UInt64Array>()
            .ok_or_else(|| Error::Corrupt("invalid patch identity type".into()))?;
        if column.null_count() != 0 {
            return Err(Error::InvalidQuery("null mutation target".into()));
        }
        for id in column.values() {
            if !ids.insert(*id) {
                return Err(super::unsupported(
                    "conflicting values for one element from multiple input rows",
                ));
            }
        }
    }
    if ids.is_empty() {
        return Ok(None);
    }
    Ok(Some((
        memory(schema, batches)?.alias("__gf_patch")?.build()?,
        ids.len(),
    )))
}

async fn edit_graph(
    data: &GraphData,
    kind: ElementKind,
    patch: &LogicalPlan,
    edit: &Edit,
    ctx: &SessionContext,
    trace: &mut Trace,
) -> Result<GraphData> {
    let mut nodes = if kind == ElementKind::Node {
        vec![]
    } else {
        data.nodes.clone()
    };
    let mut edges = if kind == ElementKind::Edge {
        vec![]
    } else {
        data.edges.clone()
    };
    let tables: Vec<_> = if kind == ElementKind::Node {
        data.nodes.iter().map(|t| (&t.0, false)).collect()
    } else {
        data.edges.iter().map(|t| (&t.table, t.directed)).collect()
    };
    for (old, directed) in tables {
        let on = col("__gf_old", ID).eq(col("__gf_patch", ID));
        let kept = source(old)?.join_on(patch.clone(), JoinType::LeftAnti, [on.clone()])?;
        let changed = source(old)?.join_on(patch.clone(), JoinType::Inner, [on])?;
        let mut projections = Vec::new();
        for field in old.schema.fields() {
            let name = field.name();
            if name.starts_with("__gf_")
                || (!edit.replace
                    && !edit.values.contains_key(name)
                    && edit.remove.as_ref() != Some(name))
            {
                projections.push(col("__gf_old", name).alias(name));
            }
        }
        for (name, value) in &edit.values {
            if let Some(column) = value {
                projections.push(col("__gf_patch", column).alias(name));
            }
        }
        let mut labels = old.labels.clone();
        if let Some(label) = &edit.add_label {
            labels.insert(label.clone());
        }
        if let Some(label) = &edit.remove_label {
            labels.remove(label);
        }
        for (plan, labels) in [
            (kept, old.labels.clone()),
            (changed.project(projections)?, labels),
        ] {
            if let Some(table) = table(plan, labels, kind == ElementKind::Edge, ctx, trace).await? {
                if kind == ElementKind::Node {
                    nodes.push(NodeTable(table));
                } else {
                    edges.push(EdgeTable { table, directed });
                }
            }
        }
    }
    GraphData::try_new(nodes, edges)
}

fn refresh(
    mut plan: LogicalPlanBuilder,
    scope: &mut Bindings,
    graph_id: ObjectId,
    data: &GraphData,
) -> Result<LogicalPlanBuilder> {
    let old: Vec<_> = scope
        .elements
        .iter()
        .filter(|(_, b)| b.graph == graph_id)
        .map(|(name, b)| (name.clone(), b.clone()))
        .collect();
    for (name, before) in old {
        let retained: Vec<_> = plan
            .schema()
            .columns()
            .into_iter()
            .filter(|c| c.relation.as_ref() != Some(&TableReference::bare(before.alias.as_str())))
            .map(Expr::Column)
            .collect();
        let (scan, after) = graph::scan(scope, graph_id, data, before.kind, None)?;
        let appended: Vec<_> = scan
            .schema()
            .columns()
            .into_iter()
            .map(Expr::Column)
            .collect();
        plan = plan
            .join_on(
                scan,
                JoinType::Left,
                [before.column(ID).eq(after.column(ID))],
            )?
            .project(retained.into_iter().chain(appended))?;
        scope.elements.insert(name, after);
    }
    Ok(plan)
}

#[allow(clippy::too_many_arguments)]
async fn delete_graph(
    plan: &LogicalPlanBuilder,
    scope: &Bindings,
    graph_id: ObjectId,
    data: &GraphData,
    delete: &ast::DeleteStatement,
    ctx: &SessionContext,
    trace: &mut Trace,
) -> Result<(GraphData, usize)> {
    let mut node_ids: Option<LogicalPlanBuilder> = None;
    let mut edge_ids: Option<LogicalPlanBuilder> = None;
    for item in &delete.items {
        let ast::Expr::Identifier(name) = item else {
            return Err(super::unsupported("DELETE requires element variables"));
        };
        let binding = target(scope, &name.value, graph_id)?;
        let ids = plan.clone().project([binding.column(ID).alias(ID)])?;
        let target = if binding.kind == ElementKind::Node {
            &mut node_ids
        } else {
            &mut edge_ids
        };
        *target = Some(match target.take() {
            Some(previous) => previous.union(ids.build()?)?,
            None => ids,
        });
    }
    let nodes = node_ids
        .map(|p| {
            p.distinct()?
                .alias("__gf_delete")
                .and_then(LogicalPlanBuilder::build)
        })
        .transpose()?;
    let edges = edge_ids
        .map(|p| {
            p.distinct()?
                .alias("__gf_delete")
                .and_then(LogicalPlanBuilder::build)
        })
        .transpose()?;
    let mut new_nodes = Vec::new();
    let mut new_edges = Vec::new();
    for node in &data.nodes {
        let mut plan = source(&node.0)?;
        if let Some(ids) = &nodes {
            plan = plan.join_on(
                ids.clone(),
                JoinType::LeftAnti,
                [col("__gf_old", ID).eq(col("__gf_delete", ID))],
            )?;
        }
        if let Some(table) = table(plan, node.0.labels.clone(), false, ctx, trace).await? {
            new_nodes.push(NodeTable(table));
        }
    }
    for edge in &data.edges {
        let mut plan = source(&edge.table)?;
        if let Some(ids) = &edges {
            plan = plan.join_on(
                ids.clone(),
                JoinType::LeftAnti,
                [col("__gf_old", ID).eq(col("__gf_delete", ID))],
            )?;
        }
        if delete.detach {
            if let Some(ids) = &nodes {
                plan = plan.join_on(
                    ids.clone(),
                    JoinType::LeftAnti,
                    [col("__gf_old", SOURCE).eq(col("__gf_delete", ID)).or(col(
                        "__gf_old",
                        DESTINATION,
                    )
                    .eq(col("__gf_delete", ID)))],
                )?;
            }
        }
        if let Some(table) = table(plan, edge.table.labels.clone(), true, ctx, trace).await? {
            new_edges.push(EdgeTable {
                table,
                directed: edge.directed,
            });
        }
    }
    let next = GraphData::try_new(new_nodes, new_edges)?;
    let affected = data.node_count() + data.edge_count() - next.node_count() - next.edge_count();
    Ok((next, affected))
}
