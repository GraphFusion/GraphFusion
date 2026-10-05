//! A session owned by a browser worker. No server, remote store or filesystem.
use graphfusion::{
    visualization::{cell, DISPLAY_ROWS},
    Database, QueryLimits, Session, StatementOutput,
};
use serde_json::{json, Value};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct Engine {
    session: Session,
    runtime: tokio::runtime::Runtime,
}
#[wasm_bindgen]
impl Engine {
    #[wasm_bindgen(constructor)]
    pub fn new() -> std::result::Result<Engine, JsValue> {
        #[cfg(target_family = "wasm")]
        console_error_panic_hook::set_once();
        let mut session = Database::new().session();
        session
            .set_query_limits(QueryLimits {
                max_path_hops: 16,
                memory_limit_bytes: 128 * 1024 * 1024,
            })
            .map_err(js_error)?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .map_err(js_error)?;
        Ok(Self { session, runtime })
    }
    /// Drives DataFusion's Tokio tasks on the worker's single thread. All exposed
    /// storage is resident, so execution needs neither browser I/O nor an idle timer.
    pub fn run(&mut self, input: &str) -> std::result::Result<String, JsValue> {
        let outputs = self
            .runtime
            .block_on(self.session.run(input))
            .map_err(js_error)?;
        let mut statements = Vec::new();
        for output in outputs {
            statements.push(match output {
                StatementOutput::Command(result) => json!({"kind":"command", "affected":result.affected_objects,
                    "commit":result.commit_seq.to_string(), "pending":result.transaction_pending,
                    "action":format!("{:?}",result.transaction_action)}),
                StatementOutput::Query(result) => {
                    let mut rows: Vec<Vec<Value>> = Vec::new();
                    for batch in &result.batches {
                        for row in 0..batch.num_rows().min(DISPLAY_ROWS.saturating_sub(rows.len())) {
                            rows.push(batch.columns().iter().map(|c| cell(c,row)).collect::<graphfusion::Result<_>>().map_err(js_error)?);
                        }
                    }
                    json!({"kind":"query", "columns":result.schema.fields().iter().map(|f| f.name()).collect::<Vec<_>>(),
                        "rows":rows, "rowCount":result.row_count(), "graph":result.graph,
                        "logicalPlan":result.logical_plan, "physicalPlan":result.physical_plan,
                        "affected":result.affected_elements, "commit":result.commit_seq.to_string(),
                        "pending":result.transaction_pending})
                }
            });
        }
        serde_json::to_string(&json!({"statements":statements, "transaction":format!("{:?}",self.session.transaction_status())})).map_err(js_error)
    }
}
fn js_error(error: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&error.to_string())
}
