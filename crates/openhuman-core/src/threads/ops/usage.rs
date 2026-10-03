//! Aggregated token/cost usage for a thread, re-audited at current pricing.

use super::support::{counts, envelope, workspace_dir};
use crate::core::Outcome;
use crate::threads::ApiEnvelope;
use tinyagents_session::transcript::spend::thread_spend;

/// Request for [`token_usage`]: the thread whose persisted usage to total.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ThreadTokenUsageRequest {
    pub thread_id: String,
}

/// Aggregated token/cost usage for one thread, read back from its persisted
/// session transcripts. Seeds the UI footer when the user selects a thread so
/// the totals reflect prior turns instead of starting at zero.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ThreadTokenUsageResponse {
    pub thread_id: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_input_tokens: u64,
    pub cost_usd: f64,
    pub turn_count: usize,
    /// Tokens of the most recent turn — numerator for the context-window gauge.
    /// The orchestrator's own, excluding sub-agents: each child runs in its own
    /// context window, so folding them in let the gauge exceed 100% (#4271).
    pub last_turn_input_tokens: u64,
    pub last_turn_output_tokens: u64,
    /// Context window (tokens) inferred from the last model; `0` when unknown.
    pub context_window: u64,
    pub model: Option<String>,
    pub updated: Option<String>,
    /// `false` when the thread has no persisted spend yet (all zeros). The UI
    /// uses this to decide whether to seed its live bucket at all, so a thread
    /// whose transcripts exist but recorded nothing must report `false` — the
    /// alternative overwrites a live in-progress bucket with zeros.
    pub has_usage: bool,
    /// Per-archetype sub-agent spend (re-audited at current pricing). The
    /// top-level totals already include this; it's broken out for the UI's
    /// per-agent footer rows.
    pub subagents: Vec<SubagentUsageDto>,
}

/// One sub-agent archetype's contribution within a thread.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SubagentUsageDto {
    pub agent_id: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
    pub runs: usize,
}

/// Total a thread's persisted token/cost usage across its root transcripts.
pub async fn token_usage(
    request: ThreadTokenUsageRequest,
) -> Result<Outcome<ApiEnvelope<ThreadTokenUsageResponse>>, String> {
    let dir = workspace_dir().await?;
    let spend = thread_spend(&dir, &request.thread_id);

    // Re-audit cost at CURRENT pricing rather than trusting the
    // `charged_amount_usd` persisted in the transcript: those values were
    // stamped at turn time and don't reflect later tier-pricing corrections.
    // Recompute from the persisted token counts using the last-known model's
    // rates; falls back to `fallback` only when the model is unknown.
    let audit_cost =
        |model: Option<&str>, input: u64, output: u64, cached: u64, fallback: f64| match model {
            Some(m) => crate::agent::cost::estimate_call_cost_usd(
                m,
                &crate::inference::provider::BilledUsage::from_counts(input, output)
                    .with_cached_input_tokens(cached),
            ),
            None => fallback,
        };

    if !spend.found_transcript {
        return Ok(envelope(
            empty_response(&request.thread_id),
            Some(counts([("has_usage", 0)])),
            None,
        ));
    }

    let root_model = spend.root.model.clone();
    let context_window = if spend.root.context_window > 0 {
        spend.root.context_window
    } else {
        root_model
            .as_deref()
            .and_then(crate::inference::model_context::context_window_for_model)
            .unwrap_or(0)
    };

    // Orchestrator (root) spend, re-audited.
    let orchestrator_cost = audit_cost(
        root_model.as_deref(),
        spend.root.input_tokens,
        spend.root.output_tokens,
        spend.root.cached_input_tokens,
        spend.root.cost_usd,
    );

    // Sub-agent archetypes, each re-audited with its own model. Older
    // sub-agent transcripts didn't persist a model on their messages, so
    // fall back to the thread's (root) model rather than pricing them at
    // $0 — sub-agents usually run on the same managed tier as the parent.
    let mut subagents = Vec::with_capacity(spend.subagents.len());
    let (mut sub_in, mut sub_out, mut sub_cached, mut sub_cost) = (0u64, 0u64, 0u64, 0.0);
    for (agent_id, (child, runs)) in &spend.subagents {
        let sub_model = child.model.as_deref().or(root_model.as_deref());
        let cost = audit_cost(
            sub_model,
            child.input_tokens,
            child.output_tokens,
            child.cached_input_tokens,
            child.cost_usd,
        );
        sub_in = sub_in.saturating_add(child.input_tokens);
        sub_out = sub_out.saturating_add(child.output_tokens);
        sub_cached = sub_cached.saturating_add(child.cached_input_tokens);
        sub_cost += cost;
        subagents.push(SubagentUsageDto {
            agent_id: agent_id.clone(),
            input_tokens: child.input_tokens,
            output_tokens: child.output_tokens,
            cost_usd: cost,
            runs: *runs,
        });
    }

    // Top-level totals = orchestrator + all sub-agents, each counted once
    // because each transcript recorded only its own spend.
    let input_tokens = spend.root.input_tokens.saturating_add(sub_in);
    let output_tokens = spend.root.output_tokens.saturating_add(sub_out);
    let cached_input_tokens = spend.root.cached_input_tokens.saturating_add(sub_cached);
    let cost_usd = orchestrator_cost + sub_cost;
    // A thread whose transcripts exist but recorded no spend must not claim
    // usage: the UI replaces its live bucket with this payload.
    let has_usage =
        input_tokens > 0 || output_tokens > 0 || cached_input_tokens > 0 || cost_usd > 0.0;

    let response = ThreadTokenUsageResponse {
        thread_id: request.thread_id.clone(),
        input_tokens,
        output_tokens,
        cached_input_tokens,
        cost_usd,
        turn_count: spend.root.turns,
        last_turn_input_tokens: spend.root.last_input_tokens,
        last_turn_output_tokens: spend.root.last_output_tokens,
        context_window,
        model: root_model,
        updated: spend.updated,
        has_usage,
        subagents,
    };

    Ok(envelope(
        response,
        Some(counts([("has_usage", usize::from(has_usage))])),
        None,
    ))
}

fn empty_response(thread_id: &str) -> ThreadTokenUsageResponse {
    ThreadTokenUsageResponse {
        thread_id: thread_id.to_string(),
        input_tokens: 0,
        output_tokens: 0,
        cached_input_tokens: 0,
        cost_usd: 0.0,
        turn_count: 0,
        last_turn_input_tokens: 0,
        last_turn_output_tokens: 0,
        context_window: 0,
        model: None,
        updated: None,
        has_usage: false,
        subagents: Vec::new(),
    }
}
