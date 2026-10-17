//! OpenAI-compatible HTTP endpoint at `/v1/chat/completions` and `/v1/models`.
//!
//! The wire types and handlers live upstream in
//! `tinyinference_llm::providers::openai::server_http` (feature `server-http`).
//! OpenHuman keeps the host side: [`backend::HostChatBackend`] resolves models
//! against `Config`, and the router is mounted by
//! `crates/openhuman-rpc/src/server/http/mod.rs`:
//! ```ignore
//! .nest("/v1", crate::core_host::inference::http::router())
//! ```
//! It inherits the core bearer-token auth middleware, but `/v1/*` also accepts
//! a stable user-managed external API key so local harnesses can treat
//! OpenHuman like an OpenAI-compatible router.

/// Auth-profile provider id used for the stable external bearer that guards
/// the OpenAI-compatible `/v1/*` endpoint.
///
/// The value is stored through the existing credentials/auth RPC surface and
/// resolved from `auth-profiles.json` on each external request. This keeps the
/// secret encrypted at rest and scoped to the active user workspace.
pub const EXTERNAL_OPENAI_COMPAT_PROVIDER: &str = "external-openai-compat";

// The axum surface is exclusive to the `http-server` feature (#5048).
// `EXTERNAL_OPENAI_COMPAT_PROVIDER` stays ungated: `core::auth` reads it in all builds.
#[cfg(feature = "http-server")]
mod backend;

/// Build the `/v1` axum sub-router over the host [`backend::HostChatBackend`].
#[cfg(feature = "http-server")]
pub fn router<S>() -> axum::Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    tinyinference_llm::providers::openai::server_http::router(std::sync::Arc::new(
        backend::HostChatBackend,
    ))
}
