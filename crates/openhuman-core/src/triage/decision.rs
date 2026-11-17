//! Structured decision emitted by the `trigger_triage` agent, plus host
//! action normalization and validation around TinyAgents' lenient parser.
//!
//! The contract is described in
//! `crates/openhuman-core/src/agent_registry/agents/trigger_triage/prompt.md` — the triage
//! agent must end its reply with a JSON object of the form:
//!
//! ```json
//! { "action":        "drop|acknowledge|react|escalate",
//!   "target_agent":  "trigger_reactor|orchestrator|null",
//!   "prompt":        "task for the target agent, or null",
//!   "reason":        "one-sentence justification" }
//! ```
//!
//! The triage agent runs on models as small as `gemma3:1b-it-qat`, which
//! routinely emit:
//!
//! - fenced `` ```json `` blocks with trailing commentary,
//! - bare JSON objects embedded in prose,
//! - trailing commas,
//! - `"action": "Drop"` (wrong case),
//!
//! so the parser is deliberately forgiving along each of those axes. On
//! parse failure the caller retries the whole turn on the remote
//! provider (see `evaluator.rs` — wired in commit 2).

use serde::Deserialize;
use thiserror::Error;
use tinyagents_harness::structured::parse_lenient_last_object;

/// The four outcomes the triage agent is allowed to choose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TriageAction {
    /// Noise / duplicate / spam / irrelevant — no downstream work.
    Drop,
    /// Log + persist a memory note; no agent is dispatched.
    Acknowledge,
    /// Narrow single-step side effect — hand off to `trigger_reactor`.
    React,
    /// Multi-step / multi-skill — hand off to `orchestrator`.
    Escalate,
}

impl TriageAction {
    /// Short stable string used in log prefixes and the
    /// [`crate::core::events::DomainEvent::TriggerEvaluated::decision`]
    /// field. Intentionally distinct from the `Debug` impl so we can
    /// change the enum representation without breaking dashboards.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Drop => "drop",
            Self::Acknowledge => "acknowledge",
            Self::React => "react",
            Self::Escalate => "escalate",
        }
    }

    /// Whether this action requires a `target_agent` and a `prompt`.
    /// Used by the parser to reject under-specified React / Escalate
    /// replies → caller falls back to a remote retry.
    pub fn requires_target(&self) -> bool {
        matches!(self, Self::React | Self::Escalate)
    }
}

/// Parsed classifier decision. Fields that are `None` on Drop /
/// Acknowledge are guaranteed to be `Some` on React / Escalate — the
/// parser enforces that invariant and returns
/// [`ParseError::MissingTarget`] otherwise.
#[derive(Debug, Clone, Deserialize)]
pub struct TriageDecision {
    pub action: TriageAction,
    /// Agent id to hand off to. Only meaningful when
    /// `action.requires_target()` returns `true`.
    #[serde(default)]
    pub target_agent: Option<String>,
    /// Prompt to pass to the target agent. Ditto.
    #[serde(default)]
    pub prompt: Option<String>,
    /// One-sentence justification, always present. Propagated into
    /// the `reason` field of `TriggerEscalationFailed` on downstream
    /// failures.
    pub reason: String,
}

/// Errors the parser returns when the classifier's reply doesn't match
/// the contract. Each variant is actionable for the caller: all of them
/// mean "retry this turn on the remote provider."
#[derive(Debug, Error)]
pub enum ParseError {
    #[error("classifier reply contained no JSON object")]
    NoJsonObject,
    #[error("failed to deserialize JSON object: {0}")]
    InvalidJson(#[source] serde_json::Error),
    #[error(
        "action `{action}` requires both `target_agent` and `prompt` \
         but at least one was missing or empty"
    )]
    MissingTarget { action: &'static str },
}

/// Parse the triage agent's raw reply text into a [`TriageDecision`].
///
/// TinyAgents selects and repairs the last complete JSON object. OpenHuman
/// keeps the triage-specific casing normalization and action-field validation:
/// React / Escalate must include a non-empty target and prompt.
pub fn parse_triage_decision(llm_text: &str) -> Result<TriageDecision, ParseError> {
    let (value, _) = parse_lenient_last_object(llm_text).ok_or(ParseError::NoJsonObject)?;
    let normalized = lowercase_action_value(&value.to_string());
    let decision: TriageDecision =
        serde_json::from_str(&normalized).map_err(ParseError::InvalidJson)?;

    if decision.action.requires_target() {
        let has_target = decision
            .target_agent
            .as_ref()
            .is_some_and(|s| !s.trim().is_empty());
        let has_prompt = decision
            .prompt
            .as_ref()
            .is_some_and(|s| !s.trim().is_empty());
        if !(has_target && has_prompt) {
            return Err(ParseError::MissingTarget {
                action: decision.action.as_str(),
            });
        }
    }

    Ok(decision)
}

/// Rewrite `"action": "Drop"` / `"action": "ESCALATE"` as
/// `"action": "drop"` / `"action": "escalate"` so serde's
/// `rename_all = "lowercase"` attribute accepts whatever casing the
/// model chose. Only touches the string value of the `action` key — a
/// regex-free, allocation-light single-pass rewrite.
fn lowercase_action_value(src: &str) -> String {
    // Find `"action"` key occurrences and lowercase the next string
    // literal. If no `"action"` key exists we return `src` unchanged
    // — the serde parse will fail with a useful error either way.
    let needle = "\"action\"";
    let Some(key_idx) = src.find(needle) else {
        return src.to_string();
    };
    let after_key = key_idx + needle.len();
    // Scan for ':' then the opening quote of the value string.
    let Some(colon_rel) = src[after_key..].find(':') else {
        return src.to_string();
    };
    let after_colon = after_key + colon_rel + 1;
    let Some(open_rel) = src[after_colon..].find('"') else {
        return src.to_string();
    };
    let value_start = after_colon + open_rel + 1;
    let Some(close_rel) = src[value_start..].find('"') else {
        return src.to_string();
    };
    let value_end = value_start + close_rel;
    let mut out = String::with_capacity(src.len());
    out.push_str(&src[..value_start]);
    out.push_str(&src[value_start..value_end].to_lowercase());
    out.push_str(&src[value_end..]);
    out
}

#[cfg(test)]
#[path = "decision_tests.rs"]
mod tests;
