//! Parse GQL files without executing database operations.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let files = std::env::args().skip(1).collect::<Vec<_>>();
    if files.is_empty() {
        return Err("usage: parse_file FILE [FILE ...]".into());
    }
    for file in files {
        let input = std::fs::read_to_string(&file)?;
        gql_parser::parse(&input).map_err(|error| format!("{file}: {error}"))?;
    }
    Ok(())
}
