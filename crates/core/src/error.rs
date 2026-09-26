use thiserror::Error;

#[derive(Debug, Error)]
pub enum EnscriveError {
    #[error("HTTP {status}: {body}")]
    Http {
        status: reqwest::StatusCode,
        body: String,
    },

    /// Server responded with an HTTP 3xx redirect. `EnscriveClient`'s http
    /// client is built with `redirect::Policy::none()` so a redirect is
    /// never silently followed — doing so would resend `X-API-Key` (and, if
    /// set, `X-Embedding-Provider-Key`) to whatever host `Location` names.
    ///
    /// ENS-6483 (Sol round 3, L): the message contains the fleet spec's
    /// exact required phrase, `refusing to follow HTTP <status> redirect
    /// to <host>` — fixed, non-dynamic explanatory text around it is
    /// permitted, but this substring itself must appear verbatim.
    #[error("refusing to follow HTTP {status} redirect to {location_host}: Enscrive credentials (X-API-Key / X-Embedding-Provider-Key) are never resent to a redirect target")]
    Redirected {
        status: reqwest::StatusCode,
        /// Host only, never the full URL or query string (which could
        /// itself carry sensitive data), and never any credential.
        location_host: String,
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
