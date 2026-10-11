//! [`fetch_connected_integrations_uncached`] — the actual mode-aware
//! backend/direct fetch behind [`super::fetch::fetch_connected_integrations_status`]'s
//! cache miss path. Kept in its own file because it is one large,
//! sequential fetch-then-merge routine that resists a further
//! responsibility split without changing its behavior.

use crate::agent::prompts::{ConnectedIntegration, ConnectedIntegrationTool, GatedIntegrationTool};
use crate::config::Config;

use super::fetch::{connectable_toolkit_slugs, resolve_toolkit_description};

/// The actual backend fetch, called on cache miss.
///
/// Returns `Some(vec)` when the backend was reachable. The returned
/// vector is the merged **integration overview** — every toolkit in
/// the backend allowlist appears as one entry, with a `connected`
/// flag indicating whether the user has an active OAuth connection.
/// Connected entries also carry the per-action tool catalogue
/// (fetched in a single batched call).
///
/// Returns `None` when we couldn't even build a client (no auth),
/// signalling the caller should NOT cache this result.
///
/// Returned boxed and `#[inline(never)]` on purpose: an `async fn` body is
/// otherwise re-instantiated inside every crate / codegen unit that awaits it,
/// and these state machines are large. Boxing here keeps one copy, compiled in
/// this crate.
#[inline(never)]
pub(super) fn fetch_connected_integrations_uncached(
    config: &Config,
) -> futures::future::BoxFuture<'_, Option<Vec<ConnectedIntegration>>> {
    Box::pin(fetch_connected_integrations_uncached_inner(config))
}

