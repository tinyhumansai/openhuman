//! OpenAI-compatible HTTP endpoint at `/v1/chat/completions` and `/v1/models`.
//!
//! ## Mounting
//!
//! The router is mounted by `src/core/jsonrpc.rs`:
//! ```ignore
//! .nest("/v1", crate::openhuman::inference::http::router())
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

// The `/v1/*` axum router lives here and is exclusive to the `http-server`
// feature (#5048). CARVE-OUT: `EXTERNAL_OPENAI_COMPAT_PROVIDER` (above) and
// `types` stay UNGATED — `core::auth` consumes the provider id (and its inert
// request/response types are dep-free) in ALL builds, so only the axum
// `server`/`router` surface is gated.
#[cfg(feature = "http-server")]
pub mod server;
pub mod types;

#[cfg(feature = "http-server")]
pub use server::router;
