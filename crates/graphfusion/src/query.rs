//! GQL binding and direct DataFusion logical/physical planning.
mod aggregates;
mod arithmetic;
mod binding;
mod casefold;
mod execution;
mod expressions;
mod graph;
mod lists;
mod mutations;
mod native;
mod path_values;
mod paths;
mod patterns;
mod references;
mod relational;

use crate::{
    catalog::CommitSeq, gql as ast, transaction::StatementTxn, Error, Result, SessionState, Value,
};
use datafusion::{
    arrow::{datatypes::SchemaRef, record_batch::RecordBatch},
    common::Column,
    execution::context::SessionContext,
    logical_expr::{Expr, LogicalPlan, LogicalPlanBuilder},
};
use expressions::Binder;
use std::collections::BTreeMap;

pub(crate) fn write_query(statement: &ast::Statement) -> Option<ast::QueryStatement> {
    let clause = match statement {
        ast::Statement::Query(query) => return Some(query.clone()),
        ast::Statement::Insert(s) => ast::QueryClause::Insert(s.clone()),
        ast::Statement::Set(s) => ast::QueryClause::Set(s.clone()),
        ast::Statement::Remove(s) => ast::QueryClause::Remove(s.clone()),
        ast::Statement::Delete(s) => ast::QueryClause::Delete(s.clone()),
        _ => return None,
    };
    Some(ast::QueryStatement {
        at_schema: None,
        use_graph: None,
        set_operations: vec![],
        body: ast::QueryBody {
            clauses: vec![clause],
            result_clause: ast::ResultClause {
                kind: ast::ResultKind::Finish,
                quantifier: None,
                distinct: false,
                items: vec![],
            },
            select_from: vec![],
            select_query: None,
            select_where: None,
            group_by: vec![],
            empty_grouping_set: false,
            having: None,
            order_by: vec![],
            offset: None,
            limit: None,
        },
    })
}

#[derive(Debug)]
pub struct QueryResult {
    /// Available even when execution returns no batches/rows.
    pub schema: SchemaRef,
    pub batches: Vec<RecordBatch>,
    pub commit_seq: CommitSeq,
    pub logical_plan: String,
    pub physical_plan: String,
    /// Inserted/deleted elements plus distinct targets per SET/REMOVE item.
    pub affected_elements: usize,
    /// Produced inside an open explicit transaction. commit_seq is its base snapshot.
    /// This flag does not change later; the COMMIT result acknowledges durability.
    pub transaction_pending: bool,
    #[cfg(feature = "visualization")]
    pub graph: crate::visualization::GraphProjection,
}

impl QueryResult {
    pub fn row_count(&self) -> usize {
        self.batches.iter().map(RecordBatch::num_rows).sum()
    }
}

pub(crate) async fn execute_statement(
    session: &SessionState,
    tx: &mut StatementTxn,
    query: &ast::QueryStatement,
    at_schema: Option<&ast::SchemaReference>,
    allow_writes: bool,
) -> Result<QueryResult> {
    execute_chain(
        session,
        tx,
        &[(None, query.clone())],
        at_schema,
        allow_writes,
    )
    .await
}

