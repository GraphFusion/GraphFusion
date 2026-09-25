use super::{
    execution::{self, Trace},
    expressions::Binder,
    graph::{self, Bindings},
};
use crate::{catalog::ObjectId, gql as ast, graph::GraphData, Error, Result, SessionState};
use datafusion::{
    common::Column,
    execution::context::SessionContext,
    logical_expr::{Expr, JoinType, LogicalPlanBuilder},
};

#[allow(clippy::too_many_arguments)]
pub(super) async fn optional(
    plan: LogicalPlanBuilder,
    scope: &mut Bindings,
    session: &SessionState,
    id: ObjectId,
    data: &GraphData,
    clauses: &[ast::MatchClause],
    ctx: &SessionContext,
    trace: &mut Trace,
) -> Result<LogicalPlanBuilder> {
    let row = scope.fresh("optional_row");
    let input = execution::ordinal(plan, ctx, trace, &row).await?;
    let input_columns = input.schema().columns();
    let mut matches = input.clone();
    for clause in clauses {
        matches = if clause.optional {
            let mut inner = clause.clone();
            inner.optional = false;
            Box::pin(optional(
                matches,
                scope,
                session,
                id,
                data,
                &[inner],
                ctx,
                trace,
            ))
            .await?
        } else {
            graph::matches(matches, scope, session, id, data, clause, ctx, trace).await?
        };
    }
    let right_row = scope.fresh("optional_key");
    let extra: Vec<_> = matches
        .schema()
        .columns()
        .into_iter()
        .filter(|c| !input_columns.contains(c))
        .collect();
    let mut right_columns: Vec<_> = extra.iter().cloned().map(Expr::Column).collect();
    right_columns.push(Expr::Column(Column::new_unqualified(&row)).alias(&right_row));
    // A materialization boundary also keeps outer-join projection pruning from
    // rewriting recursive work-table schemas (DataFusion 55 cannot safely do so).
    let right = execution::freeze(matches.project(right_columns)?, ctx, trace, None)
        .await?
        .build()?;
    let output = input_columns
        .into_iter()
        .filter(|c| c.name != row)
        .chain(extra)
        .map(Expr::Column);
    Ok(input
        .join_on(
            right,
            JoinType::Left,
            [Expr::Column(Column::new_unqualified(&row))
                .eq(Expr::Column(Column::new_unqualified(right_row)))],
        )?
        .project(output)?)
}

pub(super) async fn for_clause(
    plan: LogicalPlanBuilder,
    scope: &mut Bindings,
    session: &SessionState,
    clause: &ast::ForClause,
    ctx: &SessionContext,
    trace: &mut Trace,
) -> Result<LogicalPlanBuilder> {
    use datafusion::{
        arrow::datatypes::DataType,
        common::UnnestOptions,
        logical_expr::{cast, lit, ExprSchemable},
    };
    if scope.contains(&clause.variable.value)
        || clause
            .ordinality_or_offset
            .as_ref()
            .is_some_and(|o| scope.contains(&o.variable.value) || o.variable == clause.variable)
    {
        return Err(Error::InvalidQuery("FOR variable is already bound".into()));
    }
    let mut source = Binder::with_bindings(session, plan.schema(), scope).bind(&clause.source)?;
    if source.get_type(plan.schema())? == DataType::Null {
        source = cast(source, DataType::new_list(DataType::Null, true));
    }
    if !matches!(
        source.get_type(plan.schema())?,
        DataType::List(_) | DataType::LargeList(_) | DataType::FixedSizeList(_, _)
    ) {
        return Err(Error::InvalidQuery("FOR requires a list value".into()));
    }
    let variable = scope.scalar(&clause.variable.value);
    if let Some(domain) = super::references::domain(&clause.source, scope) {
        scope.domains.insert(clause.variable.value.clone(), domain);
    }
    let mut output: Vec<_> = plan
        .schema()
        .columns()
        .into_iter()
        .map(Expr::Column)
        .collect();
    let mut unnest = vec![variable.clone()];
    output.push(source.clone().alias(&variable.name));
    if let Some(index) = &clause.ordinality_or_offset {
        let column = scope.scalar(&index.variable.value);
        let length = cast(
            datafusion::functions_nested::expr_fn::array_length(source),
            DataType::Int64,
        );
        let start = if index.kind == ast::ForOrdinalityOrOffsetKind::Ordinality {
            1_i64
        } else {
            0
        };
        let end = length + lit(start);
        let indices = datafusion::functions_nested::expr_fn::range(lit(start), end, lit(1_i64));
        output.push(indices.alias(&column.name));
        unnest.push(column);
    }
    let plan = plan
        .project(output)?
        .unnest_columns_with_options(unnest, UnnestOptions::new().with_preserve_nulls(false))?;
    super::references::prepare(plan, scope, ctx, trace).await
}

