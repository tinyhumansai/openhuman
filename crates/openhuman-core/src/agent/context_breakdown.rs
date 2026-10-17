//! RPC `agent.context_breakdown` — a UI-friendly view over
//! [`PromptSizeReport`](super::debug::prompt_size::PromptSizeReport) plus,
//! when a `thread_id` is given, that thread's persisted history spend, so
//! the composer's context-usage indicator can show a system/tools/history
//! split instead of just the fixed per-turn prefix.
//!
//! [`PromptSizeReport::build`] is expensive: it rebuilds a real agent through
//! `OpenHumanSessionHost::from_config_for_agent` and fetches live Composio
//! connections. This module caches the last report per `agent_id`, keyed
//! additionally on a coarse config-content fingerprint so an edited prompt,
//! model route, or tool config invalidates the cache instead of serving a
//! stale breakdown.

use std::collections::HashMap;
use std::sync::Mutex;

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};

use crate::agent::debug::prompt_size::{PromptSizeReport, SectionSize, ToolSize};
use crate::agent::debug::DumpPromptOptions;
use crate::config::Config;
use crate::core::runtime::DispatchCtx;
use crate::core::Outcome;

/// Default agent whose prompt is measured when the caller names none — the
/// one every main chat turn actually runs under.
const DEFAULT_AGENT_ID: &str = "orchestrator";

/// Params for `agent.context_breakdown`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ContextBreakdownParams {
    /// Agent to measure. Defaults to [`DEFAULT_AGENT_ID`].
    #[serde(default)]
    pub agent_id: Option<String>,
    /// When given, adds a `"history"` section sized from this thread's
    /// persisted usage (`threads.token_usage`'s last-turn input tokens).
    #[serde(default)]
    pub thread_id: Option<String>,
}

/// One labeled slice of the context-window pie: `system` (prompt prose),
/// `tools` (advertised schemas), or `history` (a thread's prior turns), plus
/// any per-section/per-tool breakdown collapsed into one flat list the UI
/// can render as a stacked bar without knowing the underlying shape.
#[derive(Debug, Clone, Serialize)]
pub struct ContextSection {
    pub label: String,
    pub bytes: usize,
    pub est_tokens: usize,
}

/// Response for `agent.context_breakdown`.
#[derive(Debug, Clone, Serialize)]
pub struct ContextBreakdownResponse {
    pub agent_id: String,
    pub model: String,
    pub sections: Vec<ContextSection>,
    pub tools_bytes: usize,
    pub total_est_tokens: usize,
    /// The resolved model's context window in tokens, when known (`0`
    /// otherwise — matches `ThreadTokenUsageResponse::context_window`'s
    /// unknown convention).
    pub context_window: u64,
}

fn est_tokens(bytes: usize) -> usize {
    bytes / crate::agent::debug::prompt_size::EST_BYTES_PER_TOKEN.max(1)
}

/// Coarse "did anything in config change" fingerprint. Correctness only
/// needs this to be sensitive enough that a real config edit invalidates the
/// cache; a false-positive miss (recomputing when nothing relevant changed)
/// just costs one extra expensive rebuild, never staleness.
fn config_fingerprint(config: &Config) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    match serde_json::to_string(config) {
        Ok(serialized) => serialized.hash(&mut hasher),
        // Unserializable config (should not happen) still needs a stable
        // fingerprint so the cache degrades to "always recompute" rather
        // than panicking.
        Err(_) => 0u8.hash(&mut hasher),
    }
    hasher.finish()
}

static REPORT_CACHE: Lazy<Mutex<HashMap<String, (u64, PromptSizeReport)>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// Builds (or reuses a cached) [`PromptSizeReport`] for `agent_id` under the
/// given config, invalidating the cache entry when the config fingerprint
/// changed since the last build.
async fn cached_report(agent_id: &str, config: &Config) -> Result<PromptSizeReport, String> {
    let fingerprint = config_fingerprint(config);
    if let Some((cached_fingerprint, report)) = REPORT_CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(agent_id)
    {
        if *cached_fingerprint == fingerprint {
            log::debug!(
                "[agent:context_breakdown] cache hit agent_id={agent_id} fingerprint={fingerprint}"
            );
            return Ok(report.clone());
        }
    }

    log::debug!(
        "[agent:context_breakdown] cache miss agent_id={agent_id} fingerprint={fingerprint}; \
         rebuilding prompt (fetches live Composio connections)"
    );
    let options = DumpPromptOptions {
        agent_id: agent_id.to_string(),
        workspace_dir_override: Some(config.workspace_dir.clone()),
        config_path_override: Some(config.config_path.clone()),
        model_override: None,
    };
    let report = PromptSizeReport::build(options)
        .await
        .map_err(|e| format!("failed to build prompt-size report for {agent_id}: {e}"))?;

    REPORT_CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(agent_id.to_string(), (fingerprint, report.clone()));
    Ok(report)
}

