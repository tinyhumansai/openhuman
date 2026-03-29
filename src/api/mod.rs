//! HTTP and Socket.IO helpers for the TinyHumans / AlphaHuman hosted API.
//!
//! Use [`crate::api::config`] for default base URL and env normalization,
//! [`crate::api::jwt`] for session token retrieval and bearer formatting,
//! [`crate::api::rest`] for authenticated REST calls (`/auth/...`, `GET /settings`, etc.),
//! and [`crate::api::socket`] for Socket.IO WebSocket URLs.
//! [`crate::api::models`] holds shared DTOs for auth and realtime (server-adjacent).

pub mod config;
pub mod jwt;
pub mod models;
pub mod rest;
pub mod socket;

pub use config::{
    api_base_from_env, effective_api_url, normalize_api_base_url, DEFAULT_API_BASE_URL,
};
pub use jwt::{bearer_authorization_value, get_session_token};
pub use rest::{
    decrypt_handoff_blob, user_id_from_settings_payload, BackendOAuthClient, ConnectResponse,
    IntegrationSummary, IntegrationTokensHandoff,
};
pub use socket::websocket_url;
