pub mod gql {
    pub use gql_parser::*;
}

#[derive(Clone, Debug, Default)]
pub struct Database;

impl Database {
    pub fn new() -> Self {
        Self
    }

    pub fn parse_gql(&self, input: &str) -> gql::Result<gql::Program> {
        gql::parse(input)
    }
}
