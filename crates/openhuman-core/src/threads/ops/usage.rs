//! Aggregated token/cost usage for a thread, re-audited at current pricing.

use super::support::{counts, envelope, transcript_store};
use crate::core::runtime::DispatchCtx;
use crate::core::Outcome;
use crate::threads::ApiEnvelope;
use tinyagents_session::transcript::spend::thread_spend_in;

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
    /// The thread's cost, or `null` when some turn's cost is not known (no
    /// recorded charge and no catalogued price). Never a guessed rate.
    pub cost_usd: Option<f64>,
    /// `charged`, `estimated` (some turn priced from list rates) or `unknown`.
    pub cost_source: tinyagents_harness::cost::CostSource,
    pub turn_count: usize,
    /// Spend of the most recent turn, the orchestrator's own: every model call
    /// of that turn summed, so a long tool loop reports many times its context.
    pub last_turn_input_tokens: u64,
    pub last_turn_output_tokens: u64,
    /// Tokens the orchestrator's context held after the most recent turn's
    /// final model call — the numerator for the context-window gauge. One
    /// request, not a sum, and excluding sub-agents: each child runs in its own
    /// context window, so folding them in let the gauge exceed 100% (#4271).
    pub last_turn_context_tokens: u64,
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
    /// `null` when this archetype's cost is not known.
    pub cost_usd: Option<f64>,
    pub runs: usize,
}

/// Total a thread's persisted token/cost usage across its root transcripts
/// and their sub-agents, read from the store the request's tenant wrote them
/// to.
pub async fn token_usage(
    ctx: &DispatchCtx,
    request: ThreadTokenUsageRequest,
) -> Result<Outcome<ApiEnvelope<ThreadTokenUsageResponse>>, String> {
    // Resolved here, in the request's scope; the read itself is blocking.
    let locator = transcript_store(ctx, &request.thread_id).await?.locator();
    let thread_id = request.thread_id.clone();
    let spend = tokio::task::spawn_blocking(move || thread_spend_in(locator.as_ref(), &thread_id))
        .await
        .map_err(|e| format!("thread usage task: {e}"))?;

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

    let orchestrator_cost = recorded_cost(&spend.root.cost_split, root_model.as_deref());

    // Sub-agent archetypes. Older sub-agent transcripts didn't persist a
    // model on their messages; their unpriced turns are priced with the
    // thread's (root) model, sub-agents usually running on the parent's tier.
    let mut subagents = Vec::with_capacity(spend.subagents.len());
    let (mut sub_in, mut sub_out, mut sub_cached) = (0u64, 0u64, 0u64);
    let mut cost = orchestrator_cost;
    for (agent_id, (child, runs)) in &spend.subagents {
        let sub_model = child.model.as_deref().or(root_model.as_deref());
        let child_cost = recorded_cost(&child.cost_split, sub_model);
        sub_in = sub_in.saturating_add(child.input_tokens);
        sub_out = sub_out.saturating_add(child.output_tokens);
        sub_cached = sub_cached.saturating_add(child.cached_input_tokens);
        cost.merge(child_cost);
        subagents.push(SubagentUsageDto {
            agent_id: agent_id.clone(),
            input_tokens: child.input_tokens,
            output_tokens: child.output_tokens,
            cost_usd: child_cost.usd(),
            runs: *runs,
        });
    }

    // Top-level totals = orchestrator + all sub-agents, each counted once
    // because each transcript recorded only its own spend.
    let input_tokens = spend.root.input_tokens.saturating_add(sub_in);
    let output_tokens = spend.root.output_tokens.saturating_add(sub_out);
    let cached_input_tokens = spend.root.cached_input_tokens.saturating_add(sub_cached);
    let cost_usd = cost.usd();
    tracing::debug!(
        thread_id = %request.thread_id,
        ?cost_usd,
        cost_source = ?cost.source,
        unpriced_turns = spend.root.cost_split.unpriced_turns,
        "[threads] token_usage cost (recorded charges; unpriced turns re-priced from the catalog or left unknown)"
    );
    // A thread whose transcripts exist but recorded no spend must not claim
    // usage: the UI replaces its live bucket with this payload.
    let has_usage = input_tokens > 0
        || output_tokens > 0
        || cached_input_tokens > 0
        || cost_usd.is_some_and(|usd| usd > 0.0);

    let response = ThreadTokenUsageResponse {
        thread_id: request.thread_id.clone(),
        input_tokens,
        output_tokens,
        cached_input_tokens,
        cost_usd,
        cost_source: cost.source,
        turn_count: spend.root.turns,
        last_turn_input_tokens: spend.root.last_input_tokens,
        last_turn_output_tokens: spend.root.last_output_tokens,
        last_turn_context_tokens: spend.root.last_context_tokens,
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
        cost_usd: Some(0.0),
        cost_source: tinyagents_harness::cost::CostSource::Charged,
        turn_count: 0,
        last_turn_input_tokens: 0,
        last_turn_output_tokens: 0,
        last_turn_context_tokens: 0,
        context_window: 0,
        model: None,
        updated: None,
        has_usage: false,
        subagents: Vec::new(),
    }
}

/// A transcript's cost as the thread usage reports it.
///
/// Turns that recorded their cost's source are summed as recorded: a provider
/// charge is never re-priced. Turns without a usable source (written before
/// the field, or unknown at the time) are priced from the vendor catalog when
/// `model` has a list price, else the total is unknown. Nothing falls back to
/// a default rate.
fn recorded_cost(
    split: &tinyagents_session::transcript::spend::CostSplit,
    model: Option<&str>,
) -> tinyagents_harness::cost::CostTally {
    use tinyagents_harness::cost::{CallCost, CostSource, CostTally};
    use tinyagents_session::transcript::UsageCostSource;

    let mut cost = CostTally {
        known_usd: split.priced_cost_usd,
        source: match split.priced_source {
            Some(UsageCostSource::Estimated) => CostSource::Estimated,
            Some(UsageCostSource::Unknown) => CostSource::Unknown,
            Some(UsageCostSource::Charged) | None => CostSource::Charged,
        },
    };
    if split.unpriced_turns > 0 {
        let estimate = model.and_then(|model| {
            crate::agent::cost::estimate_call_cost_usd(
                model,
                &crate::inference::provider::BilledUsage::from_counts(
                    split.unpriced_input_tokens,
                    split.unpriced_output_tokens,
                )
                .with_cached_input_tokens(split.unpriced_cached_input_tokens),
            )
        });
        cost.add(match estimate {
            Some(usd) => CallCost::Estimated(usd),
            None => CallCost::Unknown,
        });
    }
    cost
}

#[cfg(test)]
#[path = "usage_tests.rs"]
mod tests;
