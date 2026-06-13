//! Scope-lookup operational helpers for the curated tool catalogs.
//!
//! Lives in a sibling module (extracted from the formerly-thick
//! `providers/mod.rs`) to keep the module entrypoint export-focused —
//! matches the project rule "keep mod.rs light; operational logic in
//! ops.rs / store.rs / types.rs" from CLAUDE.md.
//!
//! - [`curated_scope_for`] answers "what scope does this action slug
//!   require?" — used by `composio::ops` to render `gated_tools`
//!   unlock hints.
//! - [`toolkit_has_scope`] answers "does this toolkit have any
//!   actions at the given scope?" — currently used by tests; intended
//!   for future UI hints (grey-out a toggle that unlocks nothing).
//!
//! Both walk the native provider catalog first, then fall back to the
//! static `catalog_for_toolkit` map — so the answers match what
//! [`super::is_action_visible_with_pref`] would gate against.

use super::tool_scope::{find_curated, toolkit_from_slug, ToolScope};
use super::{catalog_for_toolkit, get_provider};

/// Look up the curated scope for `slug` if it appears in any registered
/// catalog (native provider's `curated_tools()` first, then the fallback
/// catalog from [`super::catalog_for_toolkit`]). Returns `None` for
/// genuinely uncurated slugs — callers that want a defensible heuristic
/// for those should fall back to [`super::classify_unknown`] explicitly.
///
/// Sibling of [`super::is_action_visible_with_pref`]: that one decides
/// "visible?", this one returns "what scope is required?" so callers
/// (e.g. the `gated_tools` partition in
/// `composio::ops::fetch_connected_integrations`) can render a useful
/// unlock hint to the agent without re-doing the catalog walk.
pub fn curated_scope_for(slug: &str) -> Option<ToolScope> {
    let toolkit = toolkit_from_slug(slug)?;
    let catalog = get_provider(&toolkit)
        .and_then(|p| p.curated_tools())
        .or_else(|| catalog_for_toolkit(&toolkit))?;
    find_curated(catalog, slug).map(|c| c.scope)
}

/// Does any curated action for `toolkit` require `scope`?
///
/// Currently used by this module's tests only (added when the
/// now-removed `composio_enable_scope` meta-tool needed a no-op
/// short-circuit). Kept because the same probe is useful any time we
/// ask "would flipping the {scope} bit unlock anything in this
/// toolkit?" — e.g. a UI hint that greys out a toggle with no effect.
///
/// Walks both the native provider catalog and the fallback
/// [`super::catalog_for_toolkit`] so the answer matches what
/// [`super::is_action_visible_with_pref`] would gate against.
pub fn toolkit_has_scope(toolkit: &str, scope: ToolScope) -> bool {
    let catalog = get_provider(toolkit)
        .and_then(|p| p.curated_tools())
        .or_else(|| catalog_for_toolkit(toolkit));
    match catalog {
        Some(cat) => cat.iter().any(|t| t.scope == scope),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toolkit_has_scope_distinguishes_gated_from_ungated_scopes() {
        // gmail catalog includes destructive verbs (delete / trash /
        // batch_delete), so admin-gating actually unlocks something.
        assert!(toolkit_has_scope("gmail", ToolScope::Admin));
        assert!(toolkit_has_scope("gmail", ToolScope::Read));
        assert!(toolkit_has_scope("gmail", ToolScope::Write));
        // Case-insensitive toolkit slug → still routes to the catalog.
        assert!(toolkit_has_scope("GMAIL", ToolScope::Admin));
        // Unknown toolkit → no catalog → no scope is "gating" anything.
        assert!(!toolkit_has_scope("nonexistent-toolkit", ToolScope::Admin));
    }
}
