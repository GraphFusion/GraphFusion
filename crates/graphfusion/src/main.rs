mod cli;

#[tokio::main]
async fn main() -> std::process::ExitCode {
    match cli::run().await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("graphfusion: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
