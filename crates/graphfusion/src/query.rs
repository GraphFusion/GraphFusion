//! GQL binding and direct DataFusion logical/physical planning.
mod arithmetic;
mod expressions;
mod graph;

use crate::{
    catalog::CommitSeq, gql as ast, transaction::StatementTxn, Database, Error, Result,
    SessionState, Value,
};
use datafusion::{
    arrow::{datatypes::SchemaRef, record_batch::RecordBatch},
    execution::context::SessionContext,
    logical_expr::{Expr, LogicalPlan, LogicalPlanBuilder},
    physical_plan::{collect, displayable},
};
use expressions::Binder;
use std::collections::BTreeSet;

#[derive(Debug)]
pub struct QueryResult {
    /// Available even when execution returns no batches/rows.
    pub schema: SchemaRef,
    pub batches: Vec<RecordBatch>,
    pub commit_seq: CommitSeq,
    pub logical_plan: String,
    pub physical_plan: String,
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
    execute_statement(db, session, query, program.at_schema.as_ref()).await
}

pub(crate) async fn execute_statement(
    db: &Database,
    session: &SessionState,
    query: &ast::QueryStatement,
    at_schema: Option<&ast::SchemaReference>,
) -> Result<QueryResult> {
    // The lease is held through planning AND materialization, including every await.
    let mut tx = StatementTxn::begin(db)?;
    let mut context = session.clone();
    if let Some(schema) = at_schema {
        context.current_schema = crate::session::schema_reference(&mut tx, session, schema)?;
    }
    let plan = plan(query, &context, &mut tx)?;
    let logical_plan = plan.display_indent().to_string();
    let ctx = SessionContext::new();
    let physical = ctx.state().create_physical_plan(&plan).await?;
    let schema = physical.schema();
    let physical_plan = displayable(physical.as_ref()).indent(true).to_string();
    let batches = collect(physical, ctx.task_ctx()).await?;
    let commit_seq = tx.commit()?;
    Ok(QueryResult {
        schema,
        batches,
        commit_seq,
        logical_plan,
        physical_plan,
    })
}

fn unsupported(feature: &str) -> Error {
    Error::UnsupportedFeature(feature.into())
}

fn plan(
    query: &ast::QueryStatement,
    initial: &SessionState,
    tx: &mut StatementTxn,
) -> Result<LogicalPlan> {
    let mut context = initial.clone();
    if let Some(schema) = &query.at_schema {
        context.current_schema = crate::session::schema_reference(tx, initial, schema)?;
    }
    let session = &context;
    let mut current_graph = if let Some(graph) = &query.use_graph {
        Some(crate::session::resolve_graph(tx, session, graph)?)
    } else {
        session.current_graph
    };
    let mut scope = graph::Bindings::default();
    if !query.set_operations.is_empty() {
        return Err(unsupported("composite queries"));
    }
    let body = &query.body;
    if !body.select_from.is_empty() || body.select_query.is_some() {
        return Err(unsupported("SELECT FROM graph queries"));
    }
    if !body.group_by.is_empty() || body.empty_grouping_set || body.having.is_some() {
        return Err(unsupported("grouping and HAVING"));
    }
    if body.result_clause.kind == ast::ResultKind::Finish {
        return Err(unsupported("FINISH query results"));
    }
    let mut plan = LogicalPlanBuilder::empty(true);
    for clause in &body.clauses {
        match clause {
            ast::QueryClause::UseGraph(expression) => {
                current_graph = Some(crate::session::resolve_graph(tx, session, expression)?);
            }
            ast::QueryClause::Match(clause) => {
                let id = current_graph
                    .ok_or_else(|| Error::InvalidReference("current graph is unset".into()))?;
                let data = tx.graph_data(id)?;
                plan = graph::matches(plan, &mut scope, session, id, &data, clause)?;
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
            _ => {
                return Err(unsupported(
                    "graph, procedure, or data-modifying query clause",
                ))
            }
        }
    }
    if let Some(predicate) = &body.select_where {
        let expr = Binder::with_bindings(session, plan.schema(), &scope).predicate(predicate)?;
        plan = plan.filter(expr)?;
    }
    let binder = Binder::with_bindings(session, plan.schema(), &scope);
    let mut projections = Vec::new();
    let mut output_names = BTreeSet::new();
    for (index, item) in body.result_clause.items.iter().enumerate() {
        if matches!(item.expr, ast::Expr::Wildcard) {
            if item.alias.is_some() {
                return Err(Error::InvalidQuery("wildcard cannot have an alias".into()));
            }
            for name in &scope.order {
                if !output_names.insert(name.clone()) {
                    return Err(Error::InvalidQuery("duplicate result column".into()));
                }
                projections.push(
                    binder
                        .bind(&ast::Expr::Identifier(ast::Identifier::new(name)))?
                        .alias(name),
                );
            }
            continue;
        }
        let name = item
            .alias
            .as_ref()
            .map(|name| name.value.clone())
            .unwrap_or_else(|| match &item.expr {
                ast::Expr::Identifier(name) => name.value.clone(),
                _ => format!("column_{}", index + 1),
            });
        if !output_names.insert(name.clone()) {
            return Err(Error::InvalidQuery(format!(
                "duplicate result column {name}"
            )));
        }
        projections.push(binder.bind(&item.expr)?.alias(name));
    }
    if projections.is_empty() {
        return Err(Error::InvalidQuery("result has no bound columns".into()));
    }
    plan = plan.project(projections)?;
    if body.result_clause.distinct {
        plan = plan.distinct()?;
    }
    Ok(order_and_page(
        plan,
        session,
        &body.order_by,
        body.offset.as_ref(),
        body.limit.as_ref(),
        None,
    )?
    .build()?)
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