pub(super) fn derived(
    input: LogicalPlanBuilder,
    scope: &mut Bindings,
    nested: datafusion::logical_expr::LogicalPlan,
) -> Result<LogicalPlanBuilder> {
    let mut output = Vec::new();
    for (column, field) in nested.schema().iter() {
        if scope.contains(field.name()) {
            return Err(Error::InvalidQuery(format!(
                "variable {} is already bound",
                field.name()
            )));
        }
        let source = Expr::Column(Column::new(column.cloned(), field.name()));
        output.push(source.alias(scope.scalar(field.name()).name));
    }
    Ok(input.cross_join(LogicalPlanBuilder::from(nested).project(output)?.build()?)?)
}

/// Validate GQL result names before DataFusion's positional type coercion.
pub(super) fn coerce(
    plan: datafusion::logical_expr::LogicalPlan,
    target: &datafusion::common::DFSchema,
) -> Result<datafusion::logical_expr::LogicalPlan> {
    use datafusion::{arrow::datatypes::DataType, logical_expr::cast};
    if plan.schema().fields().len() != target.fields().len() {
        return Err(Error::InvalidQuery(
            "composite result column counts differ".into(),
        ));
    }
    let mut output = Vec::new();
    for ((qualifier, field), expected) in plan.schema().iter().zip(target.fields()) {
        if field.name() != expected.name() {
            return Err(Error::InvalidQuery(
                "composite result column names or order differ".into(),
            ));
        }
        let from = field.data_type();
        let to = expected.data_type();
        if from != to
            && *from != DataType::Null
            && !(*from == DataType::Int64 && *to == DataType::Float64)
        {
            return Err(Error::InvalidQuery(format!(
                "incompatible composite column types {from} and {to}"
            )));
        }
        let mut expr = Expr::Column(Column::new(qualifier.cloned(), field.name()));
        if from != to {
            expr = cast(expr, to.clone());
        }
        output.push(expr.alias(field.name()));
    }
    Ok(LogicalPlanBuilder::from(plan).project(output)?.build()?)
}

pub(super) fn set_operation(
    left: datafusion::logical_expr::LogicalPlan,
    right: datafusion::logical_expr::LogicalPlan,
    operator: ast::QuerySetOperator,
    all: bool,
) -> Result<datafusion::logical_expr::LogicalPlan> {
    use datafusion::{
        arrow::datatypes::{DataType, Field, Schema},
        common::{DFSchema, NullEquality},
        logical_expr::ExprFunctionExt,
    };
    if left.schema().fields().len() != right.schema().fields().len() {
        return Err(Error::InvalidQuery(
            "composite result column counts differ".into(),
        ));
    }
    let fields = left
        .schema()
        .fields()
        .iter()
        .zip(right.schema().fields())
        .map(|(a, b)| {
            let data_type = match (a.data_type(), b.data_type()) {
                (DataType::Null, b) => b.clone(),
                (DataType::Int64, DataType::Float64) => DataType::Float64,
                (a, _) => a.clone(),
            };
            Field::new(a.name(), data_type, a.is_nullable() || b.is_nullable())
        })
        .collect::<Vec<_>>();
    let target = DFSchema::try_from(Schema::new(fields))?;
    let left = coerce(left, &target)?;
    let right = coerce(right, &target)?;
    if operator == ast::QuerySetOperator::Union {
        let union = LogicalPlanBuilder::from(left).union(right)?;
        return Ok(if all { union } else { union.distinct()? }.build()?);
    }
    let kind = match operator {
        ast::QuerySetOperator::Intersect => JoinType::LeftSemi,
        ast::QuerySetOperator::Except => JoinType::LeftAnti,
        _ => return Err(Error::InvalidQuery("unexpected set operator".into())),
    };
    // Match the nth occurrence of each complete row. A semijoin alone would retain
    // every left duplicate whenever even one right row exists, violating bag counts.
    let side = |plan, alias: &str| -> Result<LogicalPlanBuilder> {
        let mut plan = LogicalPlanBuilder::from(plan);
        let columns = plan.schema().columns();
        let values = columns
            .iter()
            .enumerate()
            .map(|(i, c)| Expr::Column(c.clone()).alias(format!("__gf_set_{i}")));
        plan = plan.project(values)?;
        if all {
            let keys = plan
                .schema()
                .columns()
                .into_iter()
                .map(Expr::Column)
                .collect::<Vec<_>>();
            let occurrence = datafusion::functions_window::row_number::row_number()
                .partition_by(keys)
                .build()?
                .alias("__gf_occurrence");
            plan = plan.window([occurrence])?;
        } else {
            plan = plan.distinct()?;
        }
        Ok(plan.alias(alias)?)
    };
    let left = side(left, "__gf_left")?;
    let right = side(right, "__gf_right")?;
    let keys = (left.schema().columns(), right.schema().columns());
    let result = left.join_detailed(
        right.build()?,
        kind,
        keys,
        None,
        NullEquality::NullEqualsNull,
    )?;
    Ok(result
        .project(
            target.fields().iter().enumerate().map(|(i, f)| {
                execution::col("__gf_left", &format!("__gf_set_{i}")).alias(f.name())
            }),
        )?
        .build()?)
}