pub(crate) async fn execute_chain(
    session: &SessionState,
    tx: &mut StatementTxn,
    chain: &[(Option<ast::YieldClause>, ast::QueryStatement)],
    at_schema: Option<&ast::SchemaReference>,
    allow_writes: bool,
) -> Result<QueryResult> {
    // The caller owns the transaction lease across planning and materialization.
    let mut context = session.clone();
    if let Some(schema) = at_schema {
        context.current_schema = crate::session::schema_reference(tx, session, schema)?;
    }
    let runtime = datafusion::execution::runtime_env::RuntimeEnvBuilder::new().with_memory_pool(
        std::sync::Arc::new(datafusion::execution::memory_pool::GreedyMemoryPool::new(
            session.query_limits.memory_limit_bytes,
        )),
    );
    #[cfg(target_family = "wasm")]
    let runtime = runtime.with_disk_manager_builder(
        datafusion::execution::disk_manager::DiskManagerBuilder::default()
            .with_mode(datafusion::execution::disk_manager::DiskManagerMode::Disabled),
    );
    let runtime = runtime.build()?;
    // DataFusion's leaf-projection pushdown joins a qualified table-scan schema
    // (`__gf_scan_N.__gf_id`) with the unqualified `__gf_id` on our scan
    // projection and rejects the plan as ambiguous. That shows up as soon as a
    // graph has a single table for an element kind.
    let mut config = datafusion::execution::config::SessionConfig::new();
    // Browser workers have one executor thread and no filesystem for spilling.
    #[cfg(target_family = "wasm")]
    {
        config = config.with_target_partitions(1);
    }
    config
        .options_mut()
        .optimizer
        .enable_leaf_expression_pushdown = false;
    let ctx = SessionContext::new_with_config_rt(config, std::sync::Arc::new(runtime));
    let mut writes = mutations::Writes::default();
    let mut trace = execution::Trace::default();
    let plan = drive_chain(
        &context,
        tx,
        &ctx,
        chain,
        allow_writes,
        &mut writes,
        &mut trace,
    )
    .await?;
    let (schema, batches) = trace.collect(&ctx, &plan).await?;
    #[cfg(feature = "visualization")]
    let graph = crate::visualization::project(&batches, &trace.sources, &ctx).await?;
    for (graph, data) in writes.graphs {
        tx.replace_graph_data(graph, data)?;
    }
    let commit_seq = tx.base.commit_seq;
    Ok(QueryResult {
        schema,
        batches,
        commit_seq,
        logical_plan: trace.logical.join("\n"),
        physical_plan: trace.physical.join("\n"),
        affected_elements: writes.affected,
        transaction_pending: true,
        #[cfg(feature = "visualization")]
        graph,
    })
}

async fn drive_chain(
    session: &SessionState,
    tx: &mut StatementTxn,
    ctx: &SessionContext,
    chain: &[(Option<ast::YieldClause>, ast::QueryStatement)],
    allow_writes: bool,
    writes: &mut mutations::Writes,
    trace: &mut execution::Trace,
) -> Result<LogicalPlan> {
    if chain.is_empty() {
        return Err(Error::InvalidQuery("query chain is empty".into()));
    }
    let mut carried_graph = session.current_graph;
    let mut carried_domains = references::Domains::new();
    let mut carried_sources = references::ReferenceSources::new();
    let mut working = None;
    let last = chain.len() - 1;
    for (index, (yield_clause, query)) in chain.iter().enumerate() {
        let mut step = session.clone();
        step.current_graph = carried_graph;
        let incoming = if let Some(plan) = working.take() {
            let plan = apply_yield(
                plan,
                yield_clause.as_ref(),
                &mut carried_domains,
                &mut carried_sources,
            )?;
            Some(WorkingTable {
                plan,
                domains: carried_domains.clone(),
                reference_sources: carried_sources.clone(),
            })
        } else if yield_clause.is_some() {
            return Err(Error::InvalidQuery(
                "YIELD requires a preceding query".into(),
            ));
        } else {
            None
        };
        let (plan, graph) =
            plan(query, &step, tx, ctx, writes, trace, allow_writes, incoming).await?;
        carried_graph = graph.or(carried_graph);
        carried_domains = trace.output_domains.clone();
        carried_sources = trace.output_reference_sources.clone();
        if index == last {
            return Ok(plan);
        }
        let (schema, batches) = trace.collect(ctx, &plan).await?;
        working = Some(execution::memory_named("__gf_working", schema, batches)?.build()?);
    }
    Err(Error::InvalidQuery("query chain is empty".into()))
}