/// Adds a `"history"` section sized from the context a thread's last turn
/// ended with (`threads.token_usage`'s `last_turn_context_tokens`, one
/// request, not the turn's summed spend), when `thread_id` is given and that thread has
/// recorded usage. Silent no-op (no section added) on any lookup failure —
/// a context breakdown must never fail just because the optional history
/// enrichment couldn't be computed.
fn history_tokens(context_tokens: usize, fixed_prompt_tokens: usize) -> usize {
    context_tokens.saturating_sub(fixed_prompt_tokens)
}

async fn history_section(
    ctx: &DispatchCtx,
    thread_id: &str,
    fixed_prompt_tokens: usize,
) -> Option<ContextSection> {
    let outcome = crate::threads::ops::token_usage(
        ctx,
        crate::threads::ops::ThreadTokenUsageRequest {
            thread_id: thread_id.to_string(),
        },
    )
    .await
    .ok()?;
    let usage = outcome.value.data?;
    if !usage.has_usage {
        return None;
    }
    // Tokens, not bytes, is what `token_usage` actually recorded — reverse
    // the module's own byte-per-token estimate so `bytes` stays a consistent
    // (if approximate) unit across every section in the response.
    let est = history_tokens(usage.last_turn_context_tokens as usize, fixed_prompt_tokens);
    let bytes = est.saturating_mul(crate::agent::debug::prompt_size::EST_BYTES_PER_TOKEN);
    Some(ContextSection {
        label: "history".to_string(),
        bytes,
        est_tokens: est,
    })
}

fn section_from_prompt(section: &SectionSize) -> ContextSection {
    ContextSection {
        label: section.heading.clone(),
        bytes: section.bytes,
        est_tokens: est_tokens(section.bytes),
    }
}

fn tools_section(tools: &[ToolSize]) -> ContextSection {
    let bytes: usize = tools.iter().map(|t| t.bytes).sum();
    ContextSection {
        label: "tools".to_string(),
        bytes,
        est_tokens: est_tokens(bytes),
    }
}

/// Builds the `agent.context_breakdown` response: the agent's rendered
/// prompt sections, one rolled-up `tools` section, and (when `thread_id` is
/// given) a `history` section.
pub async fn context_breakdown(
    ctx: &DispatchCtx,
    params: ContextBreakdownParams,
) -> Result<Outcome<ContextBreakdownResponse>, String> {
    let config = ctx.load_config().await?;
    let agent_id = params
        .agent_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_AGENT_ID)
        .to_string();

    let report = cached_report(&agent_id, &config).await?;

    let mut sections: Vec<ContextSection> =
        report.sections.iter().map(section_from_prompt).collect();
    sections.push(tools_section(&report.tools));
    let fixed_prompt_tokens = sections.iter().map(|section| section.est_tokens).sum();

    if let Some(thread_id) = params
        .thread_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if let Some(history) = history_section(ctx, thread_id, fixed_prompt_tokens).await {
            sections.push(history);
        }
    }

    let total_est_tokens: usize = sections.iter().map(|s| s.est_tokens).sum();
    let provider = crate::inference::provider::provider_for_role("chat", &config);
    let context_window = crate::inference::model_context::context_window_for_route(
        &provider,
        &report.model,
        &config,
    )
    .unwrap_or(0);

    let response = ContextBreakdownResponse {
        agent_id: report.agent.clone(),
        model: report.model.clone(),
        sections,
        tools_bytes: report.tool_bytes,
        total_est_tokens,
        context_window,
    };
    Ok(Outcome::new(response, Vec::new()))
}

#[cfg(test)]
#[path = "context_breakdown_tests.rs"]
mod tests;
