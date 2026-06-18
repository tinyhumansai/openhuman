//! Sub-agent provider and model resolution.
//!
//! Resolves `(provider, model)` from a declarative [`ModelSpec`], plus
//! Composio sign-in probe and the lazy toolkit action resolver.

use std::sync::Arc;

use crate::openhuman::inference::provider::Provider;

// ─────────────────────────────────────────────────────────────────────────────
// Provider / model resolution
// ─────────────────────────────────────────────────────────────────────────────

/// Resolve a sub-agent's `(provider, model)` based on its declarative
/// `[model]` spec.
///
///   - inline `model` override — highest precedence for one call.
///   - config-level pin — `[orchestrator] model` or `[teams.*]`
///     `lead_model` / `agent_model`, when present.
///   - `Inherit` — use the parent's provider AND model. Literally
///     "do what the parent does".
///   - `Hint(workload)` — build a fresh provider via the per-workload
///     factory (e.g. `integrations_agent`'s `[model] hint = "agentic"`
///     resolves to whatever `agentic_provider` is routed to in
///     AI Settings). The factory returns the *exact* model id for that
///     workload — the OpenHuman backend and every third-party provider
///     accept exact model names, so there's no `{hint}-v1` synthesis
///     anywhere on this path.
///   - `Exact(name)` — escape hatch: use the parent's provider with
///     this model name overriding the parent's. Callers are expected
///     to know the model is valid for the parent's provider; the enum
///     is the wrong place to encode provider switching, which belongs
///     to `Hint` + AI-settings routing.
///
/// `config` is `None` when the live `Config::load_or_init()` failed
/// (rare — transient I/O). Both `None` config and factory build errors
/// fall back to `(parent_provider, parent_model)` so a config glitch
/// can't sink sub-agent execution entirely.
///
/// The async part (config load) is hoisted out of the caller so this
/// helper stays sync and can be exercised by a focused unit test
/// without spinning up a `tokio::test` runtime per case.
pub(crate) fn resolve_subagent_provider(
    spec: &crate::openhuman::agent::harness::definition::ModelSpec,
    agent_id: &str,
    config: Option<&crate::openhuman::config::Config>,
    parent_provider: Arc<dyn Provider>,
    parent_model: String,
    is_team_lead: bool,
    model_override: Option<&str>,
) -> (Arc<dyn Provider>, String) {
    use crate::openhuman::agent::harness::definition::ModelSpec;
    if let Some(model) = model_override
        .map(str::trim)
        .filter(|model| !model.is_empty())
    {
        log::debug!(
            "[subagent_runner] agent_id={} using inline model override model={}",
            agent_id,
            model
        );
        return (parent_provider, model.to_string());
    }

    if let Some(model) = config.and_then(|cfg| cfg.configured_agent_model(agent_id, is_team_lead)) {
        log::debug!(
            "[subagent_runner] agent_id={} using config-level model pin model={}",
            agent_id,
            model
        );
        return (parent_provider, model.to_string());
    }

    match spec {
        ModelSpec::Hint(workload) => match config {
            Some(cfg) => {
                match crate::openhuman::inference::provider::create_chat_provider(workload, cfg) {
                    Ok((p, m)) => {
                        log::info!(
                        "[subagent_runner] role={} agent_id={} resolved via workload factory model={}",
                        workload, agent_id, m
                    );
                        (std::sync::Arc::from(p), m)
                    }
                    Err(e) => {
                        let suggested_key = match workload.as_str() {
                            "summarization" | "memory" => "memory_provider".to_string(),
                            _ => format!("{workload}_provider"),
                        };
                        log::warn!(
                            "[subagent_runner] workload='{}' provider build failed for agent_id={} error='{}' \
                             falling back to parent provider (parent_model='{}'). \
                             Consider setting {} in config.",
                            workload,
                            agent_id,
                            e,
                            parent_model,
                            suggested_key
                        );
                        (parent_provider, parent_model)
                    }
                }
            }
            None => {
                log::warn!(
                    "[subagent_runner] config load failed for workload '{}' (agent_id={}) — \
                     falling back to parent provider + parent model '{}'",
                    workload,
                    agent_id,
                    parent_model
                );
                (parent_provider, parent_model)
            }
        },
        ModelSpec::Inherit => (parent_provider, parent_model),
        ModelSpec::Exact(name) => (parent_provider, name.clone()),
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
    crate::openhuman::composio::client::create_composio_client(config).is_ok()
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
/// [`crate::openhuman::composio::ComposioClient`] so the live
/// `composio.mode` toggle is honoured per execute — see
/// [`crate::openhuman::composio::ComposioActionTool`] and issue #1710.
pub(crate) struct LazyToolkitResolver {
    pub(crate) config: std::sync::Arc<crate::openhuman::config::Config>,
    pub(crate) actions: Vec<crate::openhuman::context::prompt::ConnectedIntegrationTool>,
}

/// Minimum normalized-slug length before the prefix/superstring tier in
/// [`LazyToolkitResolver::find_action`] engages (#3152). Below this, a stray
/// short slug (`notion`, `gmail`) would prefix-match too many actions; the
/// uniqueness check would reject it anyway, but the length gate makes the
/// intent explicit and skips needless scans.
const TIER4_MIN_SLUG_LEN: usize = 8;

impl LazyToolkitResolver {
    pub(super) fn resolve(&self, name: &str) -> Option<Box<dyn crate::openhuman::tools::Tool>> {
        let action = self.find_action(name)?;
        Some(Box::new(
            crate::openhuman::composio::ComposioActionTool::new(
                self.config.clone(),
                action.name.clone(),
                action.description.clone(),
                action.parameters.clone(),
            ),
        ))
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
    ) -> Option<&crate::openhuman::context::prompt::ConnectedIntegrationTool> {
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
pub(crate) fn normalize_slug(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}
