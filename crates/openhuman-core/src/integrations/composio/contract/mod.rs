//! The Composio vocabulary the host's integration code shares: the toolkit
//! catalogs (curated action lists and descriptions, read from the connector
//! module), connected-identity
//! profiles and the normalised task shape. The scope vocabulary
//! ([`ToolScope`], [`UserScopePref`], [`CuratedTool`]) is the connector
//! contract's, re-exported here for the host's callers.
//!
//! These shapes live beside the integration that produces and reads them.
//! They are plain data and pure functions: no HTTP, no persistence.

pub mod curated;
pub mod identity;
pub mod validation;

pub use identity::{
    canonicalize_values, normalize_connection_identifiers, render_connected_identities_section,
};
pub use tinyconnectors_bus::vocabulary::{
    action_slug_matches, canonical_action_slug, classify_unknown, find_curated, toolkit_from_slug,
    ConnectedIdentity, CuratedTool, GithubFetchMode, IdentityKind, NormalizedTask,
    ProviderUserProfile, TaskContainer, TaskFetchFilter, TaskKind, ToolScope, UserScopePref,
};

pub use curated::{
    agent_ready_toolkits, catalog_for_toolkit, curated_scope_for, curated_toolkits,
    has_native_provider, is_action_visible_with_pref, toolkit_description,
};
