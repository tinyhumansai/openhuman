//! HTTP and Socket.IO helpers for the TinyHumans / AlphaHuman hosted API.
//!
//! Use [`crate::api::config`] for default base URL and env normalization,
//! [`crate::api::jwt`] for session token retrieval and bearer formatting,
//! [`crate::api::rest`] for authenticated REST calls (`/auth/...`, `GET /auth/me`, etc.),
//! [`crate::api::product`] for the `x-sdk-name` product identity every
//! backend-bound request carries,
//! and [`crate::api::socket`] for Socket.IO WebSocket URLs.
//! [`crate::api::models`] holds shared DTOs for auth and realtime (server-adjacent).

pub mod config;
pub mod jwt;
pub mod models;
pub mod product;
pub mod rest;
pub mod socket;

pub use config::{
    api_base_from_env, effective_api_url, effective_backend_api_url, normalize_api_base_url,
    DEFAULT_API_BASE_URL,
};
pub use jwt::{bearer_authorization_value, get_session_token};
pub use product::{
    product_identity, product_identity_header, product_identity_headers, set_product_identity,
    ProductIdentity, DEFAULT_PRODUCT_IDENTITY, PRODUCT_IDENTITY_HEADER,
};
pub use rest::{
    decrypt_handoff_blob, flatten_authed_error, user_id_from_auth_me_payload,
    user_id_from_profile_payload, BackendApiError, BackendOAuthClient, ConnectResponse,
    IntegrationSummary, IntegrationTokensHandoff,
};
pub use socket::websocket_url;
