//! GQL binding and direct DataFusion logical/physical planning.
mod aggregates;
mod arithmetic;
mod execution;
mod expressions;
mod graph;
mod mutations;
mod relational;

use crate::{
    catalog::CommitSeq, gql as ast, transaction::StatementTxn, Database, Error, Result,
    SessionState, Value,
};
use datafusion::{
    arrow::{datatypes::SchemaRef, record_batch::RecordBatch},
    execution::context::SessionContext,
    logical_expr::{Expr, LogicalPlan, LogicalPlanBuilder},
};
use expressions::Binder;

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
}

impl QueryResult {
    pub fn row_count(&self) -> usize {
        self.batches.iter().map(RecordBatch::num_rows).sum()
    }
}

pub(crate) async fn execute(
    db: &Database,
    session: &SessionState,
    input: &str,
) -> Result<QueryResult> {
    if session.closed {
        return Err(Error::SessionClosed);
    }
    let program = ast::parse(input)?;
    if !program.definitions.is_empty() {
        return Err(unsupported(
            "query schema context and procedure definitions",
        ));
    }
    let [ast::Statement::Query(query)] = program.statements.as_slice() else {
        return Err(unsupported("query() requires exactly one read-only query"));
    };
    execute_statement(db, session, query, program.at_schema.as_ref(), false).await
}

pub(crate) async fn execute_statement(
    db: &Database,
    session: &SessionState,
    query: &ast::QueryStatement,
    at_schema: Option<&ast::SchemaReference>,
    allow_writes: bool,
) -> Result<QueryResult> {
    // The lease is held through planning AND materialization, including every await.
    let mut tx = StatementTxn::begin(db)?;
    let mut context = session.clone();
    if let Some(schema) = at_schema {
        context.current_schema = crate::session::schema_reference(&mut tx, session, schema)?;
    }
    let ctx = SessionContext::new();
    let mut writes = mutations::Writes::default();
    let mut trace = execution::Trace::default();
    let plan = plan(
        query,
        &context,
        &mut tx,
        &ctx,
        &mut writes,
        &mut trace,
        allow_writes,
    )
    .await?;
    let (schema, batches) = trace.collect(&ctx, &plan).await?;
    for (graph, data) in writes.graphs {
        tx.replace_graph_data(graph, data)?;
    }
    let commit_seq = tx.commit()?;
    Ok(QueryResult {
        schema,
        batches,
        commit_seq,
        logical_plan: trace.logical.join("\n"),
        physical_plan: trace.physical.join("\n"),
        affected_elements: writes.affected,
    })
}

fn unsupported(feature: &str) -> Error {
    Error::UnsupportedFeature(feature.into())
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
) -> Result<LogicalPlan> {
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
    if otherwise {
        // Bind every branch before execution. Empty barrier results retain their schema,
        // so a cold branch is checked without evaluating any of its expressions.
        let mut validation = execution::Trace {
            validate_only: true,
            ..Default::default()
        };
        let mut checked = plan_body(
            &query.body,
            &first_context,
            tx,
            ctx,
            writes,
            &mut validation,
        )
        .await?;
        for op in &query.set_operations {
            let right = plan_body(&op.body, &context, tx, ctx, writes, &mut validation).await?;
            checked =
                relational::set_operation(checked, right, ast::QuerySetOperator::Union, true)?;
        }
        let schema = checked.schema().clone();
        if trace.validate_only {
            return Ok(checked);
        }
        for (body, context) in std::iter::once((&query.body, &first_context))
            .chain(query.set_operations.iter().map(|op| (&op.body, &context)))
        {
            let branch = plan_body(body, context, tx, ctx, writes, trace).await?;
            let branch = relational::coerce(branch, &schema)?;
            let (schema, batches) = trace.collect(ctx, &branch).await?;
            if batches.iter().any(|b| b.num_rows() > 0) {
                return Ok(execution::memory(schema, batches)?.build()?);
            }
        }
        return Ok(
            execution::memory(std::sync::Arc::new(schema.as_arrow().clone()), vec![])?.build()?,
        );
    }
    let mut result = plan_body(&query.body, &first_context, tx, ctx, writes, trace).await?;
    for op in &query.set_operations {
        let right = plan_body(&op.body, &context, tx, ctx, writes, trace).await?;
        result = relational::set_operation(
            result,
            right,
            op.operator,
            op.quantifier == Some(ast::SetQuantifier::All),
        )?;
    }
    Ok(result)
}

