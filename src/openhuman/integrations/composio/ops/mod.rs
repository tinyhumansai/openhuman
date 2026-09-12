//! RPC-facing operations for the Composio domain.
//!
//! Each `composio_*` function wraps a [`ComposioClient`] call, translates
//! errors to strings, and returns an [`RpcOutcome`] so the controller
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
//! | `error_utils`     | `OpResult`, `resolve_client`, `report_composio_op_error`, helpers |
//! | `toolkits`        | `composio_list_toolkits`, `composio_list_capabilities`, ...        |
//! | `connections`     | `composio_list_connections`, `composio_authorize`, `_delete_...`  |
//! | `memory_cleanup`  | Memory-cleanup helpers for connection deletion                     |
//! | `tools_ops`       | `composio_list_tools`                                              |
//! | `execute`         | `composio_execute`                                                 |
//! | `triggers`        | GitHub repos + trigger CRUD + trigger history                      |
//! | `providers_ops`   | `composio_get_user_profile`, `_refresh_...`, `composio_sync`       |
//! | `direct_mode`     | `composio_get_mode`, `composio_set_api_key`, `_clear_...`          |
//! | `user_scopes`     | per-toolkit agent scope prefs, over the bound memory driver         |

mod connections;
mod direct_mode;
mod error_utils;
mod execute;
mod memory_cleanup;
mod pass_budget;
mod providers_ops;
mod toolkits;
mod tools_ops;
mod triggers;
mod user_scopes;

// ── Public re-exports (match original ops.rs public surface) ───────────────

pub use connections::{composio_authorize, composio_delete_connection, composio_list_connections};
pub use direct_mode::{composio_clear_api_key, composio_get_mode, composio_set_api_key};
pub(crate) use error_utils::{report_composio_op_error, should_forward_tags};
pub use execute::composio_execute;
#[cfg(test)]
pub(crate) use providers_ops::{
    completed_sync_detail, completed_sync_detail_for_test, next_pass_budget,
    pick_source_sync_depth_days,
};
pub use providers_ops::{
    composio_get_user_profile, composio_refresh_all_identities, composio_sync,
    composio_sync_budgeted, composio_sync_for_source, RefreshIdentitiesReport, SYNC_PASS_MAX_ITEMS,
};
// The tinyconnectors-mediated sync pass, repeated within one call's item
// budget for the entry points that sync once per invocation (periodic tick,
// manual provider sync, `connection_created`, the Slack ingest RPC) — see
// `pass_budget`'s doc comment.
pub(crate) use pass_budget::run_sync_within_budget;
pub use toolkits::{
    composio_list_agent_ready_toolkits, composio_list_capabilities, composio_list_toolkits,
};
pub use tools_ops::composio_list_tools;
pub use triggers::{
    composio_create_trigger, composio_disable_trigger, composio_enable_trigger,
    composio_list_available_triggers, composio_list_github_repos, composio_list_trigger_history,
    composio_list_triggers,
};
// The `composio.{get,set}_user_scopes` handlers' storage half. Host code now —
// it was `tinymemory_core::sync::composio::providers::user_scopes` reached
// through the in-process engine handle until openhuman#5560; see the module.
pub(crate) use user_scopes::{
    load_or_default as load_user_scope_pref, save as save_user_scope_pref,
};

// ── Re-export connected_integrations public items ──────────────────────────
// (originally at the bottom of ops.rs)

pub use super::connected_integrations::{
    cached_active_integrations, cached_active_integrations_including_expired, connected_set_hash,
    fetch_connected_integrations, fetch_connected_integrations_status, fetch_toolkit_actions,
    invalidate_connected_integrations_cache, FetchConnectedIntegrationsStatus,
};

// ── Type aliases re-exported for callers ──────────────────────────────────

pub use super::types::{ComposioConnection as Connection, ComposioToolSchema as ToolSchemaType};

// ── Test-only re-exports (pub(crate) to match original visibility) ─────────

#[cfg(test)]
pub(crate) use super::connected_integrations::cache_key;
#[cfg(test)]
pub(crate) use super::connected_integrations::{CachedIntegrations, CACHE_TTL, INTEGRATIONS_CACHE};
#[cfg(test)]
pub(crate) use crate::openhuman::agent::context::prompt::ConnectedIntegration;
#[cfg(test)]
pub(crate) use std::time::{Duration, Instant};

// Private items needed by the test module via `use super::*`
#[cfg(test)]
pub(crate) use super::connected_integrations::sync_cache_with_connections;
#[cfg(test)]
pub(crate) use crate::openhuman::config::Config;
#[cfg(test)]
pub(crate) use crate::openhuman::integrations::composio::providers::SyncReason;
#[cfg(test)]
pub(crate) use connections::enrich_connections_with_identity;
#[cfg(test)]
pub(crate) use error_utils::{
    backend_mode_without_session, classify_composio_failure_tag, direct_mode_without_key,
    extract_backend_returned_status, resolve_client,
};
#[cfg(test)]
pub(crate) use providers_ops::parse_sync_reason;

#[cfg(test)]
#[path = "../ops_tests.rs"]
mod tests;
