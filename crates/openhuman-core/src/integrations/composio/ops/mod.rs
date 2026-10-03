//! RPC-facing operations for the Composio domain.
//!
//! Each `composio_*` function wraps a [`ComposioClient`] call, translates
//! errors to strings, and returns an [`Outcome`] so the controller
//! schemas can log a user-visible line. The handlers in [`super::schemas`]
//! call into these.
//!
//! These ops are also callable directly from other domains (e.g. the
//! agent harness) when they need composio data at runtime.
//!
//! ## Module layout
//!
//! | Sub-module        | Contents                                                           |
//! |-------------------|--------------------------------------------------------------------|
//! | `error_utils`     | `OpResult`, `report_composio_op_error`, helpers |
//! | `toolkits`        | `composio_list_toolkits`, `composio_list_capabilities`, ...        |
//! | `connections`     | `composio_list_connections`, `composio_authorize`, `_delete_...`, `active_connection_ids` |
//! | `tools_ops`       | `composio_list_tools`                                              |
//! | `execute`         | `composio_execute`                                                 |
//! | `triggers`        | GitHub repos + trigger CRUD + trigger history                      |
//! | `providers_ops`   | `composio_get_user_profile`, `composio_refresh_all_identities`     |
//! | `sync`            | `composio_sync`, `run_sync_pass` (connector records → memory)      |
//! | `direct_mode`     | `composio_get_mode`, `composio_set_api_key`, `_clear_...`          |
//! | `user_scopes`     | per-toolkit agent scope prefs, in a workspace JSON file            |
//! | `pass_failure`    | when a connector pass failed; the sync retry schedule (openhuman#6255) |

mod connections;
mod direct_mode;
mod error_utils;
mod execute;
mod pass_failure;
mod providers_ops;
mod sync;
mod toolkits;
mod tools_ops;
mod triggers;
mod user_scopes;

// ── Public re-exports (match original ops.rs public surface) ───────────────

pub use connections::{
    active_connection_ids, composio_authorize, composio_delete_connection,
    composio_list_connections,
};
pub use direct_mode::{composio_clear_api_key, composio_get_mode, composio_set_api_key};
pub(crate) use error_utils::{report_composio_op_error, should_forward_tags};
pub use execute::composio_execute;
pub use providers_ops::{
    composio_get_user_profile, composio_refresh_all_identities, RefreshIdentitiesReport,
};
pub use sync::{composio_sync, run_sync_pass, SyncPassOutcome, SYNC_PASS_MAX_ITEMS};
pub use toolkits::{
    composio_list_agent_ready_toolkits, composio_list_capabilities, composio_list_toolkits,
};
pub use tools_ops::composio_list_tools;
pub use triggers::{
    composio_create_trigger, composio_disable_trigger, composio_enable_trigger,
    composio_list_available_triggers, composio_list_github_repos, composio_list_trigger_history,
    composio_list_triggers,
};
// The `composio.{get,set}_user_scopes` handlers' storage half.
pub(crate) use user_scopes::{
    load_or_default as load_user_scope_pref, save as save_user_scope_pref,
};

// ── Re-export connected_integrations public items ──────────────────────────
// (originally at the bottom of ops.rs)

pub use super::connected_integrations::{
    cached_active_integrations, cached_active_integrations_including_expired, connected_set_hash,
    fetch_connected_integrations, fetch_connected_integrations_status,
    invalidate_connected_integrations_cache, FetchConnectedIntegrationsStatus,
};

// ── Type aliases re-exported for callers ──────────────────────────────────

pub use super::types::{ComposioConnection as Connection, ComposioToolSchema as ToolSchemaType};

// ── Test-only re-exports (pub(crate) to match original visibility) ─────────

#[cfg(test)]
pub(crate) use super::connected_integrations::cache_key;
#[cfg(test)]
pub(crate) use super::connected_integrations::{CachedIntegrations, INTEGRATIONS_CACHE};
#[cfg(test)]
pub(crate) use crate::agent::prompts::ConnectedIntegration;
#[cfg(test)]
pub(crate) use std::time::{Duration, Instant};

// Private items needed by the test module via `use super::*`
#[cfg(test)]
pub(crate) use super::connected_integrations::sync_cache_with_connections;
#[cfg(test)]
pub(crate) use crate::config::Config;
#[cfg(test)]
pub(crate) use crate::integrations::composio::providers::SyncReason;
#[cfg(test)]
pub(crate) use connections::enrich_connections_with_identity;
#[cfg(test)]
pub(crate) use error_utils::{
    backend_mode_without_session, classify_composio_failure_tag, direct_mode_without_key,
    extract_backend_returned_status,
};
#[cfg(test)]
pub(crate) use sync::parse_sync_reason;

#[cfg(test)]
#[path = "../ops_tests.rs"]
mod tests;
