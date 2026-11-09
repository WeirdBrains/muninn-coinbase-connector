/// Failure from encoding, the session, or a rejected order shape.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Codec(String),
    #[error("{0}")]
    Auth(String),
    #[error("{0}")]
    Protocol(String),
    #[error("{0}")]
    Rejected(String),
    #[error("rate limit: {0}")]
    RateLimit(String),
    #[error("disconnected")]
    Disconnected,
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("tls: {0}")]
    Tls(String),
}

impl Error {
    pub(crate) fn codec(msg: impl Into<String>) -> Self {
        Error::Codec(msg.into())
    }
    pub(crate) fn auth(msg: impl Into<String>) -> Self {
        Error::Auth(msg.into())
    }
    pub(crate) fn protocol(msg: impl Into<String>) -> Self {
        Error::Protocol(msg.into())
    }
    pub(crate) fn rejected(msg: impl Into<String>) -> Self {
        Error::Rejected(msg.into())
    }

    /// A second value with the same message. `std::io::Error` is rebuilt from its text.
    pub(crate) fn same_kind(&self) -> Self {
        match self {
            Error::Codec(s) => Error::Codec(s.clone()),
            Error::Auth(s) => Error::Auth(s.clone()),
            Error::Protocol(s) => Error::Protocol(s.clone()),
            Error::Rejected(s) => Error::Rejected(s.clone()),
            Error::RateLimit(s) => Error::RateLimit(s.clone()),
            Error::Disconnected => Error::Disconnected,
            Error::Io(e) => Error::Io(std::io::Error::new(e.kind(), e.to_string())),
            Error::Tls(s) => Error::Tls(s.clone()),
        }
    }
}

impl PartialEq for Error {
    fn eq(&self, other: &Self) -> bool {
        self.to_string() == other.to_string()
    }
}

pub type Result<T> = std::result::Result<T, Error>;
