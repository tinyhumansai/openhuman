//! Agent-facing tools that proxy through the openhuman backend's
//! `/agent-integrations/composio/*` routes.
//!
//! These expose Composio capabilities to the autonomous agent loop
//! (discovery + execution) and to the CLI/RPC surface via the normal
//! `Tool` trait plumbing in [`crate::tools`].
//!
//! The surface is intentionally small and model-friendly:
//!
//! | Tool name                     | Purpose                                                     |
//! | ----------------------------- | ----------------------------------------------------------- |
//! | `composio_list_toolkits`      | Inspect the server allowlist (e.g. `["gmail", "notion"]`)   |
//! | `composio_list_connections`   | See which accounts are already connected                    |
//! | `composio_authorize`          | Start an OAuth handoff for a toolkit, returns `connectUrl`  |
//! | `composio_list_tools`         | Discover available action slugs + their JSON schemas        |
//! | `composio_execute`            | Run a Composio action with `{tool, arguments}`              |
//!
//! Scope elevation (read/write/admin) is deliberately NOT an agent tool;
//! the user must toggle it themselves in the Connections UI.
//!
//! The agent loop is expected to chain `composio_list_tools` →
//! `composio_execute` when it needs to use a new action. The full schema
//! is returned in `composio_list_tools`'s output so the model can pick
//! the right slug and supply valid arguments without a separate round
//! trip.

mod direct;

mod authorize;
mod connect;
mod execute;
mod list_connections;
mod list_toolkits;
mod list_tools;
mod live_config;
mod registry;
mod visibility;

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "tools_tests.rs"]
mod tests;

pub use direct::{ComposioAction, ComposioConnectedAccount, ComposioTool};
pub use execute::ComposioExecuteTool;
pub use registry::all_composio_agent_tools;
pub(crate) use live_config::live_composio_config;

// Brought into this module's own namespace (private `use`, not `pub use`)
// so `tools_tests.rs` — declared as a direct child module of `tools` above
// — can still reach these via a plain `use super::*;`, exactly as when
// this was one un-split file. See each item's `pub(super)` in its owning
// submodule.
pub(crate) use visibility::{action_mutates_external_state, resolve_action_scope};

pub use authorize::ComposioAuthorizeTool;
#[cfg(test)]
use connect::{
    canonicalize_toolkit_slug, connection_is_active, parse_composio_connect_timeout,
    ComposioConnectTool, DEFAULT_COMPOSIO_CONNECT_TIMEOUT_SECS,
};
pub use list_connections::ComposioListConnectionsTool;
pub use list_toolkits::ComposioListToolkitsTool;
pub use list_tools::ComposioListToolsTool;
#[cfg(test)]
use tinytools::Tool;
#[cfg(test)]
use tinytools::{PermissionLevel, ToolCategory};
#[cfg(test)]
use visibility::{
    empty_uncurated_toolkits_message, normalized_scope_toolkits, render_tools_markdown,
    retain_connected_tools,
};
