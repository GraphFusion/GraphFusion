//! DataFusion materialization barriers shared by queries and writes.
use crate::{Error, Result};
use datafusion::{
    arrow::{
        array::UInt64Array,
        datatypes::{DataType, Field, Schema, SchemaRef},
        record_batch::{RecordBatch, RecordBatchOptions},
    },
    common::{Column, TableReference},
    datasource::{provider_as_source, MemTable},
    execution::context::SessionContext,
    logical_expr::{Expr, LogicalPlan, LogicalPlanBuilder},
    physical_plan::{collect, displayable},
};
use std::sync::Arc;
#[derive(Default)]
pub(super) struct Trace {
    pub logical: Vec<String>,
    pub physical: Vec<String>,
    pub validate_only: bool,
    pub sources: std::collections::BTreeMap<crate::catalog::ObjectId, Arc<crate::graph::GraphData>>,
    pub output_domains: std::collections::BTreeMap<String, super::references::Domain>,
}
impl Trace {
    pub async fn collect(
        &mut self,
        ctx: &SessionContext,
        plan: &LogicalPlan,
    ) -> Result<(SchemaRef, Vec<RecordBatch>)> {
        if self.validate_only {
            return Ok((Arc::new(plan.schema().as_arrow().clone()), vec![]));
        }
        let physical = ctx.state().create_physical_plan(plan).await?;
        self.logical.push(plan.display_indent().to_string());
        self.physical
            .push(displayable(physical.as_ref()).indent(true).to_string());
        let schema = physical.schema();
        Ok((schema, collect(physical, ctx.task_ctx()).await?))
    }
}

pub(super) fn col(relation: &str, name: &str) -> Expr {
    Expr::Column(Column::new(Some(TableReference::bare(relation)), name))
}
pub(super) fn memory(schema: SchemaRef, batches: Vec<RecordBatch>) -> Result<LogicalPlanBuilder> {
    Ok(LogicalPlanBuilder::scan(
        "__gf_buffer",
        provider_as_source(Arc::new(MemTable::try_new(schema, vec![batches])?)),
        None,
    )?)
}
pub(super) fn batch(
    schema: SchemaRef,
    columns: Vec<datafusion::arrow::array::ArrayRef>,
    rows: usize,
) -> Result<RecordBatch> {
    RecordBatch::try_new_with_options(
        schema,
        columns,
        &RecordBatchOptions::new().with_row_count(Some(rows)),
    )
    .map_err(|e| datafusion::error::DataFusionError::from(e).into())
}

/// Materialize once at an execution barrier, retaining GQL variable qualifiers.
pub(super) async fn freeze(
    plan: LogicalPlanBuilder,
    ctx: &SessionContext,
    trace: &mut Trace,
    ids: Option<(&str, Vec<u64>)>,
) -> Result<LogicalPlanBuilder> {
    freeze_impl(plan, ctx, trace, ids, false).await
}

pub(super) async fn ordinal(
    plan: LogicalPlanBuilder,
    ctx: &SessionContext,
    trace: &mut Trace,
    name: &str,
) -> Result<LogicalPlanBuilder> {
    freeze_impl(plan, ctx, trace, Some((name, vec![])), true).await
}

async fn freeze_impl(
    plan: LogicalPlanBuilder,
    ctx: &SessionContext,
    trace: &mut Trace,
    ids: Option<(&str, Vec<u64>)>,
    ordinal: bool,
) -> Result<LogicalPlanBuilder> {
    let columns = plan.schema().columns();
    let renamed = plan
        .project(
            columns
                .iter()
                .enumerate()
                .map(|(i, c)| Expr::Column(c.clone()).alias(format!("__gf_frozen_{i}"))),
        )?
        .build()?;
    let (schema, batches) = trace.collect(ctx, &renamed).await?;
    let mut fields = schema.fields().to_vec();
    if ids.is_some() {
        // Keep generated IDs behind the same physical-column renaming barrier as
        // input values; projection pushdown must not expose qualified duplicates.
        fields.push(Arc::new(Field::new(
            format!("__gf_frozen_{}", columns.len()),
            DataType::UInt64,
            false,
        )));
    }
    let schema = Arc::new(Schema::new(fields));
    let mut offset = 0;
    let mut extended = Vec::new();
    for input in batches {
        let mut values = input.columns().to_vec();
        if let Some((_, ids)) = &ids {
            let values_ids = if ordinal {
                (offset as u64..(offset + input.num_rows()) as u64).collect()
            } else {
                ids.get(offset..offset + input.num_rows())
                    .ok_or_else(|| Error::Corrupt("write input cardinality changed".into()))?
                    .to_vec()
            };
            values.push(Arc::new(UInt64Array::from(values_ids)));
        }
        offset += input.num_rows();
        extended.push(batch(schema.clone(), values, input.num_rows())?);
    }
    if let Some((_, ids)) = &ids {
        if !ordinal && offset != ids.len() {
            return Err(Error::Corrupt("write input cardinality changed".into()));
        }
    }
    let mut output: Vec<_> = columns
        .iter()
        .enumerate()
        .map(|(i, c)| {
            col("__gf_buffer", &format!("__gf_frozen_{i}"))
                .alias_qualified(c.relation.clone(), &c.name)
        })
        .collect();
    if let Some((name, _)) = ids {
        output.push(col("__gf_buffer", &format!("__gf_frozen_{}", columns.len())).alias(name));
    }
    Ok(memory(schema, extended)?.project(output)?)
}
