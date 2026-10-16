//! The authenticated hosted-backend client, owned by this crate.
//!
//! The client sends through this crate's [`SdkBackendTransport`](crate::SdkBackendTransport)
//! and resolves URLs from [`crate::backend::url`]. Every hosted caller in this
//! crate uses it; the core carries no backend client.
//!
//! - [`BackendClient`]: `authed_json`, the channel routes (`channel.rs`), the
//!   credential/endpoint guards and the error classification (`classify.rs`).
//! - [`HostedApiError`] / [`flatten_authed_error`]: the typed expected states
//!   and the sentinel strings the core's observability classifies.
//! - [`IntegrationsHttp`]: the `/agent-integrations/*` transport helper.
//! - [`endpoint`]: where a credential may be sent.
//! - [`enricher`]: fills `Config.resolved_endpoints` (registered by
//!   [`register`]).

mod backend_client;
mod channel;
mod classify;
pub mod endpoint;
pub mod enricher;
mod error;
mod integrations_http;

pub(crate) use backend_client::reset_shared_transport;
pub use backend_client::{shared_transport, BackendClient};
pub use endpoint::{
    is_managed_endpoint_for_api_key, is_safe_endpoint_for_managed_bearer,
    validate_credential_endpoint,
};
pub use error::{flatten_authed_error, HostedApiError};
pub use integrations_http::{
    credential_headers, reject_privileged_backend_path, sanitize_backend_url, IntegrationsHttp,
    API_KEY_HEADER,
};

use crate::install::InstallOptions;

/// Registers this area's hooks with the core. Idempotent.
pub(crate) fn register(_options: &InstallOptions) {
    enricher::register();
}
