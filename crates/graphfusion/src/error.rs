use std::{fmt, io};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    Parse(gql_parser::Error),
    DataFusion(Box<datafusion::error::DataFusionError>),
    InvalidQuery(String),
    Io(io::Error),
    Corrupt(String),
    UnsupportedFeature(String),
    AlreadyExists(String),
    NotFound(String),
    InvalidDefinition(String),
    DependencyExists(String),
    Conflict(String),
    InvalidReference(String),
    SessionClosed,
    TransactionActive,
    NoTransaction,
    TransactionFailed,
    ReadOnlyTransaction,
    Busy,
    CommitUnknown(io::Error),
    Poisoned,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(e) => write!(f, "{e}"),
            Self::DataFusion(e) => write!(f, "query execution error: {e}"),
            Self::InvalidQuery(s) => write!(f, "invalid query: {s}"),
            Self::Io(e) => write!(f, "I/O error: {e}"),
            Self::Corrupt(s) => write!(f, "corrupt database: {s}"),
            Self::UnsupportedFeature(s) => write!(f, "unsupported feature: {s}"),
            Self::AlreadyExists(s) => write!(f, "already exists: {s}"),
            Self::NotFound(s) => write!(f, "not found: {s}"),
            Self::InvalidDefinition(s) => write!(f, "invalid definition: {s}"),
            Self::DependencyExists(s) => write!(f, "dependent objects exist: {s}"),
            Self::Conflict(s) => write!(f, "transaction conflict: {s}"),
            Self::InvalidReference(s) => write!(f, "invalid reference: {s}"),
            Self::SessionClosed => write!(f, "session is closed"),
            Self::TransactionActive => write!(f, "a transaction is already active"),
            Self::NoTransaction => write!(f, "no explicit transaction is active"),
            Self::TransactionFailed => write!(f, "transaction is failed; ROLLBACK is required"),
            Self::ReadOnlyTransaction => {
                write!(f, "writes are forbidden in a READ ONLY transaction")
            }
            Self::Busy => write!(f, "active statements or transactions prevent checkpoint"),
            Self::CommitUnknown(e) => {
                write!(f, "commit outcome is unknown; reopen before retrying: {e}")
            }
            Self::Poisoned => write!(f, "database coordinator is poisoned"),
        }
    }
}

impl std::error::Error for Error {}
impl From<datafusion::error::DataFusionError> for Error {
    fn from(e: datafusion::error::DataFusionError) -> Self {
        Self::DataFusion(Box::new(e))
    }
}
impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}
impl From<gql_parser::Error> for Error {
    fn from(e: gql_parser::Error) -> Self {
        Self::Parse(e)
    }
}
impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Self::Corrupt(e.to_string())
    }
}
