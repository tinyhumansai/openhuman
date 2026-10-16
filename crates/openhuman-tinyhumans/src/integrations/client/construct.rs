//! `IntegrationClient` type definition, backend-URL sanitisation, and the
//! constructors that wire up its HTTP transports.

use std::sync::Arc;

use crate::client::{sanitize_backend_url, IntegrationsHttp};
use crate::integrations::types::IntegrationPricing;

/// Shared client for all integration tools. Holds backend URL, auth token,
/// the download `reqwest::Client`, and a lazily-fetched pricing cache. JSON
/// traffic rides this crate's [`IntegrationsHttp`] (the shared SDK transport).
pub struct IntegrationClient {
    pub backend_url: String,
    pub auth_token: String,
    /// `auth_token` in the shape the transport takes: a session JWT rides
    /// `Authorization: Bearer`, a TinyHumans API key rides `x-api-key` (see
    /// [`crate::client::credential_headers`]). The 401 handling in `errors.rs`
    /// branches on which one it is.
    pub(super) credential:
        openhuman_embed::__host::security::credentials::session_support::BackendCredential,
    pub(super) budget_config: Option<Arc<openhuman_embed::__host::config::Config>>,
    // The binary download path never rode the SDK: file storage also consumes
    // Content-Type and Content-Disposition, and the presigned-redirect hop
    // must not carry attribution headers (see `new_inner`).
    pub(super) download_client: reqwest::Client,
    /// Follows storage redirects without inheriting any backend credential.
    pub(super) presigned_client: reqwest::Client,
    pub(super) pricing: tokio::sync::OnceCell<IntegrationPricing>,
    pub(super) http: IntegrationsHttp,
}

impl IntegrationClient {
    /// A client authenticating with a session JWT.
    pub fn new(backend_url: String, auth_token: String) -> Self {
        Self::new_inner(
            backend_url,
            openhuman_embed::__host::security::credentials::session_support::BackendCredential::Session(auth_token),
            None,
        )
    }

    /// A client authenticating with whichever credential
    /// [`resolve_backend_credential`](openhuman_embed::__host::security::credentials::session_support::resolve_backend_credential)
    /// chose: the TinyHumans API key or the session JWT.
    pub fn new_with_credential(
        backend_url: String,
        credential: openhuman_embed::__host::security::credentials::session_support::BackendCredential,
    ) -> Self {
        Self::new_inner(backend_url, credential, None)
    }

    /// Credential-aware variant of [`Self::new_with_budget_config`].
    pub fn new_with_credential_and_budget_config(
        backend_url: String,
        credential: openhuman_embed::__host::security::credentials::session_support::BackendCredential,
        config: Arc<openhuman_embed::__host::config::Config>,
    ) -> Self {
        Self::new_inner(backend_url, credential, Some(config))
    }

    /// Whether this client authenticates with a TinyHumans API key rather
    /// than a session JWT.
    pub fn uses_api_key(&self) -> bool {
        self.credential.is_api_key()
    }

    /// The auth headers for a request built outside the backend transport
    /// (binary download, raw DELETE): `Authorization: Bearer` for a session,
    /// `x-api-key` for an API key.
    pub(crate) fn auth_headers(&self) -> anyhow::Result<reqwest::header::HeaderMap> {
        self.http.auth_headers()
    }

    pub(super) fn validate_credential_endpoint(&self) -> anyhow::Result<()> {
        self.http.validate_credential_endpoint()
    }

    fn new_inner(
        backend_url: String,
        credential: openhuman_embed::__host::security::credentials::session_support::BackendCredential,
        budget_config: Option<Arc<openhuman_embed::__host::config::Config>>,
    ) -> Self {
        // Defense-in-depth (issue #2075 / Sentry OPENHUMAN-TAURI-H6, -HN):
        // every prod call site routes `backend_url` through
        // `effective_backend_api_url` which strips inference-style paths,
        // but any future caller that forgets that step would silently
        // produce 404 URLs like
        //   https://api.tinyhumans.ai/openai/v1/chat/completions/agent-integrations/composio/connections
        // (the inference path concatenated with every domain path). We
        // re-strip here so the field invariant — "backend_url has no
        // inference path" — holds locally, and `warn!` once when we have
        // to fix up the input so the regression is observable in logs.
        let backend_url = sanitize_backend_url(&backend_url);

        // JSON traffic goes through the shared SDK transport
        // (`TransportProfile::Integrations`: platform TLS, 60 s timeout,
        // product identity). The binary download clients are shared by
        // `IntegrationsHttp` and deliberately do NOT carry the product
        // identity: their one caller (`get_bytes`) fetches a route that can
        // redirect to presigned storage. The first stops at the redirect; the
        // second follows it without x-api-key or any other backend header.
        let http = IntegrationsHttp::new(&backend_url, credential.clone());
        let download_client = http.download_client();
        let presigned_client = http.presigned_client();

        let auth_token = credential.secret().to_owned();
        Self {
            backend_url,
            auth_token,
            credential,
            budget_config,
            download_client,
            presigned_client,
            pricing: tokio::sync::OnceCell::new(),
            http,
        }
    }
}