fn apply_yield(
    plan: LogicalPlan,
    yield_clause: Option<&ast::YieldClause>,
    domains: &mut references::Domains,
    reference_sources: &mut references::ReferenceSources,
) -> Result<LogicalPlan> {
    let Some(yield_clause) = yield_clause else {
        return Ok(plan);
    };
    if yield_clause
        .items
        .iter()
        .any(|item| matches!(item, ast::YieldItem::All))
    {
        if yield_clause.items.len() != 1 {
            return Err(Error::InvalidQuery(
                "YIELD * must be the only yield item".into(),
            ));
        }
        return Ok(plan);
    }
    let mut projections = Vec::new();
    let mut renamed = BTreeMap::new();
    let mut seen = casefold::CaseFoldSet::new();
    for item in &yield_clause.items {
        let ast::YieldItem::Item { name, alias } = item else {
            continue;
        };
        let output = alias.as_ref().unwrap_or(name);
        if !seen.insert(output.value.clone()) {
            return Err(Error::InvalidQuery(format!(
                "duplicate yield column {}",
                output.value
            )));
        }
        let column = plan
            .schema()
            .columns()
            .into_iter()
            .find(|column| column.name().eq_ignore_ascii_case(&name.value))
            .ok_or_else(|| Error::InvalidQuery(format!("unbound yield column {}", name.value)))?;
        projections.push(Expr::Column(column).alias(&output.value));
        renamed.insert(output.value.clone(), name.value.clone());
    }
    if projections.is_empty() {
        return Err(Error::InvalidQuery("YIELD requires a column".into()));
    }
    let mut next_domains = references::Domains::new();
    for (output, source) in &renamed {
        if let Some(domain) = domains.get(source).or_else(|| {
            domains
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(source))
                .map(|(_, domain)| domain)
        }) {
            next_domains.insert(output.clone(), domain.clone());
        }
    }
    *reference_sources = renamed
        .iter()
        .filter_map(|(output, source)| {
            reference_sources
                .get(source)
                .map(|binding| (output.clone(), binding.clone()))
        })
        .collect();
    *domains = next_domains;
    Ok(LogicalPlanBuilder::from(plan)
        .project(projections)?
        .build()?)
}

fn unsupported(feature: &str) -> Error {
    Error::UnsupportedFeature(feature.into())
}

struct WorkingTable {
    plan: LogicalPlan,
    domains: references::Domains,
    reference_sources: references::ReferenceSources,
}

fn clone_working(table: &Option<WorkingTable>) -> Option<WorkingTable> {
    table.as_ref().map(|table| WorkingTable {
        plan: table.plan.clone(),
        domains: table.domains.clone(),
        reference_sources: table.reference_sources.clone(),
    })
}

