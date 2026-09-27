//! Load errors.
//!
//! Java `DsnReader.readBoard` has three failure outcomes plus uncaught runtime
//! exceptions (NPEs, class cast exceptions, ...) that escape `readBoard` and are
//! turned into an `IoError` by `HeadlessBoardManager.loadFromSpecctraDsn`.
//! [`LoadError`] mirrors them so callers can reproduce the Java result exactly.

use std::fmt;

/// Why a DSN could not be turned into a board.
#[derive(Debug, Clone, PartialEq)]
pub enum LoadError {
    /// Java `BoardReadResult.ParseError`: a scope reader returned `false`
    /// (e.g. no layers, keepout on layer `pcb`, unknown via padstack in the wiring).
    ParseError(String),
    /// Java `BoardReadResult.OutlineMissing`: no usable boundary (the Java board is `null`).
    OutlineMissing(String),
    /// A Java runtime exception would escape `DsnReader.readBoard`
    /// (`HeadlessBoardManager` reports it as an I/O error).
    JavaException(String),
    /// The file uses a scope order the loader does not emulate (see crate docs).
    Unsupported(String),
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadError::ParseError(m) => write!(f, "DSN parse error: {m}"),
            LoadError::OutlineMissing(m) => write!(f, "board outline missing: {m}"),
            LoadError::JavaException(m) => write!(f, "Java would throw: {m}"),
            LoadError::Unsupported(m) => write!(f, "unsupported DSN layout: {m}"),
        }
    }
}

impl std::error::Error for LoadError {}

/// Internal result type: `Err` for a Java runtime exception or a reader returning false.
pub(crate) type LResult<T> = Result<T, LoadError>;

pub(crate) fn npe(what: &str) -> LoadError {
    LoadError::JavaException(format!("NullPointerException: {what}"))
}
