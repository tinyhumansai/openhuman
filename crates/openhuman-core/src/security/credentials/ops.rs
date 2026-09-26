//! JSON-RPC / CLI controller surface for credentials and app session auth.

mod boot_env;
mod composio;
mod credential;
mod gated_services;
mod oauth;
mod provider_credentials;
mod secrets;
mod session_query;
mod user_scope;

pub use boot_env::{
    seed_api_key_from_env, seed_session_from_env, BACKEND_API_KEY_ENV, BACKEND_SESSION_TOKEN_ENV,
};
pub use composio::{
    clear_composio_api_key, get_composio_api_key, rpc_store_composio_api_key,
    store_composio_api_key, COMPOSIO_DIRECT_PROVIDER,
};
pub use credential::{
    clear_credential, clear_session, set_credential, store_session, SetCredentialRequest,
    PENDING_BACKEND_VALIDATION_FIELD,
};
pub use gated_services::{
    start_credential_gated_services, start_login_gated_services, stop_credential_gated_services,
    stop_login_gated_services,
};
pub use oauth::oauth_fetch_client_key;
pub use provider_credentials::{
    list_provider_credentials, list_provider_credentials_by_prefix, remove_provider_credentials,
    store_provider_credentials,
};
pub use secrets::{decrypt_secret, encrypt_secret};
pub use session_query::{auth_get_session_token_json, auth_get_state};

#[cfg(test)]
pub(crate) use secrets::secret_store_for_config;

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