#[allow(clippy::too_many_arguments)]
async fn plan(
    query: &ast::QueryStatement,
    initial: &SessionState,
    tx: &mut StatementTxn,
    ctx: &SessionContext,
    writes: &mut mutations::Writes,
    trace: &mut execution::Trace,
    allow_writes: bool,
    incoming: Option<WorkingTable>,
) -> Result<(LogicalPlan, Option<crate::catalog::ObjectId>)> {
    let mut context = initial.clone();
    if let Some(schema) = &query.at_schema {
        context.current_schema = crate::session::schema_reference(tx, initial, schema)?;
    }
    if !allow_writes && has_mutations(query) {
        return Err(unsupported(
            "query() is read-only; use run() for graph mutations",
        ));
    }
    if !query.set_operations.is_empty() && has_mutations(query) {
        return Err(unsupported("mutations in composite queries"));
    }
    let mut first_context = context.clone();
    if let Some(graph) = &query.use_graph {
        first_context.current_graph = Some(crate::session::resolve_graph(tx, &context, graph)?);
    }
    let otherwise = query
        .set_operations
        .iter()
        .any(|op| op.operator == ast::QuerySetOperator::Otherwise);
    // A trailing ORDER BY, OFFSET, or LIMIT belongs to the combined rows. The
    // parser stores it on the last branch, which would page that branch alone.
    let trailing_page = query.set_operations.last().filter(|op| {
        !op.body.order_by.is_empty() || op.body.offset.is_some() || op.body.limit.is_some()
    });
    if otherwise {
        // Bind every branch before execution. Empty barrier results retain their schema,
        // so a cold branch is checked without evaluating any of its expressions.
        let mut validation = execution::Trace {
            validate_only: true,
            ..Default::default()
        };
        let (mut checked, graph) = plan_body(
            &query.body,
            &first_context,
            tx,
            ctx,
            writes,
            &mut validation,
            clone_working(&incoming),
        )
        .await?;
        let mut checked_domains = validation.output_domains.clone();
        let mut checked_sources = validation.output_reference_sources.clone();
        for (index, op) in query.set_operations.iter().enumerate() {
            let body = without_trailing_page(query, index, &op.body);
            let (right, _) = plan_body(
                &body,
                &context,
                tx,
                ctx,
                writes,
                &mut validation,
                clone_working(&incoming),
            )
            .await?;
            merge_result_metadata(&mut checked_domains, &mut checked_sources, &validation)?;
            checked =
                relational::set_operation(checked, right, ast::QuerySetOperator::Union, true)?;
        }
        validation.output_domains = checked_domains;
        validation.output_reference_sources = checked_sources;
        let schema = checked.schema().clone();
        if trace.validate_only {
            let checked =
                apply_trailing_page(checked, &context, trailing_page, ctx, &mut validation).await?;
            trace.sources.extend(validation.sources);
            trace.output_domains = validation.output_domains;
            trace.output_reference_sources = validation.output_reference_sources;
            return Ok((checked, graph));
        }
        for (index, (body, context)) in std::iter::once((&query.body, &first_context))
            .chain(query.set_operations.iter().map(|op| (&op.body, &context)))
            .enumerate()
        {
            // Index 0 is the leading query. Later indexes are set-operation branches.
            let body = if index == 0 {
                std::borrow::Cow::Borrowed(body)
            } else {
                without_trailing_page(query, index - 1, body)
            };
            let (branch, branch_graph) = plan_body(
                &body,
                context,
                tx,
                ctx,
                writes,
                trace,
                clone_working(&incoming),
            )
            .await?;
            let branch = relational::coerce(branch, &schema)?;
            let (schema, batches) = trace.collect(ctx, &branch).await?;
            if batches.iter().any(|b| b.num_rows() > 0) {
                let chosen = execution::memory(schema, batches)?.build()?;
                return Ok((
                    apply_trailing_page(chosen, context, trailing_page, ctx, trace).await?,
                    branch_graph,
                ));
            }
        }
        let empty =
            execution::memory(std::sync::Arc::new(schema.as_arrow().clone()), vec![])?.build()?;
        return Ok((
            apply_trailing_page(empty, &context, trailing_page, ctx, trace).await?,
            graph,
        ));
    }
    let (mut result, graph) = plan_body(
        &query.body,
        &first_context,
        tx,
        ctx,
        writes,
        trace,
        clone_working(&incoming),
    )
    .await?;
    let mut domains = trace.output_domains.clone();
    let mut reference_sources = trace.output_reference_sources.clone();
    for (index, op) in query.set_operations.iter().enumerate() {
        let body = without_trailing_page(query, index, &op.body);
        let (right, _) = plan_body(
            &body,
            &context,
            tx,
            ctx,
            writes,
            trace,
            clone_working(&incoming),
        )
        .await?;
        if op.operator == ast::QuerySetOperator::Union {
            merge_result_metadata(&mut domains, &mut reference_sources, trace)?;
        }
        result = relational::set_operation(
            result,
            right,
            op.operator,
            op.quantifier == Some(ast::SetQuantifier::All),
        )?;
    }
    trace.output_domains = domains;
    trace.output_reference_sources = reference_sources;
    Ok((
        apply_trailing_page(result, &context, trailing_page, ctx, trace).await?,
        graph,
    ))
}

