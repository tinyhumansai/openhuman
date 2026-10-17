//! Scope gating of the Composio agent tools, as a thin adapter over the
//! connector module.
//!
//! The curated catalogs and the stored per-toolkit preference belong to
//! `tinyconnectors` (`ListCuratedCatalog` / `GetVocabulary`, `GetUserScopes`);
//! this file only asks them and turns the answer into the decisions the
//! tools need: [`resolve_action_scope`] / [`action_mutates_external_state`]
//! (sandbox and approval gates), [`evaluate_tool_visibility`] (asks the
//! module's `ListVisibleTools`), and the user-facing scope error. Listings
//! need no host filter: `ListTools` applies the stored preference itself.

use crate::config::Config;

use tinyconnectors_bus::names::methods;
use tinyconnectors_bus::vocabulary::{VisibleToolsRequest, VisibleToolsResponse};

use super::super::contract::{
    catalog_for_toolkit, classify_unknown, find_curated, toolkit_from_slug, ToolScope,
    UserScopePref,
};
use super::super::module_client as connectors;

/// Decision returned by [`evaluate_tool_visibility`].
pub(super) enum ToolDecision {
    /// Action is curated for this toolkit and user scope allows it.
    Allow,
    /// Action exists in the curated list but the user's scope blocks
    /// it. `scope` is the curated classification.
    BlockedByScope { scope: ToolScope },
    /// Action is not in the toolkit's curated whitelist (and the
    /// toolkit has one). Hidden / rejected.
    NotCurated,
    /// Toolkit has no curated catalog — pass through, but still gate by
    /// the user scope using the [`classify_unknown`] heuristic.
    PassthroughCheckScope { scope: ToolScope },
}

/// Resolve a Composio action slug to its [`ToolScope`] classification.
///
/// Prefers the toolkit's curated catalog when available (most accurate
/// — curated entries are hand-classified) and falls back to the
/// [`classify_unknown`] heuristic for un-curated toolkits. Unparseable
/// slugs default to `Write` so the sandbox gate errs on the side of
/// blocking rather than letting a potentially-mutating action slip
/// through uncategorised.
pub(crate) async fn resolve_action_scope(slug: &str) -> ToolScope {
    resolve_action_scope_sync(slug)
}

/// Synchronous core used by policy hooks that cannot await.
fn resolve_action_scope_sync(slug: &str) -> ToolScope {
    let Some(toolkit) = toolkit_from_slug(slug) else {
        return ToolScope::Write;
    };
    let catalog = catalog_for_toolkit(&toolkit);
    if let Some(cat) = catalog {
        if let Some(entry) = find_curated(&cat, slug) {
            return entry.scope;
        }
    }
    classify_unknown(slug)
}

/// Whether an action must pass through the human approval gate.
pub(crate) fn action_mutates_external_state(slug: &str) -> bool {
    matches!(
        resolve_action_scope_sync(slug),
        ToolScope::Write | ToolScope::Admin
    )
}

/// Decide whether a Composio action slug should be visible / executable
/// for the current user.
///
/// Asks the connector module which curated actions the user's stored scope
/// preference exposes for the slug's toolkit (`ListVisibleTools`), so the
/// preference and the catalog are read from the module that enforces them.
/// A module that cannot be reached answers as the default preference would,
/// which is what the host did before the preference moved.
pub(super) async fn evaluate_tool_visibility(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    slug: &str,
) -> ToolDecision {
    let Some(toolkit) = toolkit_from_slug(slug) else {
        // Unparseable slug — let the backend return its own error.
        return ToolDecision::Allow;
    };
    // Rows still in the legacy workspace file must be in the module before it
    // is asked, or its default (`write` on) would outrank a stored `write=false`.
    super::super::ops::ensure_user_scopes_migrated(ctx, config).await;
    let request = VisibleToolsRequest {
        toolkit: toolkit.clone(),
        preference: None,
    };
    match connectors::call::<_, VisibleToolsResponse>(config, methods::LIST_VISIBLE_TOOLS, request)
        .await
    {
        Ok(visible) => decide_from_visible(slug, &visible),
        Err(error) => {
            tracing::debug!(
                toolkit = %toolkit,
                "[composio][scopes] list_visible_tools failed, using the default preference: {error}"
            );
            decide_visibility(&toolkit, slug, UserScopePref::default())
        }
    }
}

/// The decision for `slug` given the module's visible-tool listing for its
/// toolkit.
///
/// A catalogued toolkit exposes exactly `visible.tools`; a curated action
/// missing from that list is blocked by its scope, and one the catalog does not
/// name at all is not curated. An uncatalogued toolkit passes through, gated by
/// the verb heuristic against the preference the module applied.
fn decide_from_visible(slug: &str, visible: &VisibleToolsResponse) -> ToolDecision {
    if visible.has_catalog {
        if visible
            .tools
            .iter()
            .any(|tool| tool.slug.eq_ignore_ascii_case(slug))
        {
            return ToolDecision::Allow;
        }
        return match catalog_for_toolkit(&visible.toolkit)
            .and_then(|catalog| find_curated(&catalog, slug).map(|tool| tool.scope))
        {
            Some(scope) => ToolDecision::BlockedByScope { scope },
            None => ToolDecision::NotCurated,
        };
    }
    let scope = classify_unknown(slug);
    if visible.preference.allows(scope) {
        ToolDecision::PassthroughCheckScope { scope }
    } else {
        ToolDecision::BlockedByScope { scope }
    }
}

/// The pure half of [`evaluate_tool_visibility`], given the toolkit's stored
/// preference. The catalog is the connector module's, read from the host's
/// curated snapshot.
fn decide_visibility(toolkit: &str, slug: &str, pref: UserScopePref) -> ToolDecision {
    match catalog_for_toolkit(toolkit) {
        Some(catalog) => match find_curated(&catalog, slug) {
            Some(curated) if pref.allows(curated.scope) => ToolDecision::Allow,
            Some(curated) => ToolDecision::BlockedByScope {
                scope: curated.scope,
            },
            None => ToolDecision::NotCurated,
        },
        None => {
            let scope = classify_unknown(slug);
            if pref.allows(scope) {
                ToolDecision::PassthroughCheckScope { scope }
            } else {
                ToolDecision::BlockedByScope { scope }
            }
        }
    }
}

/// Format a user-facing error message for a scope-blocked execution.
///
/// Embeds the unlock path in the error itself so the agent reads the
/// instruction straight off the tool response — same policy-in-data
/// approach as the `gated_tools` surface. Only ONE path: the user
/// toggles the scope in the Connections UI. The agent has no tool to
/// flip scopes (see the note above the removed `ComposioEnableScopeTool`
/// for why) — it can only describe the gate and point at the UI.
pub(super) fn scope_error_message(slug: &str, scope: ToolScope, pref: UserScopePref) -> String {
    let toolkit = toolkit_from_slug(slug).unwrap_or_default();
    let scope_str = scope.as_str();
    format!(
        "composio_execute: action `{slug}` is classified `{scope_str}` and is \
         disabled in the user's current scope preferences for `{toolkit}` \
         (read={}, write={}, admin={}). Tell the user this action requires the \
         `{scope_str}` scope and they can enable it themselves in \
         **Connections → {toolkit} → {scope_str}**. Do not claim you can flip \
         it — you cannot.",
        pref.read, pref.write, pref.admin,
    )
}

#[cfg(test)]
#[path = "scope_gate_tests.rs"]
mod tests;
