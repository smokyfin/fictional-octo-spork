use thiserror::Error;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("VPN engine is already running")]
    AlreadyRunning,

    #[error("VPN engine is not running")]
    NotRunning,

    #[error("invalid configuration: {0}")]
    InvalidConfig(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("http error: {0}")]
    Http(String),

    #[error("leaf error: {0}")]
    Leaf(String),

    #[error("arti error: {0}")]
    Arti(String),

    #[error("engine error: {0}")]
    Engine(String),

    #[error("xray runtime is unavailable on this platform")]
    XrayUnavailable,

    #[error("dns error: {0}")]
    Dns(String),

    #[error("ipc error: {0}")]
    Ipc(String),

    #[error("cancelled")]
    Cancelled,

    #[error("{0}")]
    Other(String),
}

impl From<anyhow::Error> for Error {
    fn from(value: anyhow::Error) -> Self {
        Error::Other(value.to_string())
    }
}

impl From<reqwest::Error> for Error {
    fn from(value: reqwest::Error) -> Self {
        Error::Http(value.to_string())
    }
}
