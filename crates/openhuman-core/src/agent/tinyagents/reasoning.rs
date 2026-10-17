//! The user's reasoning ("thinking") choice for an agent turn, resolved into
//! the provider-neutral [`ReasoningConfig`] the harness attaches to every
//! model request (`RunPolicy::default_reasoning`).
//!
//! Precedence, highest first:
//!
//! 1. the thread's own choice, recorded by `channel_web_chat`'s
//!    `reasoning_effort` param ([`apply_requested_effort`]) — read at turn
//!    start, so changing it never evicts the thread's warm session;
//! 2. the turn model's own level, `runtime.reasoning_effort_by_model`;
//! 3. `runtime.reasoning_effort` in the session's effective config;
//! 4. `runtime.reasoning_enabled = false`, meaning "no reasoning".
//!
//! Nothing set means the provider keeps its own default.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;
use tinyinference_llm::model::{ReasoningConfig, ReasoningEffort};

use crate::config::Config;
use crate::core::runtime::DispatchCtx;

type ThreadEfforts = Mutex<HashMap<String, ReasoningEffort>>;

fn thread_efforts(ctx: &DispatchCtx) -> Arc<ThreadEfforts> {
    crate::core::runtime::slot_for::<ThreadEfforts>(ctx)
}

/// The effort a thread's user picked, if any.
pub(crate) fn thread_effort(ctx: &DispatchCtx, thread_id: &str) -> Option<ReasoningEffort> {
    thread_efforts(ctx).lock().get(thread_id).copied()
}

/// Records (`Some`) or clears (`None`) a thread's chosen effort.
pub(crate) fn set_thread_effort(
    ctx: &DispatchCtx,
    thread_id: &str,
    effort: Option<ReasoningEffort>,
) {
    let efforts = thread_efforts(ctx);
    let mut map = efforts.lock();
    let previous = match effort {
        Some(effort) => map.insert(thread_id.to_string(), effort),
        None => map.remove(thread_id),
    };
    if previous != effort {
        log::info!(
            "[agent][reasoning] thread reasoning effort changed thread_id={thread_id} {:?} -> {:?}",
            previous.map(ReasoningEffort::as_str),
            effort.map(ReasoningEffort::as_str)
        );
    }
}

/// Applies a chat request's `reasoning_effort` param to its thread.
///
/// Absent leaves the thread's choice as it was (older clients and the socket
/// path never send it); `""`/`default`/`auto` clears it back to config; any
/// other value must parse, otherwise the request is rejected so a typo'd
/// client cannot silently run at the wrong level.
pub(crate) fn apply_requested_effort(
    ctx: &DispatchCtx,
    thread_id: &str,
    raw: Option<&str>,
) -> Result<(), String> {
    let Some(raw) = raw else {
        return Ok(());
    };
    let trimmed = raw.trim();
    if trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("default")
        || trimmed.eq_ignore_ascii_case("auto")
    {
        set_thread_effort(ctx, thread_id, None);
        return Ok(());
    }
    let effort = parse_reasoning_effort(trimmed)
        .ok_or_else(|| format!("unknown reasoning_effort '{trimmed}'"))?;
    set_thread_effort(ctx, thread_id, Some(effort));
    Ok(())
}

/// The reasoning config for the turn `ctx` is about to run, or `None` for the
/// provider default.
///
/// Only a root turn (spawn depth 0) follows the user's thinking level; a
/// sub-agent runs at its provider default, since the user picked a level for
/// the conversation, not for every delegated helper. `config` is the session
/// config a hosted root turn carries (`OpenHumanHostBase::config`).
///
/// `max_output_tokens` is the turn's per-call output cap. When reasoning is
/// on and the turn is capped, the config also carries a thinking budget
/// ([`with_output_room`]) so the visible reply always has room left.
pub(crate) fn turn_reasoning_for(
    ctx: &crate::agent::tinyagents::host::OpenHumanRunContext,
    config: Option<&Config>,
    max_output_tokens: Option<u32>,
) -> Option<ReasoningConfig> {
    if ctx.spawn_depth > 0 {
        return None;
    }
    let reasoning = turn_reasoning(&ctx.dispatch_ctx(), ctx.thread_id.as_deref(), config)
        .map(|reasoning| with_output_room(reasoning, max_output_tokens));
    if let Some(reasoning) = reasoning.as_ref() {
        log::debug!(
            "[agent][reasoning] turn reasoning effort={:?} budget_tokens={:?} thread_id={:?}",
            reasoning.effort,
            reasoning.budget_tokens,
            ctx.thread_id
        );
    }
    reasoning
}

