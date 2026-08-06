//! Sub-agent provider and model resolution.
//!
//! Resolves `(provider, model)` from a declarative [`ModelSpec`], plus
//! Composio sign-in probe and the lazy toolkit action resolver.

use std::sync::Arc;

pub(crate) fn resolve_subagent_source(
    spec: &crate::openhuman::agent::harness::definition::ModelSpec,
    agent_id: &str,
    config: Option<&crate::openhuman::config::Config>,
    parent_source: crate::openhuman::agent::tinyagents::TurnModelSource,
    parent_model: String,
    is_team_lead: bool,
    model_override: Option<&str>,
    temperature: f64,
) -> (crate::openhuman::agent::tinyagents::TurnModelSource, String) {
    use crate::openhuman::agent::harness::definition::ModelSpec;
    if let Some(model) = model_override
        .map(str::trim)
        .filter(|model| !model.is_empty())
    {
        tracing::debug!(
            agent_id,
            model,
            "[subagent_runner] using inline model override"
        );
        return (parent_source, model.to_string());
    }
    if let Some(model) = config.and_then(|cfg| cfg.configured_agent_model(agent_id, is_team_lead)) {
        tracing::debug!(
            agent_id,
            model,
            "[subagent_runner] using config-level model pin"
        );
        return (parent_source, model.to_string());
    }
    match spec {
        ModelSpec::Hint(workload) => match config {
            Some(config) => {
                match crate::openhuman::inference::provider::create_chat_model_with_model_id(
                    workload,
                    config,
                    temperature,
                ) {
                    Ok((_model, model_id)) => {
                        tracing::info!(
                            agent_id,
                            role = workload,
                            model = %model_id,
                            "[subagent_runner] resolved crate-native workload source"
                        );
                        (
                            crate::openhuman::agent::tinyagents::TurnModelSource::new_crate_native(
                                workload.clone(),
                                Arc::new(config.clone()),
                            ),
                            model_id,
                        )
                    }
                    Err(error) => {
                        tracing::warn!(
                            agent_id,
                            role = workload,
                            %error,
                            parent_model,
                            "[subagent_runner] workload model build failed; inheriting parent source"
                        );
                        (parent_source, parent_model)
                    }
                }
            }
            None => {
                tracing::warn!(
                    agent_id,
                    role = workload,
                    parent_model,
                    "[subagent_runner] config unavailable; inheriting parent source"
                );
                (parent_source, parent_model)
            }
        },
        ModelSpec::Inherit => (parent_source, parent_model),
        ModelSpec::Exact(model) => (parent_source, model.clone()),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Composio sign-in probe
// ─────────────────────────────────────────────────────────────────────────────

/// Probe whether the user can call Composio at all under the current
/// config. Returns `true` when the mode-aware factory can build EITHER
/// a backend-mode client (legacy JWT-driven path) OR a direct-mode
/// client (BYO Composio API key). The resolved client is dropped
/// immediately — this is purely a "signed-in vs not" check used by the
/// spawn-time refresh path. Per-action dispatch resolves a fresh client
/// elsewhere via [`create_composio_client`] so the live `composio.mode`
/// toggle keeps winning.
///
/// Extracted as a free function so the regression suite can exercise
/// the same probe the runner uses without spinning up the full
/// `run_typed_mode` plumbing.
pub(crate) fn user_is_signed_in_to_composio(config: &crate::openhuman::config::Config) -> bool {
    crate::openhuman::integrations::composio::client::create_composio_client(config).is_ok()
}

// ─────────────────────────────────────────────────────────────────────────────
// Lazy toolkit resolver
// ─────────────────────────────────────────────────────────────────────────────

/// Lazy resolver that lets `integrations_agent` recover when the model
/// calls a Composio action slug that exists in the bound toolkit's full
/// catalogue but was filtered out of the up-front fuzzy top-K. On a
/// match we build the [`ComposioActionTool`] on demand so the call
/// dispatches normally instead of dead-ending in
/// `Error: tool '...' is not available`.
///
/// Holds an [`Arc<Config>`] rather than a pre-baked
/// [`crate::openhuman::integrations::composio::ComposioClient`] so the live
/// `composio.mode` toggle is honoured per execute — see
/// [`crate::openhuman::integrations::composio::ComposioActionTool`] and issue #1710.
///
/// ## Tool caching (#5119)
///
/// `resolve()` caches the built [`ComposioActionTool`] (and therefore its
/// [`ContractGate`]) per slug so that a given action reuses the same gate
/// instance across multiple `resolve()` calls. This is forward-looking: the
/// `lazy_resolver` is not yet wired for production dispatch (#4249 1b), but
/// when it is, caching prevents the fresh-gate-per-call problem that would
/// cause every resolution to surface the contract instead of executing.
/// Additionally, the [`ContractGate`] itself has a process-wide auto-proceed
/// safety net that handles the re-delegation pattern even without caching.
pub(crate) struct LazyToolkitResolver {
    pub(super) config: std::sync::Arc<crate::openhuman::config::Config>,
    pub(super) actions: Vec<crate::openhuman::agent::context::prompt::ConnectedIntegrationTool>,
    /// Cache of resolved tools keyed by action slug. Once a tool is built
    /// for a slug, subsequent `resolve()` calls for the same slug reuse the
    /// cached instance — sharing its [`ContractGate`] state (#5119).
    #[allow(dead_code)] // used via pub(super) from tests
    pub(super) resolved: std::sync::Mutex<
        std::collections::HashMap<String, std::sync::Arc<dyn crate::openhuman::tools::Tool>>,
    >,
}

/// Minimum normalized-slug length before the prefix/superstring tier in
/// [`LazyToolkitResolver::find_action`] engages (#3152). Below this, a stray
/// short slug (`notion`, `gmail`) would prefix-match too many actions; the
/// uniqueness check would reject it anyway, but the length gate makes the
/// intent explicit and skips needless scans.
const TIER4_MIN_SLUG_LEN: usize = 8;

impl LazyToolkitResolver {
    /// Resolve a `ComposioActionTool` for `name`, caching the built tool per
    /// slug so subsequent calls for the same slug return the same
    /// [`ContractGate`] instance (#5119).
    ///
    /// ## Caching
    ///
    /// Tools are cached by slug in the `resolved` map. The first call for a
    /// slug builds the tool and stores it; subsequent calls return the cached
    /// `Arc<dyn Tool>`, sharing the [`ContractGate`] state across calls. This
    /// prevents the fresh-gate-per-call problem: the contract is surfaced at
    /// most once per resolver instance, and the retry proceeds normally.
    ///
    /// Additionally, the [`ContractGate`] has a process-wide auto-proceed
    /// safety net that fires when too many fresh gate instances have all
    /// surfaced the same contract without executing (#5119). This handles
    /// the cross-spawn re-delegation pattern even when caching is bypassed.
    pub(super) fn resolve(
        &self,
        name: &str,
    ) -> Option<std::sync::Arc<dyn crate::openhuman::tools::Tool>> {
        // Check cache first — returns the same Arc (and therefore the same
        // ContractGate) for repeated resolve calls on the same slug.
        {
            let cache = self
                .resolved
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(cached) = cache.get(name) {
                tracing::trace!(
                    target: "subagent_runner",
                    slug = %name,
                    "[subagent_runner] returning cached composio tool"
                );
                return Some(cached.clone());
            }
        }

        let action = self.find_action(name)?;
        let tool: std::sync::Arc<dyn crate::openhuman::tools::Tool> = std::sync::Arc::new(
            crate::openhuman::integrations::composio::ComposioActionTool::new(
                self.config.clone(),
                action.name.clone(),
                action.description.clone(),
                action.parameters.clone(),
            ),
        );

        // Store in cache for future lookups.
        {
            let mut cache = self
                .resolved
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            cache.insert(action.name.clone(), tool.clone());
        }

        Some(tool)
    }

    /// Match a model-supplied tool name to a real toolkit action, tolerant
    /// of the near-miss slugs models routinely emit — case differences and
    /// separator/prefix drift (bug-report-2026-05-26 A2). Tries, in order:
    /// exact, case-insensitive, then a normalized alphanumeric match
    /// (accepted only when **unique**, so a fabricated slug can't silently
    /// resolve to the wrong action — those still fall through to the
    /// "tool not available" error, which lists `known_slugs` for the model
    /// to self-correct).
    fn find_action(
        &self,
        name: &str,
    ) -> Option<&crate::openhuman::agent::context::prompt::ConnectedIntegrationTool> {
        if let Some(action) = self.actions.iter().find(|a| a.name == name) {
            return Some(action);
        }
        if let Some(action) = self
            .actions
            .iter()
            .find(|a| a.name.eq_ignore_ascii_case(name))
        {
            tracing::debug!(
                requested = %name,
                matched = %action.name,
                "[subagent_runner] resolved tool by case-insensitive match"
            );
            return Some(action);
        }
        let norm = normalize_slug(name);
        if !norm.is_empty() {
            let mut matches = self
                .actions
                .iter()
                .filter(|a| normalize_slug(&a.name) == norm);
            if let Some(action) = matches.next() {
                if matches.next().is_none() {
                    tracing::info!(
                        requested = %name,
                        matched = %action.name,
                        "[subagent_runner] resolved tool by normalized-slug match"
                    );
                    return Some(action);
                }
                // Ambiguous: 2+ actions normalize to the same slug (e.g.
                // `read_file` and `ReadFile` → `readfile`). We deliberately
                // refuse to guess. Warn (not debug): a slug collision is a
                // toolkit configuration anomaly that should surface in normal
                // operator logs, not stay hidden behind debug filtering.
                tracing::warn!(
                    requested = %name,
                    norm = %norm,
                    "[subagent_runner] ambiguous normalized-slug match — multiple actions resolve to the same slug; not resolving"
                );
                return None;
            }

            // Tier 4: unique prefix/superstring match (#3152). Models
            // routinely emit a TRUNCATED action slug — `NOTION_SEARCH_NOTION`
            // for the catalogued `NOTION_SEARCH_NOTION_PAGE` — or, less often,
            // a suffixed one. Accept only when exactly one action's normalized
            // slug extends the request (or vice-versa). Gated on a non-trivial
            // request length so a short or hallucinated slug can't fan out
            // across many actions, and strictly unique so a near-miss WRITE
            // can never silently dispatch to the wrong action (data-integrity:
            // a mis-resolved create/update would touch the wrong resource).
            if norm.len() >= TIER4_MIN_SLUG_LEN {
                let mut prefix_matches = self.actions.iter().filter(|a| {
                    let cand = normalize_slug(&a.name);
                    !cand.is_empty() && (cand.starts_with(&norm) || norm.starts_with(&cand))
                });
                if let Some(action) = prefix_matches.next() {
                    if prefix_matches.next().is_none() {
                        tracing::info!(
                            requested = %name,
                            matched = %action.name,
                            "[subagent_runner] resolved tool by unique prefix/superstring match"
                        );
                        return Some(action);
                    }
                    tracing::warn!(
                        requested = %name,
                        norm = %norm,
                        "[subagent_runner] ambiguous prefix/superstring match — multiple actions share the slug prefix; not resolving"
                    );
                }
            }
        }
        None
    }

    /// Slugs from the bound toolkit, for inclusion in unknown-tool
    /// errors so the model can self-correct without burning a turn.
    pub(super) fn known_slugs(&self) -> Vec<&str> {
        self.actions.iter().map(|a| a.name.as_str()).collect()
    }
}

/// Lowercased, non-alphanumerics stripped — collapses separator/prefix
/// drift (`GOOGLESLIDES_BATCH_UPDATE` vs `googleslides_batch_update`) so
/// near-miss tool slugs still resolve, while genuinely different slugs
/// (e.g. a hallucinated `GMAIL_GET_LAST_3_MESSAGES`) stay distinct.
pub(super) fn normalize_slug(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}
