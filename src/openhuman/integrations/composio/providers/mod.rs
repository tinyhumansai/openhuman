//! The Composio provider surface, sourced by what each half actually is.
//!
//! This was one line — `pub use crate::openhuman::memory::sync::composio::providers::*;`
//! — a compatibility shim over the engine crate's provider module. The glob
//! hid the fact that two very different things were coming through it, and
//! that most of what this host reads is not provider behaviour at all
//! (OpenHuman#5560).
//!
//! - **The curated catalogs, the scope verdicts and the identity vocabulary**
//!   come from the contract crate (`tinymemory-bus`/`tinymemory-api`). They
//!   are `&'static str` tables, pure functions over them, and inert payload
//!   types; nothing about answering "is this action curated, and at what
//!   scope" needs a provider, an HTTP client or a store. Every host read
//!   here — the agent's visible tool list, the `gated_tools` unlock hints,
//!   the agent-ready badge — is one of these.
//! - **The provider registry, the `ComposioProvider` trait, `ProviderContext`
//!   and the run types are gone.** tinymemory v1.13.4 deleted the in-process
//!   Composio pipeline outright (72 files, ~18.3k lines) rather than moving
//!   it behind `MemorySourceSync` — reaching a connected account needs a
//!   credential this crate must not hold. There is no registry to glob-import
//!   any more. Every former `get_provider(toolkit)` call site now either:
//!
//!     - answers from `catalog_for_toolkit`/`has_native_provider` directly
//!       (curated-tool lookups — a native provider's `curated_tools()` was
//!       always verified identical to its catalog entry, so the registry hop
//!       was pure indirection), or
//!     - calls the `tinyconnectors` module directly through
//!       [`super::module_client`] (profile fetch, action execution, sync —
//!       see `ops::providers_ops::run_sync_pass` and `ops::execute`).
//!
//!   `slack` stays re-exported below: it is this host's own RPC layer over
//!   the connector module (`memory::sync::composio::providers::slack`), not
//!   an engine provider.

// ── The contract half ───────────────────────────────────────────────────────
pub use tinymemory_api::composio::catalogs::{
    catalog_for_toolkit, curated_scope_for, has_native_provider, is_action_visible_with_pref,
    native_provider_sync_interval_secs, sync_interval_env_var, toolkit_description,
    toolkit_has_scope, CAPABILITY_TOOLKITS, NATIVE_PROVIDERS,
};
pub use tinymemory_api::composio::scopes::{
    agent_ready_toolkits, classify_unknown, find_curated, toolkit_from_slug, CuratedTool,
    ToolScope, UserScopePref,
};
pub use tinymemory_api::composio::tasks::{
    GithubFetchMode, NormalizedTask, TaskContainer, TaskFetchFilter, TaskKind,
};
pub use tinymemory_api::composio::{
    render_connected_identities_section, ConnectedIdentity, ProviderUserProfile, SyncOutcome,
    SyncReason,
};

// ── This host's own RPC layer over the connector module ────────────────────
pub use crate::openhuman::memory::sync::composio::providers::slack;