fn merge_result_metadata(
    domains: &mut references::Domains,
    sources: &mut references::ReferenceSources,
    right: &execution::Trace,
) -> Result<()> {
    references::merge_sources(
        sources,
        &right.output_reference_sources,
        domains,
        &right.output_domains,
        &right.sources,
    )?;
    domains.retain(|name, domain| {
        if let Some(right) = right.output_domains.get(name) {
            domain.extend(right.iter().copied());
            true
        } else {
            false
        }
    });
    Ok(())
}

fn without_trailing_page<'a>(
    query: &ast::QueryStatement,
    set_index: usize,
    body: &'a ast::QueryBody,
) -> std::borrow::Cow<'a, ast::QueryBody> {
    if set_index + 1 != query.set_operations.len()
        || (body.order_by.is_empty() && body.offset.is_none() && body.limit.is_none())
    {
        return std::borrow::Cow::Borrowed(body);
    }
    let mut body = body.clone();
    body.order_by.clear();
    body.offset = None;
    body.limit = None;
    std::borrow::Cow::Owned(body)
}

async fn apply_trailing_page(
    plan: LogicalPlan,
    session: &SessionState,
    trailing: Option<&ast::QuerySetOperation>,
    ctx: &SessionContext,
    trace: &mut execution::Trace,
) -> Result<LogicalPlan> {
    let Some(trailing) = trailing else {
        return Ok(plan);
    };
    if trailing.body.order_by.is_empty() {
        return Ok(order_and_page(
            LogicalPlanBuilder::from(plan),
            session,
            &[],
            trailing.body.offset.as_ref(),
            trailing.body.limit.as_ref(),
            None,
        )?
        .build()?);
    }
    let plan = freeze_page_before_sort(LogicalPlanBuilder::from(plan), true, ctx, trace)
        .await?
        .build()?;
    let columns = plan.schema().columns();
    let mut scope = graph::Bindings::default();
    for column in &columns {
        scope.scalars.insert(column.name.clone(), column.clone());
        scope.order.push(column.name.clone());
    }
    scope.sources = trace.sources.clone();
    scope.domains = trace.output_domains.clone();
    scope.reference_sources = trace.output_reference_sources.clone();
    let plan = references::prepare(LogicalPlanBuilder::from(plan), &mut scope, ctx, trace).await?;
    Ok(order_and_page(
        plan,
        session,
        &trailing.body.order_by,
        trailing.body.offset.as_ref(),
        trailing.body.limit.as_ref(),
        Some(&mut scope),
    )?
    .project(columns.into_iter().map(Expr::Column))?
    .build()?)
}

pub(crate) fn has_mutations(query: &ast::QueryStatement) -> bool {
    std::iter::once(&query.body)
        .chain(query.set_operations.iter().map(|op| &op.body))
        .any(|body| {
            body.clauses.iter().any(|clause| {
                mutations::is_mutation(clause)
                    || match clause {
                        ast::QueryClause::NestedQuery(query) => has_mutations(query),
                        ast::QueryClause::BindingTable(
                            ast::BindingTableExpression::NestedQuery(query),
                        ) => has_mutations(query),
                        _ => false,
                    }
            }) || body
                .select_query
                .as_ref()
                .is_some_and(|s| has_mutations(&s.query))
        })
}

