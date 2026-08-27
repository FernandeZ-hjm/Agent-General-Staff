//! Kernel error type: stable machine-readable code + human message.
//! Adapters surface `code` verbatim; callers must never match on the message.

use std::fmt;

#[derive(Debug, Clone)]
pub struct Error {
    pub code: &'static str,
    pub message: String,
}

impl Error {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Error {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

/// Wrap an io error with a stable code.
pub fn io(code: &'static str, err: &std::io::Error) -> Error {
    Error::new(code, err.to_string())
}
