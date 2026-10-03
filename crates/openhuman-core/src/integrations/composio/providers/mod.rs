//! The Composio provider surface: curated catalogs, scope verdicts, the
//! identity vocabulary and the run types, all from
//! [`crate::integrations::composio::contract`]. They are `&'static str`
//! tables, pure functions and inert payload types. Reading a connected
//! account goes through the connector module ([`super::module_client`]):
//! profile fetch, action execution and sync (`ops::sync::run_sync_pass`).

// ── The contract half ───────────────────────────────────────────────────────
pub use crate::integrations::composio::contract::catalogs::{
    catalog_for_toolkit, curated_scope_for, has_native_provider, is_action_visible_with_pref,
    toolkit_description,
};
pub use crate::integrations::composio::contract::scopes::{
    agent_ready_toolkits, classify_unknown, find_curated, toolkit_from_slug, CuratedTool,
    ToolScope, UserScopePref,
};
pub use crate::integrations::composio::contract::tasks::{
    GithubFetchMode, NormalizedTask, TaskContainer, TaskFetchFilter, TaskKind,
};
pub use crate::integrations::composio::contract::{
    render_connected_identities_section, ConnectedIdentity, ProviderUserProfile, SyncOutcome,
    SyncReason,
};
