//! Credential management for app session and provider auth profiles.

pub mod api_key;
pub mod backend_credential;
pub mod bus;
mod core;
pub mod credential_ref;
pub mod identity;
pub mod jwt;
pub mod openai_oauth;
pub mod ops;
pub mod profiles;
pub mod provider_credential_source;
pub mod responses;
mod schemas;
pub mod session_state;
pub mod tools;

#[cfg(feature = "security-module")]
pub(crate) use core::credential_files;
pub use core::{
    channel_credential_provider, default_profile_id, normalize_provider, select_profile_id,
    state_dir_from_config, AuthService, APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME,
};
pub use credential_ref::{CredentialRef, CredentialRefError, CredentialRefScheme, ResolvedSecret};
pub use jwt::user_id_from_profile_payload;
pub use ops::{
    auth_get_session_token_json, auth_get_state, clear_credential, clear_session, decrypt_secret,
    encrypt_secret, list_provider_credentials, list_provider_credentials_by_prefix,
    remove_provider_credentials, seed_api_key_from_env, seed_session_from_env, set_credential,
    start_credential_gated_services, stop_credential_gated_services, store_provider_credentials,
    store_session, SetCredentialRequest, BACKEND_API_KEY_ENV, BACKEND_SESSION_TOKEN_ENV,
    PENDING_BACKEND_VALIDATION_FIELD,
};
pub use profiles::{AuthProfile, AuthProfileKind, TokenSet};
pub use provider_credential_source::{
    clear_provider_credential_source, install_provider_credential_source,
    provider_credential_source, ProviderCredentialSource, ProviderOAuthStart, ProviderOAuthTokens,
};
pub use schemas::{
    all_controller_schemas as all_credentials_controller_schemas,
    all_registered_controllers as all_credentials_registered_controllers,
};
