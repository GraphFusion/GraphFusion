use graphfusion::{arrow::util::pretty::pretty_format_batches, Database, Value};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let db = Database::new();
    let mut session = db.session();
    session.set_parameter("n", Value::Integer(6))?;
    let result = session
        .query("LET answer = $n * 7 FILTER answer > 40 RETURN answer")
        .await?;
    println!("{}", pretty_format_batches(&result.batches)?);
    println!("DataFusion physical plan:\n{}", result.physical_plan);
    Ok(())
}
