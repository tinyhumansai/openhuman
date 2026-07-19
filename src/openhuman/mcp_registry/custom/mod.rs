//! Custom (user-entered) MCP server installs.
//!
//! The paths in `ops.rs` resolve a server's command/args/url by fetching an
//! upstream catalog listing keyed by `qualified_name`. This module covers the
//! servers that have no listing: the user types the launch command (stdio) or
//! the endpoint URL (http_remote) in directly.
//!
//! Only the *provenance* differs. The record written here is an ordinary
//! [`InstalledServer`] carrying [`ServerProvenance::Custom`], so
//! [`super::connections`], the supervisor, boot spawn, and the agent tool
//! surface treat it exactly like a catalog install — which is what keeps
//! OAuth, redirect resolution, and the tool safety filter working here without
//! a second transport implementation.

use std::collections::HashMap;

/// Namespace for generated `qualified_name`s. Keeps hand-entered servers from
/// ever colliding with a catalog name (no registry publishes under `custom/`).
pub(super) const CUSTOM_QUALIFIED_PREFIX: &str = "custom/";

/// Env keys starting with this are reserved for internal connection state —
/// `__oauth__` holds the OAuth refresh bundle. `connections::build_http_auth`
/// filters them out of outgoing headers, so a user-created one would silently
/// do nothing on http_remote while risking a collision with OAuth storage.
pub(super) const RESERVED_ENV_PREFIX: &str = "__";

/// Upper bound on slug de-duplication attempts before giving up. Only reached
/// if a user really has this many servers sharing one display name.
pub(super) const MAX_SLUG_ATTEMPTS: usize = 100;

/// The user-editable half of a custom server record, as submitted by the add /
/// edit form. Identity (`server_id`, `qualified_name`, `installed_at`) and
/// provenance are owned by this module, never by the caller.
#[derive(Debug, Clone, Default)]
pub struct CustomServerInput {
    pub display_name: String,
    /// `"stdio"` or `"http_remote"` — matches [`Transport::dispatch_kind`].
    pub transport: String,
    /// Launch binary for stdio servers (`npx`, `uvx`, an absolute path, …).
    pub command: Option<String>,
    /// Arguments passed to `command`. Ignored for http_remote.
    pub args: Vec<String>,
    /// Endpoint for http_remote servers. Ignored for stdio.
    pub url: Option<String>,
    /// stdio: environment variables for the subprocess.
    /// http_remote: request headers (key = header name) — the convention
    /// `connections::build_http_auth` already reads for catalog installs.
    pub env: HashMap<String, String>,
    pub description: Option<String>,
}

pub mod ops;
mod validate;