async fn plan_body(
    body: &ast::QueryBody,
    session: &SessionState,
    tx: &mut StatementTxn,
    ctx: &SessionContext,
    writes: &mut mutations::Writes,
    trace: &mut execution::Trace,
    incoming: Option<WorkingTable>,
) -> Result<(LogicalPlan, Option<crate::catalog::ObjectId>)> {
    let mut current_graph = session.current_graph;
    let mut scope = graph::Bindings::default();
    let mut plan = if let Some(incoming) = incoming {
        let plan = LogicalPlanBuilder::from(incoming.plan);
        for (qualifier, field) in plan.schema().iter() {
            let name = field.name().to_string();
            scope
                .scalars
                .insert(name.clone(), Column::new(qualifier.cloned(), field.name()));
            scope.order.push(name);
        }
        scope.domains = incoming.domains;
        scope.reference_sources = incoming.reference_sources;
        // NEXT materializes the working table, so the next statement no longer
        // has the previous MATCH bindings. Element references still name rows
        // in the graphs that statement read or wrote; without those snapshots
        // a live element looks deleted.
        scope.sources.extend(trace.sources.clone());
        let mut missing = Vec::new();
        for domain in scope.domains.values() {
            for (id, _) in domain {
                if !scope.sources.contains_key(id) && !missing.contains(id) {
                    missing.push(*id);
                }
            }
        }
        for id in missing {
            scope.sources.insert(id, writes.data(tx, id)?);
        }
        references::prepare(plan, &mut scope, ctx, trace).await?
    } else {
        LogicalPlanBuilder::empty(true)
    };
    for clause in &body.clauses {
        match clause {
            ast::QueryClause::UseGraph(expression) => {
                current_graph = Some(crate::session::resolve_graph(tx, session, expression)?);
            }
            ast::QueryClause::Match(clause) => {
                let id = current_graph
                    .ok_or_else(|| Error::InvalidReference("current graph is unset".into()))?;
                let data = writes.data(tx, id)?;
                plan = if clause.optional {
                    let mut mandatory = clause.clone();
                    mandatory.optional = false;
                    relational::optional(
                        plan,
                        &mut scope,
                        session,
                        id,
                        &data,
                        std::slice::from_ref(&mandatory),
                        ctx,
                        trace,
                    )
                    .await?
                } else {
                    graph::matches(plan, &mut scope, session, id, &data, clause, ctx, trace).await?
                };
            }
            ast::QueryClause::OptionalMatchBlock(clauses) => {
                let id = current_graph
                    .ok_or_else(|| Error::InvalidReference("current graph is unset".into()))?;
                let data = writes.data(tx, id)?;
                plan =
                    relational::optional(plan, &mut scope, session, id, &data, clauses, ctx, trace)
                        .await?;
            }
            ast::QueryClause::BindingTable(expression) => {
                let saved_domains = trace.output_domains.clone();
                let nested = binding::source(
                    expression,
                    session,
                    current_graph,
                    tx,
                    ctx,
                    writes,
                    trace,
                    !scope.order.is_empty(),
                )
                .await?;
                let nested_domains = std::mem::replace(&mut trace.output_domains, saved_domains);
                scope.sources.extend(trace.sources.clone());
                plan = relational::derived(plan, &mut scope, nested)?;
                scope.domains.extend(nested_domains);
                plan = references::prepare(plan, &mut scope, ctx, trace).await?;
            }
            ast::QueryClause::NestedQuery(query) => {
                if !scope.order.is_empty() {
                    return Err(unsupported("correlated nested queries"));
                }
                let mut nested_context = session.clone();
                nested_context.current_graph = current_graph;
                let (nested, _) = Box::pin(self::plan(
                    query,
                    &nested_context,
                    tx,
                    ctx,
                    writes,
                    trace,
                    false,
                    None,
                ))
                .await?;
                scope.sources.extend(trace.sources.clone());
                scope.domains.extend(trace.output_domains.clone());
                plan = relational::derived(plan, &mut scope, nested)?;
                plan = references::prepare(plan, &mut scope, ctx, trace).await?;
            }
            ast::QueryClause::For(clause) => {
                plan =
                    relational::for_clause(plan, &mut scope, session, clause, ctx, trace).await?;
            }
            ast::QueryClause::Let(clause) => {
                for item in &clause.items {
                    if item.typed || item.value_type.is_some() {
                        return Err(unsupported("typed LET definitions"));
                    }
                    if scope.contains(&item.name.value) {
                        return Err(Error::InvalidQuery(format!(
                            "variable {} is already bound",
                            item.name.value
                        )));
                    }
                    let columns = plan.schema().columns();
                    let (next, mixed) =
                        native::fragment_boolean(plan, &mut scope, session, &item.value)?;
                    plan = next;
                    let expression = if let Some(expr) = mixed {
                        expr
                    } else {
                        Binder::with_bindings(session, plan.schema(), &scope).bind(&item.value)?
                    };
                    let mut expressions: Vec<_> = columns.into_iter().map(Expr::Column).collect();
                    if let ast::Expr::Identifier(source) = &item.value {
                        if let Some(element) = scope
                            .elements
                            .get(&source.value)
                            .map(|binding| vec![binding.clone()])
                            .or_else(|| scope.reference_sources.get(&source.value).cloned())
                        {
                            scope
                                .reference_sources
                                .insert(item.name.value.clone(), element.clone());
                        }
                    }
                    let domain = references::domain(&item.value, &scope);
                    let column = scope.scalar(&item.name.value);
                    if let Some(domain) = domain {
                        scope.domains.insert(item.name.value.clone(), domain);
                    }
                    expressions.push(expression.alias(column.name));
                    plan = plan.project(expressions)?;
                    plan = references::prepare(plan, &mut scope, ctx, trace).await?;
                }
            }
            ast::QueryClause::Filter(clause) => {
                if let Some(filtered) = native::fragment_predicate(
                    plan.clone(),
                    &mut scope,
                    session,
                    &clause.predicate,
                )? {
                    plan = filtered;
                } else {
                    let expr = Binder::with_bindings(session, plan.schema(), &scope)
                        .predicate(&clause.predicate)?;
                    plan = plan.filter(expr)?;
                }
            }
            ast::QueryClause::OrderByPage(clause) => {
                plan =
                    freeze_page_before_sort(plan, !clause.order_by.is_empty(), ctx, trace).await?;
                plan = order_and_page(
                    plan,
                    session,
                    &clause.order_by,
                    clause.offset.as_ref(),
                    clause.limit.as_ref(),
                    Some(&mut scope),
                )?;
            }
            clause if mutations::is_mutation(clause) => {
                let id = current_graph
                    .ok_or_else(|| Error::InvalidReference("current graph is unset".into()))?;
                plan = mutations::apply(
                    plan, &mut scope, session, id, clause, tx, ctx, writes, trace,
                )
                .await?;
            }
            _ => {
                return Err(unsupported(
                    "graph, procedure, or data-modifying query clause",
                ))
            }
        }
    }
    for from in &body.select_from {
        let id = crate::session::resolve_graph(tx, session, &from.graph)?;
        let data = writes.data(tx, id)?;
        plan = if from.match_clause.optional {
            let mut mandatory = from.match_clause.clone();
            mandatory.optional = false;
            relational::optional(
                plan,
                &mut scope,
                session,
                id,
                &data,
                std::slice::from_ref(&mandatory),
                ctx,
                trace,
            )
            .await?
        } else {
            graph::matches(
                plan,
                &mut scope,
                session,
                id,
                &data,
                &from.match_clause,
                ctx,
                trace,
            )
            .await?
        };
    }
    if let Some(from) = &body.select_query {
        let mut nested_context = session.clone();
        nested_context.current_graph = if let Some(graph) = &from.graph {
            Some(crate::session::resolve_graph(tx, session, graph)?)
        } else {
            current_graph
        };
        let (nested, _) = Box::pin(self::plan(
            &from.query,
            &nested_context,
            tx,
            ctx,
            writes,
            trace,
            false,
            None,
        ))
        .await?;
        scope.sources.extend(trace.sources.clone());
        scope.domains.extend(trace.output_domains.clone());
        plan = relational::derived(plan, &mut scope, nested)?;
        plan = references::prepare(plan, &mut scope, ctx, trace).await?;
    }
    if body.result_clause.kind == ast::ResultKind::Finish {
        trace.output_domains.clear();
        trace.output_reference_sources.clear();
        trace.collect(ctx, &plan.build()?).await?;
        return Ok((LogicalPlanBuilder::empty(false).build()?, current_graph));
    }
    trace.sources.extend(scope.sources.clone());
    trace.output_domains = references::result_domains(body, &scope);
    trace.output_reference_sources = references::result_sources(body, &scope);
    if let Some(predicate) = &body.select_where {
        if let Some(filtered) =
            native::fragment_predicate(plan.clone(), &mut scope, session, predicate)?
        {
            plan = filtered;
        } else {
            let expr =
                Binder::with_bindings(session, plan.schema(), &scope).predicate(predicate)?;
            plan = plan.filter(expr)?;
        }
    }
    plan = freeze_page_before_sort(plan, !body.order_by.is_empty(), ctx, trace).await?;
    Ok((
        aggregates::result(plan, session, &mut scope, body)?.build()?,
        current_graph,
    ))
}

