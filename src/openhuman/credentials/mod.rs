//! Backend OAuth HTTP client (`/auth/...`) and JSON-RPC surface (`rpc`, `cli`).
//! Persistent session and profile storage live in [`crate::openhuman::auth_profiles`].

pub mod backend_oauth;
pub mod cli;
pub mod rpc;

pub use crate::openhuman::auth_profiles::profiles;
pub use crate::openhuman::auth_profiles::responses;
pub use crate::openhuman::auth_profiles::session_support;
pub use crate::openhuman::auth_profiles::{
    AuthService, APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME,
};

pub use backend_oauth::BackendOAuthClient;