/// The reasoning config one agent turn should request: the thread's own
/// choice, else [`reasoning_for_config`].
pub(crate) fn turn_reasoning(
    dispatch: &DispatchCtx,
    thread_id: Option<&str>,
    config: Option<&Config>,
) -> Option<ReasoningConfig> {
    if let Some(effort) = thread_id.and_then(|thread_id| thread_effort(dispatch, thread_id)) {
        return Some(ReasoningConfig::effort(effort));
    }
    config.and_then(reasoning_for_config)
}

/// Share of a turn's output cap a reasoning model may spend thinking. The rest
/// is left for the visible reply — in an agent turn usually a tool call whose
/// arguments can be a whole file. Without a budget only an effort level is
/// sent, and a high-effort model can think through the entire cap and return
/// `finish_reason = length` with no tool call (#6951).
const REASONING_BUDGET_PERCENT: u32 = 55;

/// Smallest thinking budget worth sending. Anthropic models (reached through
/// OpenRouter or the managed backend, which pass `reasoning.max_tokens` on as
/// a thinking budget) reject budgets under 1024; below it the effort level
/// alone is sent.
const MIN_REASONING_BUDGET_TOKENS: u32 = 1024;

/// Adds a thinking budget of [`REASONING_BUDGET_PERCENT`] of the turn's output
/// cap to an *enabled* reasoning config that has none.
///
/// Left unchanged: reasoning switched off (`effort = none`), no effort chosen
/// (a bare budget would turn reasoning on where the provider default is off),
/// an uncapped turn, a config that already names a budget, and a cap too small
/// for [`MIN_REASONING_BUDGET_TOKENS`]. Only OpenRouter and the managed
/// backend consume the budget, sending it as `reasoning.max_tokens`. Every
/// other route sends what it sent before: native Anthropic keeps adaptive
/// thinking with the effort (the budget applies there only with no effort),
/// and plain OpenAI-compatible endpoints and the Responses API drop it.
pub(crate) fn with_output_room(
    mut reasoning: ReasoningConfig,
    max_output_tokens: Option<u32>,
) -> ReasoningConfig {
    let enabled = matches!(reasoning.effort, Some(effort) if effort != ReasoningEffort::None);
    if !enabled || reasoning.budget_tokens.is_some() {
        return reasoning;
    }
    let Some(cap) = max_output_tokens else {
        return reasoning;
    };
    let budget = (u64::from(cap) * u64::from(REASONING_BUDGET_PERCENT) / 100) as u32;
    if budget < MIN_REASONING_BUDGET_TOKENS {
        log::debug!(
            "[agent][reasoning] output cap {cap} too small for a thinking budget; effort only"
        );
        return reasoning;
    }
    reasoning.budget_tokens = Some(budget);
    reasoning
}

/// Parses a user-facing effort name. Accepts the wire tokens plus the aliases
/// `off`/`disabled` (none) and `max`/`maximum` (xhigh), case-insensitively.
pub(crate) fn parse_reasoning_effort(raw: &str) -> Option<ReasoningEffort> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "none" | "off" | "disabled" => Some(ReasoningEffort::None),
        "minimal" => Some(ReasoningEffort::Minimal),
        "low" => Some(ReasoningEffort::Low),
        "medium" | "med" => Some(ReasoningEffort::Medium),
        "high" => Some(ReasoningEffort::High),
        "xhigh" | "max" | "maximum" => Some(ReasoningEffort::XHigh),
        _ => None,
    }
}

/// The reasoning config an agent turn on `config` should request, or `None`
/// to leave the provider default alone.
///
/// The turn's model's own level (`runtime.reasoning_effort_by_model`, keyed by
/// `default_model`, which a chat's `model_override` has already replaced in
/// the session config) outranks the global `runtime.reasoning_effort`.
pub(crate) fn reasoning_for_config(config: &Config) -> Option<ReasoningConfig> {
    let per_model = config
        .default_model
        .as_deref()
        .and_then(|model| config.runtime.reasoning_effort_by_model.get(model.trim()));
    if let Some(raw) = per_model
        .or(config.runtime.reasoning_effort.as_ref())
        .map(|raw| raw.trim())
        .filter(|raw| !raw.is_empty())
    {
        return match parse_reasoning_effort(raw) {
            Some(effort) => Some(ReasoningConfig::effort(effort)),
            None => {
                log::warn!(
                    "[agent][reasoning] ignoring unknown runtime.reasoning_effort {raw:?}; \
                     using the provider default"
                );
                None
            }
        };
    }
    if config.runtime.reasoning_enabled == Some(false) {
        return Some(ReasoningConfig::effort(ReasoningEffort::None));
    }
    None
}

#[cfg(test)]
#[path = "reasoning_tests.rs"]
mod tests;
