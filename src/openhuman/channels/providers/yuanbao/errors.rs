//! Yuanbao channel error types.

use thiserror::Error;

/// Close codes from the yuanbao gateway that indicate the connection
/// must **not** be retried (auth failure, kicked off, etc.).
///
/// Mirrors `NO_RECONNECT_CLOSE_CODES` in hermes-agent `yuanbao.py`.
pub const NO_RECONNECT_CLOSE_CODES: &[u16] = &[4012, 4013, 4014, 4018, 4019, 4021];

/// Auth-related response codes that mean "credentials are bad" — surface
/// to the user, don't auto-retry.
pub const AUTH_FAILED_CODES: &[u32] = &[40001, 40002, 40003];

/// Auth-related codes that are transient — retry with backoff.
pub const AUTH_RETRYABLE_CODES: &[u32] = &[40010, 40011];

#[derive(Debug, Error)]
pub enum YuanbaoError {
    #[error("protocol encode error: {0}")]
    ProtoEncode(String),

    #[error("protocol decode error: {0}")]
    ProtoDecode(String),

    #[error("not connected")]
    NotConnected,

    #[error("connection closed: code={code}, reason={reason}")]
    ConnectionClosed { code: u16, reason: String },

    #[error("WebSocket error: {0}")]
    WebSocket(String),

    #[error("HTTP/connection error: {0}")]
    Connection(String),

    #[error("auth-bind failed: {0}")]
    AuthFailed(String),

    #[error("auth-bind timeout")]
    AuthTimeout,

    #[error("login timeout")]
    LoginTimeout,

    #[error("request timeout: {0}")]
    Timeout(String),

    #[error("send-message failed: {0}")]
    SendFailed(String),

    #[error("media error: {0}")]
    Media(String),

    #[error("invalid message: {0}")]
    InvalidMessage(String),

    #[error("config error: {0}")]
    Config(String),
}
