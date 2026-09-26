//! HTTP and Socket.IO helpers for the TinyHumans / AlphaHuman hosted API.
//!
//! Use [`crate::api::config`] for default base URL and env normalization,
//! [`crate::api::jwt`] for session token retrieval and bearer formatting,
//! [`crate::api::rest`] for authenticated (bearer-only) REST calls,
//! [`crate::api::product`] for the `x-sdk-name` product identity every
//! backend-bound request carries,
//! [`crate::api::transport`] for the backend port every request rides
//! (the SDK-backed implementation lives in `openhuman-tinyhumans`),
//! [`crate::api::headers`] for the attribution headers and client profiles
//! that implementation builds from,
//! [`crate::api::classify`] for backend error-body classification shared
//! across domains,
//! and [`crate::api::socket`] for Socket.IO WebSocket URLs.
//! [`crate::api::models`] holds shared DTOs for realtime (server-adjacent).

pub mod classify;
pub mod config;
pub mod headers;
pub mod jwt;
pub mod models;
pub mod product;
pub mod rest;
pub mod socket;
pub mod transport;

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
    decrypt_handoff_blob, flatten_authed_error, user_id_from_profile_payload, BackendApiError,
    BackendOAuthClient,
};
pub use socket::websocket_url;
pub use transport::{
    install_backend_transport, installed_backend_transport, resolve_backend_transport,
    BackendRequest, BackendTransport, BackendTransportError, TransportProfile,
};