fn has_mutations(query: &ast::QueryStatement) -> bool {
    std::iter::once(&query.body)
        .chain(query.set_operations.iter().map(|op| &op.body))
        .any(|body| {
            body.clauses.iter().any(|clause| {
                mutations::is_mutation(clause)
                    || matches!(clause, ast::QueryClause::NestedQuery(q) if has_mutations(q))
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
) -> Result<LogicalPlan> {
    let mut current_graph = session.current_graph;
    let mut scope = graph::Bindings::default();
    let mut plan = LogicalPlanBuilder::empty(true);
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
                    graph::matches(plan, &mut scope, session, id, &data, clause)?
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
            ast::QueryClause::NestedQuery(query) => {
                if !scope.order.is_empty() {
                    return Err(unsupported("correlated nested queries"));
                }
                let mut nested_context = session.clone();
                nested_context.current_graph = current_graph;
                let nested = Box::pin(self::plan(
                    query,
                    &nested_context,
                    tx,
                    ctx,
                    writes,
                    trace,
                    false,
                ))
                .await?;
                plan = relational::derived(plan, &mut scope, nested)?;
            }
            ast::QueryClause::For(clause) => {
                plan = relational::for_clause(plan, &mut scope, session, clause)?;
            }
            ast::QueryClause::Let(clause) => {
                for item in &clause.items {
                    if item.typed || item.value_type.is_some() {
                        return Err(unsupported("typed LET definitions"));
                    }
                    let binder = Binder::with_bindings(session, plan.schema(), &scope);
                    if scope.contains(&item.name.value) {
                        return Err(Error::InvalidQuery(format!(
                            "variable {} is already bound",
                            item.name.value
                        )));
                    }
                    let mut expressions: Vec<_> = plan
                        .schema()
                        .columns()
                        .into_iter()
                        .map(Expr::Column)
                        .collect();
                    let expression = binder.bind(&item.value)?;
                    let column = scope.scalar(&item.name.value);
                    expressions.push(expression.alias(column.name));
                    plan = plan.project(expressions)?;
                }
            }
            ast::QueryClause::Filter(clause) => {
                let expr = Binder::with_bindings(session, plan.schema(), &scope)
                    .predicate(&clause.predicate)?;
                plan = plan.filter(expr)?;
            }
            ast::QueryClause::OrderByPage(clause) => {
                plan = order_and_page(
                    plan,
                    session,
                    &clause.order_by,
                    clause.offset.as_ref(),
                    clause.limit.as_ref(),
                    Some(&scope),
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
            graph::matches(plan, &mut scope, session, id, &data, &from.match_clause)?
        };
    }
    if let Some(from) = &body.select_query {
        let mut nested_context = session.clone();
        nested_context.current_graph = if let Some(graph) = &from.graph {
            Some(crate::session::resolve_graph(tx, session, graph)?)
        } else {
            current_graph
        };
        let nested = Box::pin(self::plan(
            &from.query,
            &nested_context,
            tx,
            ctx,
            writes,
            trace,
            false,
        ))
        .await?;
        plan = relational::derived(plan, &mut scope, nested)?;
    }
    if body.result_clause.kind == ast::ResultKind::Finish {
        trace.collect(ctx, &plan.build()?).await?;
        return Ok(LogicalPlanBuilder::empty(false).build()?);
    }
    if let Some(predicate) = &body.select_where {
        let expr = Binder::with_bindings(session, plan.schema(), &scope).predicate(predicate)?;
        plan = plan.filter(expr)?;
    }
    Ok(aggregates::result(plan, session, &scope, body)?.build()?)
}

fn order_and_page(
    mut plan: LogicalPlanBuilder,
    session: &SessionState,
    ordering: &[ast::OrderByItem],
    offset: Option<&ast::UnsignedIntegerSpecification>,
    limit: Option<&ast::UnsignedIntegerSpecification>,
    scope: Option<&graph::Bindings>,
) -> Result<LogicalPlanBuilder> {
    if !ordering.is_empty() {
        let binder = match scope {
            Some(scope) => Binder::with_bindings(session, plan.schema(), scope),
            None => Binder::new(session, plan.schema()),
        };
        let sorts = ordering
            .iter()
            .map(|item| {
                let ascending = item.direction != Some(ast::SortDirection::Desc);
                // Nulls sort as the greatest values unless explicitly specified.
                let nulls_first = match item.null_ordering {
                    Some(ast::NullOrdering::First) => true,
                    Some(ast::NullOrdering::Last) => false,
                    None => !ascending,
                };
                Ok(binder.bind(&item.expr)?.sort(ascending, nulls_first))
            })
            .collect::<Result<Vec<_>>>()?;
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
                .parameters
                .get(name)
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