/// A later sort must see only the rows an earlier LIMIT or OFFSET already kept.
/// DataFusion otherwise absorbs that limit into a TopK or pushes the sort under
/// the offset, so the new key chooses a different page.
async fn freeze_page_before_sort(
    plan: LogicalPlanBuilder,
    ordering: bool,
    ctx: &SessionContext,
    trace: &mut execution::Trace,
) -> Result<LogicalPlanBuilder> {
    if ordering && contains_limit(plan.plan()) {
        execution::freeze(plan, ctx, trace, None).await
    } else {
        Ok(plan)
    }
}

fn contains_limit(plan: &LogicalPlan) -> bool {
    matches!(plan, LogicalPlan::Limit(_)) || plan.inputs().iter().any(|input| contains_limit(input))
}

fn order_and_page(
    mut plan: LogicalPlanBuilder,
    session: &SessionState,
    ordering: &[ast::OrderByItem],
    offset: Option<&ast::UnsignedIntegerSpecification>,
    limit: Option<&ast::UnsignedIntegerSpecification>,
    mut scope: Option<&mut graph::Bindings>,
) -> Result<LogicalPlanBuilder> {
    if !ordering.is_empty() {
        let mut sorts = Vec::new();
        for item in ordering {
            let (next, mixed) = if let Some(scope) = scope.as_deref_mut() {
                native::fragment_value(plan, scope, session, &item.expr)?
            } else {
                (plan, None)
            };
            plan = next;
            let expr = if let Some(mixed) = mixed {
                mixed.expr
            } else if let Some(scope) = scope.as_deref() {
                Binder::with_bindings(session, plan.schema(), scope).bind(&item.expr)?
            } else {
                Binder::new(session, plan.schema()).bind(&item.expr)?
            };
            let ascending = item.direction != Some(ast::SortDirection::Desc);
            // Nulls sort as the greatest values unless explicitly specified.
            let nulls_first = match item.null_ordering {
                Some(ast::NullOrdering::First) => true,
                Some(ast::NullOrdering::Last) => false,
                None => !ascending,
            };
            sorts.push(expr.sort(ascending, nulls_first));
        }
        plan = plan.sort(sorts)?;
    }
    if offset.is_some() || limit.is_some() {
        let offset = offset
            .map(|n| page_count(n, session))
            .transpose()?
            .unwrap_or(0);
        let limit = limit.map(|n| page_count(n, session)).transpose()?;
        plan = plan.limit(offset, limit)?;
    }
    Ok(plan)
}

fn page_count(value: &ast::UnsignedIntegerSpecification, session: &SessionState) -> Result<usize> {
    let value = match value {
        ast::UnsignedIntegerSpecification::Literal(n) => *n,
        ast::UnsignedIntegerSpecification::Parameter(name) => {
            let value = session
                .parameter(name)
                .ok_or_else(|| Error::NotFound(format!("parameter {name}")))?;
            match value.value {
                Value::Integer(n) if n >= 0 => n as u64,
                _ => {
                    return Err(Error::InvalidQuery(
                        "page count must be a nonnegative integer".into(),
                    ))
                }
            }
        }
    };
    // DataFusion represents limits as signed integer expressions internally.
    if value > i64::MAX as u64 {
        return Err(Error::InvalidQuery(
            "page count exceeds supported integer range".into(),
        ));
    }
    usize::try_from(value)
        .map_err(|_| Error::InvalidQuery("page count exceeds platform range".into()))
}
