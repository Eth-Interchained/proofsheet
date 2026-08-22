use std::fmt;

/// Everything that can go wrong inside the core.
#[derive(Debug)]
pub enum Error {
    /// Underlying socket / process I/O failure.
    Io(std::io::Error),
    /// The WebSocket upgrade or framing went wrong.
    Protocol(String),
    /// Chrome accepted the command but answered with an error.
    Cdp { method: String, message: String },
    /// Chrome could not be found, launched, or attached to.
    Browser(String),
    /// A response did not have the shape we required.
    Shape(String),
    /// Serialization / deserialization failure.
    Json(serde_json::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "io: {e}"),
            Error::Protocol(m) => write!(f, "websocket protocol: {m}"),
            Error::Cdp { method, message } => write!(f, "cdp {method}: {message}"),
            Error::Browser(m) => write!(f, "browser: {m}"),
            Error::Shape(m) => write!(f, "unexpected response shape: {m}"),
            Error::Json(e) => write!(f, "json: {e}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            Error::Json(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Json(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;
