use thiserror::Error;

#[derive(Debug, Error)]
pub enum EnscriveError {
    #[error("HTTP {status}: {body}")]
    Http {
        status: reqwest::StatusCode,
        body: String,
    },

    #[error("request failed: {0}")]
    Request(#[from] reqwest::Error),

    #[error("response parse failed: {0}")]
    Parse(#[from] serde_json::Error),

    #[error("config: {0}")]
    Config(String),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("toml parse: {0}")]
    Toml(#[from] toml::de::Error),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, EnscriveError>;
