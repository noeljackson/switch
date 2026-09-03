use std::fmt;

/// The crate's error type.
///
/// Cancellation is its own variant because the CLI treats it differently from
/// a failure: declining a prompt should exit non-zero without printing an
/// error banner. That distinction used to be a string comparison against the
/// message "cancelled", inherited from the Go original.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The user declined a prompt.
    Cancelled,
    Message(String),
}

impl Error {
    pub fn new(msg: impl Into<String>) -> Error {
        Error::Message(msg.into())
    }

    pub fn is_cancelled(&self) -> bool {
        matches!(self, Error::Cancelled)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Cancelled => f.write_str("cancelled"),
            Error::Message(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Error {
        Error::Message(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;
