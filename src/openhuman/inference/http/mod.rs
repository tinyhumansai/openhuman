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

pub mod server;
pub mod types;

pub use server::router;
