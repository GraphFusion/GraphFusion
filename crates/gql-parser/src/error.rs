use std::fmt;

use crate::token::TokenKind;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    UnrecognizedToken {
        offset: usize,
        token_text: String,
    },
    UnterminatedString {
        offset: usize,
        token_text: String,
    },
    UnterminatedBlockComment {
        offset: usize,
        token_text: String,
    },
    BadNumber {
        offset: usize,
        token_text: String,
    },
    UnexpectedEof,
    UnexpectedToken {
        offset: usize,
        expected: Vec<TokenKind>,
        got: TokenKind,
        token_text: String,
    },
    Message {
        offset: usize,
        message: String,
    },
}

impl Error {
    pub(crate) fn expected(
        offset: usize,
        expected: impl Into<Vec<TokenKind>>,
        got: TokenKind,
        token_text: impl Into<String>,
    ) -> Self {
        Self::UnexpectedToken {
            offset,
            expected: expected.into(),
            got,
            token_text: token_text.into(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnrecognizedToken { offset, token_text } => {
                write!(f, "unrecognized token '{token_text}' at offset {offset}")
            }
            Self::UnterminatedString { offset, token_text } => {
                write!(f, "unterminated string '{token_text}' at offset {offset}")
            }
            Self::UnterminatedBlockComment { offset, token_text } => {
                write!(
                    f,
                    "unterminated block comment '{token_text}' at offset {offset}"
                )
            }
            Self::BadNumber { offset, token_text } => {
                write!(f, "bad number '{token_text}' at offset {offset}")
            }
            Self::UnexpectedEof => write!(f, "incomplete input"),
            Self::UnexpectedToken {
                offset,
                expected,
                got,
                token_text,
            } => write!(
                f,
                "near '{token_text}' at offset {offset}: expected {expected:?}, got {got:?}"
            ),
            Self::Message { offset, message } => write!(f, "{message} at offset {offset}"),
        }
    }
}

impl std::error::Error for Error {}
