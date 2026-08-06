use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Callback error: {0}")]
    Callback(String),

    #[error("State mismatch (possible CSRF): received unexpected state value")]
    StateMismatch,

    #[error("Token exchange failed: {0}")]
    TokenExchange(String),
}