async fn fetch_connected_integrations_uncached_inner(
    config: &Config,
) -> Option<Vec<ConnectedIntegration>> {
    use super::super::client::{direct_list_connections, resolve_composio_route, ComposioRoute};
    use super::super::module_client::{self as connectors};
    use super::super::types::{
        ComposioConnectionsResponse, ComposioListToolsRequest, ComposioToolkitsResponse,
        ComposioToolsResponse,
    };
    use tinyconnectors_bus::names::methods;

    // Route via the mode-aware factory so the chat-agent's
    // "connected_integrations" view reflects the live tenant — backend
    // (tinyhumans) or direct (user's personal Composio). Prior to #1710
    // Wave 3 this path called `build_composio_client` directly, which
    // is backend-only — after a `composio.mode = "direct"` toggle the
    // cache kept replaying the tinyhumans-tenant connections back into
    // the integration overview (e.g. gmail / notion appearing as
    // connected in direct mode even when the user's direct tenant had
    // a different set of toolkits). Resolving per call closes the
    // loop: `ComposioConfigChangedSubscriber` invalidates the cache on
    // toggle and the next miss re-populates it from the live tenant.
    let kind = match resolve_composio_route(config) {
        Ok(kind) => kind,
        Err(e) => {
            tracing::debug!(
                error = %e,
                "[composio] fetch_connected_integrations: no client (not signed in?)"
            );
            return None;
        }
    };

    // Pull the allowlist + connections + tool catalogue. Backend mode
    // walks the tinyhumans tenant's curated allowlist via
    // `list_toolkits`; direct mode has no centralised allowlist (per
    // `ops::composio_list_toolkits`'s direct-mode branch) so the
    // user's set of active connections IS the universe of valid
    // toolkit arguments.
    //
    // On transient errors we return `None` instead of a degraded
    // `Some(Vec::new())` so `fetch_connected_integrations` does NOT
    // cache the failure. Caching an empty allowlist would hide every
    // integration from the orchestrator until the process restarts or
    // the cache is explicitly invalidated — a single 5xx during
    // startup would silently break delegation for the whole session.
    let (allowlisted_toolkits, connections, tools_by_toolkit, catalog_descriptions): (
        Vec<String>,
        Vec<super::super::types::ComposioConnection>,
        Vec<super::super::types::ComposioToolSchema>,
        std::collections::HashMap<String, String>,
    ) = match &kind {
        ComposioRoute::Backend => {
            let (allowlist, catalog_descriptions): (
                Vec<String>,
                std::collections::HashMap<String, String>,
            ) = match connectors::call_bare::<ComposioToolkitsResponse>(
                config,
                methods::LIST_TOOLKITS,
            )
            .await
            {
                Ok(resp) => {
                    // Index the dynamic catalog's descriptions by lowercased
                    // slug so the prompt builder can prefer them over the
                    // hardcoded table (see `resolve_toolkit_description`).
                    // Empty descriptions are skipped so the fallback applies.
                    let catalog_descriptions: std::collections::HashMap<String, String> = resp
                        .catalog
                        .iter()
                        .filter_map(|entry| {
                            let slug = entry.slug.trim().to_ascii_lowercase();
                            if slug.is_empty() {
                                return None;
                            }
                            let desc = entry
                                .description
                                .as_deref()
                                .map(str::trim)
                                .filter(|d| !d.is_empty())?;
                            Some((slug, desc.to_string()))
                        })
                        .collect();
                    // Source the connectable set from the catalog's enabled
                    // entries (the gate's own predicate), falling back to the
                    // flat `toolkits` array for pre-catalog backends. See
                    // `connectable_toolkit_slugs`.
                    let allowlist = connectable_toolkit_slugs(&resp.toolkits, &resp.catalog);
                    tracing::debug!(
                        catalog_entries = resp.catalog.len(),
                        catalog_descriptions = catalog_descriptions.len(),
                        connectable = allowlist.len(),
                        catalog_sourced = !resp.catalog.is_empty(),
                        "[composio] fetch_connected_integrations: resolved connectable set from catalog"
                    );
                    (allowlist, catalog_descriptions)
                }
                Err(e) => {
                    tracing::warn!(
                        "[composio] fetch_connected_integrations: list_toolkits (backend) failed: {e}"
                    );
                    return None;
                }
            };

            if allowlist.is_empty() {
                tracing::debug!(
                    "[composio] fetch_connected_integrations: backend allowlist is empty"
                );
                return Some(Vec::new());
            }

            let connections = match connectors::call_bare::<ComposioConnectionsResponse>(
                config,
                methods::LIST_CONNECTIONS,
            )
            .await
            {
                Ok(resp) => resp.connections,
                Err(e) => {
                    tracing::warn!(
                        "[composio] fetch_connected_integrations: list_connections (backend) failed: {e}"
                    );
                    return None;
                }
            };

            // Tool catalogue scoped to the active subset only —
            // not-connected toolkits won't be invoked from a sub-agent.
            let connected_slugs_for_tools: Vec<String> = {
                let mut v: Vec<String> = connections
                    .iter()
                    .filter(|c| c.is_active())
                    .map(|c| c.normalized_toolkit())
                    .filter(|t| !t.is_empty())
                    .collect();
                v.sort();
                v.dedup();
                v
            };
            let tools = if connected_slugs_for_tools.is_empty() {
                Vec::new()
            } else {
                match connectors::call::<_, ComposioToolsResponse>(
                    config,
                    methods::LIST_TOOLS,
                    ComposioListToolsRequest {
                        toolkits: connected_slugs_for_tools.clone(),
                        tags: Vec::new(),
                        apply_user_scopes: false,
                    },
                )
                .await
                {
                    Ok(resp) => resp.tools,
                    Err(e) => {
                        tracing::warn!(
                            "[composio] fetch_connected_integrations: list_tools (backend) failed: {e}"
                        );
                        return None;
                    }
                }
            };

            (allowlist, connections, tools, catalog_descriptions)
        }
        ComposioRoute::Direct(direct) => {
            // Direct mode: walk the user's personal Composio tenant
            // for *connection state* (active accounts on their key) —
            // there's no central allowlist in direct mode, so the
            // active set IS the allowlist.
            //
            // Tool *schemas* are tenant-agnostic — Composio's action
            // definitions (e.g. GMAIL_SEND_EMAIL parameter shape) are
            // identical regardless of which Composio tenant a user is
            // connected via. So we best-effort fetch schemas through
            // the backend client (curated list_tools) if a backend
            // session is available, even though connection routing
            // goes to the user's direct tenant. This preserves the
            // chat agent's "21 gmail actions available" view while
            // execution itself (via `ComposioActionTool` / Wave 1
            // factory) still routes to the user's tenant. Direct-only
            // users without a backend session get empty tools — that
            // matches `composio_list_tools`'s direct-mode policy.
            let connections = match direct_list_connections(config, direct).await {
                Ok(resp) => resp.connections,
                Err(e) => {
                    tracing::warn!(
                        "[composio] fetch_connected_integrations: list_connections (direct) failed: {e:#}"
                    );
                    return None;
                }
            };
            let allowlist: Vec<String> = {
                let mut v: Vec<String> = connections
                    .iter()
                    .filter(|c| c.is_active())
                    .map(|c| c.normalized_toolkit())
                    .filter(|t| !t.is_empty())
                    .collect();
                v.sort();
                v.dedup();
                v
            };
            if allowlist.is_empty() {
                tracing::info!(
                    "[composio-direct] fetch_connected_integrations: direct tenant has no active connections; returning empty overview"
                );
                return Some(Vec::new());
            }
            tracing::debug!(
                connected = allowlist.len(),
                "[composio-direct] fetch_connected_integrations: using direct tenant's active set as allowlist (no central allowlist in direct mode)"
            );

            // Tool schemas are tenant-agnostic, but direct mode must keep
            // every Composio request on the user's direct route. Failure is
            // non-fatal — lazy resolution can still discover actions later.
            let tools = match super::super::client::direct_list_tools(
                config, direct, &allowlist, None,
            )
            .await
            {
                Ok(resp) => {
                    tracing::debug!(
                        count = resp.tools.len(),
                        "[composio-direct] fetch_connected_integrations: pulled tool schemas from direct route"
                    );
                    resp.tools
                }
                Err(e) => {
                    tracing::info!(
                        "[composio-direct] fetch_connected_integrations: direct list_tools failed (will use lazy fallback at delegation time): {e:#}"
                    );
                    Vec::new()
                }
            };
            // Direct mode has no central catalog endpoint, so prompt
            // descriptions come from the hardcoded fallback table.
            (
                allowlist,
                connections,
                tools,
                std::collections::HashMap::new(),
            )
        }
    };

    // Active connection slugs (status filter mirrors the original logic).
    let connected_slugs: std::collections::HashSet<String> = connections
        .iter()
        .filter(|c| c.is_active())
        .map(|c| c.normalized_toolkit())
        .filter(|toolkit| !toolkit.is_empty())
        .collect();

    // Most-informative *non-active* status per toolkit slug (#2365):
    // distinguishes a connection row that isn't usable yet (`INITIATED`
    // — OAuth still in progress) or any longer (`EXPIRED` / `FAILED`)
    // — instead of the legacy generic "available but not authorized".
    //
    // Status priority (UI-actionability):
    //   1. EXPIRED  — reconnect path
    //   2. FAILED / ERROR — reconnect path
    //   3. INITIATED / INITIALIZING / PENDING — finish OAuth in browser
    //   4. anything else — passes through verbatim
    let non_active_status_by_slug: std::collections::HashMap<String, String> = {
        fn priority(status: &str) -> u8 {
            let s = status.trim().to_ascii_uppercase();
            match s.as_str() {
                "EXPIRED" => 4,
                "FAILED" | "ERROR" => 3,
                "INITIATED" | "INITIALIZING" | "PENDING" => 2,
                _ => 1,
            }
        }
        let mut map: std::collections::HashMap<String, (u8, String)> =
            std::collections::HashMap::new();
        for conn in connections.iter().filter(|c| !c.is_active()) {
            let slug = conn.normalized_toolkit();
            if slug.is_empty() {
                continue;
            }
            // Don't override an ACTIVE-slug — those carry no non-active
            // status from this map's perspective.
            if connected_slugs.contains(&slug) {
                continue;
            }
            let p = priority(&conn.status);
            map.entry(slug.clone())
                .and_modify(|cur| {
                    if p > cur.0 {
                        tracing::debug!(
                            target: "composio",
                            toolkit = %slug,
                            previous_status = %cur.1,
                            previous_priority = cur.0,
                            new_status = %conn.status,
                            new_priority = p,
                            "[composio] non_active_status_by_slug: upgraded most-informative status"
                        );
                        *cur = (p, conn.status.clone());
                    } else {
                        tracing::trace!(
                            target: "composio",
                            toolkit = %slug,
                            kept_status = %cur.1,
                            kept_priority = cur.0,
                            candidate_status = %conn.status,
                            candidate_priority = p,
                            "[composio] non_active_status_by_slug: kept higher-priority status"
                        );
                    }
                })
                .or_insert_with(|| {
                    tracing::debug!(
                        target: "composio",
                        toolkit = %slug,
                        status = %conn.status,
                        priority = p,
                        "[composio] non_active_status_by_slug: first non-active row"
                    );
                    (p, conn.status.clone())
                });
        }
        let final_map: std::collections::HashMap<String, String> =
            map.into_iter().map(|(k, (_, v))| (k, v)).collect();
        tracing::debug!(
            target: "composio",
            entries = final_map.len(),
            "[composio] non_active_status_by_slug: final map"
        );
        final_map
    };

    // Deduplicate the allowlist so a backend that returns duplicates
    // doesn't produce dual entries downstream.
    let mut unique_toolkits: Vec<String> = allowlisted_toolkits.clone();
    unique_toolkits.sort();
    unique_toolkits.dedup();

    // Build one entry per allowlisted toolkit. Connected entries
    // carry their action catalogue; not-connected entries carry an
    // empty `tools` vec.
    let mut integrations: Vec<ConnectedIntegration> = Vec::with_capacity(unique_toolkits.len());
    for slug in &unique_toolkits {
        let connected = connected_slugs.contains(slug);
        // Anchor the prefix with an underscore so slugs that share
        // a text prefix (e.g. `git` vs `github`) don't false-match
        // each other's actions. `GMAIL_SEND_EMAIL` matches `gmail_`,
        // not just `gmail`, so siblings stay in their own buckets.
        let action_prefix = format!("{}_", slug.to_uppercase());
        let (tools, gated_tools): (Vec<ConnectedIntegrationTool>, Vec<GatedIntegrationTool>) =
            if connected {
                // Apply the same curated-whitelist + user-scope filter the
                // meta-tool layer uses, so a prompt only advertises actions
                // the agent is actually allowed to call. One pref load per toolkit (not per action).
                //
                // Actions that the catalog *does* know about but the user's
                // current scope pref denies are routed into `gated_tools` so
                // the agent can honestly answer "I have this capability but
                // it needs the {scope} toggle in Connections → {toolkit}".
                // The agent cannot flip the scope itself — that's a UI-only
                // action. Without this gated surface the LLM has no way to
                // know the gated action exists at all and will tell the user
                // "I don't support that" — technically correct about its
                // callable surface, but misleading about the toolkit.
                let pref = super::super::ops::load_user_scope_pref(config, slug).await;
                let mut visible: Vec<ConnectedIntegrationTool> = Vec::new();
                let mut gated: Vec<GatedIntegrationTool> = Vec::new();
                for t in tools_by_toolkit
                    .iter()
                    .filter(|t| t.function.name.starts_with(&action_prefix))
                {
                    if super::super::contract::is_action_visible_with_pref(&t.function.name, &pref)
                    {
                        visible.push(ConnectedIntegrationTool {
                            name: t.function.name.clone(),
                            description: t.function.description.clone().unwrap_or_default(),
                            parameters: t.function.parameters.clone(),
                        });
                    } else if let Some(required_scope) =
                        super::super::contract::curated_scope_for(&t.function.name)
                    {
                        // Only surface CURATED actions as `gated` — uncurated
                        // tools (which fall through to `classify_unknown` and
                        // happen to land outside the user's pref) are not
                        // first-class user-facing capabilities, and listing
                        // them would clutter the prompt with internal slugs.
                        // Deliberately NO `parameters` field: the LLM should
                        // not be able to construct a call envelope; it can
                        // only describe + point at the unlock path.
                        // Ship the unlock path as data — single path today
                        // (the Connections UI toggle). The agent does NOT
                        // have a tool to flip scopes; that capability was
                        // removed because LLM-mediated scope elevation made
                        // the safety contract depend on model behavior and
                        // was a soft gate the model could route around. If
                        // more unlock paths exist in future (per-action
                        // approval modal, time-boxed elevation, etc.) they
                        // land here and the prompt renderer picks them up.
                        let scope_str = required_scope.as_str();
                        let unlock_paths = vec![format!(
                            "the user enables it themselves in \
                             Connections → {slug} → {scope_str}"
                        )];
                        gated.push(GatedIntegrationTool {
                            name: t.function.name.clone(),
                            description: t.function.description.clone().unwrap_or_default(),
                            required_scope: scope_str.to_string(),
                            unlock_paths,
                        });
                    }
                }
                tracing::debug!(
                    toolkit = %slug,
                    visible = visible.len(),
                    gated = gated.len(),
                    "[composio][scopes] integrations prompt action set"
                );
                (visible, gated)
            } else {
                (Vec::new(), Vec::new())
            };

        let integration_connections: Vec<crate::agent::prompts::IntegrationConnection> =
            if connected {
                let mut conns: Vec<_> = connections
                    .iter()
                    .filter(|c| c.is_active() && c.normalized_toolkit() == *slug)
                    .collect();
                conns.sort_by(|a, b| a.created_at.cmp(&b.created_at));
                conns
                    .iter()
                    .enumerate()
                    .map(|(idx, c)| {
                        let label = [
                            c.account_email.as_deref(),
                            c.workspace.as_deref(),
                            c.username.as_deref(),
                        ]
                        .into_iter()
                        .flatten()
                        .map(str::trim)
                        .find(|s| !s.is_empty())
                        .map(str::to_string);
                        crate::agent::prompts::IntegrationConnection {
                            connection_id: c.id.clone(),
                            label,
                            is_default: idx == 0,
                        }
                    })
                    .collect()
            } else {
                Vec::new()
            };

        integrations.push(ConnectedIntegration {
            toolkit: slug.clone(),
            description: resolve_toolkit_description(&catalog_descriptions, slug),
            tools,
            gated_tools,
            connected,
            connections: integration_connections,
            non_active_status: if connected {
                None
            } else {
                non_active_status_by_slug.get(slug).cloned()
            },
        });
    }

    integrations.sort_by(|a, b| a.toolkit.cmp(&b.toolkit));

    let connected_count = integrations.iter().filter(|i| i.connected).count();
    tracing::info!(
        total = integrations.len(),
        connected = connected_count,
        "[composio] fetch_connected_integrations: done"
    );
    for ci in &integrations {
        tracing::debug!(
            toolkit = %ci.toolkit,
            connected = ci.connected,
            non_active_status = ?ci.non_active_status,
            tool_count = ci.tools.len(),
            "[composio] integration overview"
        );
    }

    Some(integrations)
}
