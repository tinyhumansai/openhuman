//! Structured decision emitted by the `trigger_triage` agent, plus a
//! deliberately-tolerant parser that accepts whatever shape a small
//! local model is likely to produce.
//!
//! The contract is described in
//! `src/openhuman/agent_registry/agents/trigger_triage/prompt.md` — the triage
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
    /// [`crate::core::event_bus::DomainEvent::TriggerEvaluated::decision`]
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
/// Algorithm (keep in sync with the prompt's output contract):
///
/// 1. Try to extract the **last** fenced ```json block — small models
///    often add commentary *after* the JSON and we want the JSON.
/// 2. If no fence, brace-match the **last** balanced `{ … }` object in
///    the text. This handles "Here's my decision: { … }" and
///    "{ … } (hope that helps)".
/// 3. Strip trailing commas before the parse (`,}` → `}`, `,]` → `]`).
/// 4. Parse as JSON, lowercasing the `action` string in flight.
/// 5. Reject if React / Escalate but `target_agent`/`prompt` missing.
pub fn parse_triage_decision(llm_text: &str) -> Result<TriageDecision, ParseError> {
    let slice = extract_json_slice(llm_text).ok_or(ParseError::NoJsonObject)?;
    let cleaned = strip_trailing_commas(&slice);
    let normalized = lowercase_action_value(&cleaned);
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

// ─────────────────────────────────────────────────────────────────────────────
// Extraction helpers — all private, all exhaustively unit-tested below.
// ─────────────────────────────────────────────────────────────────────────────

/// Return the content of the **last** JSON object in `text`, preferring
/// fenced blocks over raw braces so we don't accidentally pick up a
/// half-written object inside a code fence's preamble.
fn extract_json_slice(text: &str) -> Option<String> {
    if let Some(fenced) = last_fenced_json_block(text) {
        return Some(fenced);
    }
    last_balanced_brace_object(text)
}

/// Find the last ```json … ``` fenced block in `text`, if any.
/// Accepts `` ```json ``, `` ```JSON ``, and plain `` ``` `` fences
/// since small models are inconsistent about the language tag.
fn last_fenced_json_block(text: &str) -> Option<String> {
    // Walk fence starts from the end so we naturally find the last one.
    let mut last: Option<String> = None;
    let mut search_from = 0;
    while let Some(rel) = text[search_from..].find("```") {
        let start = search_from + rel + 3;
        // Skip an optional language tag on the same line.
        let body_start = match text[start..].find('\n') {
            Some(nl) => {
                let tag = &text[start..start + nl];
                // Accept "json", "JSON", or empty tags. If the tag is
                // something else (e.g. "python") we still try to parse
                // the block — small models mislabel fences all the
                // time and the content may still be JSON.
                let _ = tag;
                start + nl + 1
            }
            None => return last,
        };
        let close = text[body_start..].find("```")?;
        let body = &text[body_start..body_start + close];
        last = Some(body.trim().to_string());
        search_from = body_start + close + 3;
    }
    last
}

/// Brace-match the last balanced `{ … }` object in `text`, ignoring
/// braces inside string literals. Returns the substring including the
/// outer braces. O(n) single pass.
fn last_balanced_brace_object(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut best: Option<(usize, usize)> = None;
    let mut depth = 0usize;
    let mut start: Option<usize> = None;
    let mut in_string = false;
    let mut escape = false;
    for (i, &b) in bytes.iter().enumerate() {
        if in_string {
            if escape {
                escape = false;
            } else if b == b'\\' {
                escape = true;
            } else if b == b'"' {
                in_string = false;
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'{' => {
                if depth == 0 {
                    start = Some(i);
                }
                depth += 1;
            }
            b'}' => {
                if depth > 0 {
                    depth -= 1;
                    if depth == 0 {
                        if let Some(s) = start.take() {
                            best = Some((s, i + 1));
                        }
                    }
                }
            }
            _ => {}
        }
    }
    best.map(|(s, e)| text[s..e].to_string())
}

/// Strip trailing commas before closing `}` / `]` — a very common
/// small-model mistake that otherwise trips `serde_json`.
fn strip_trailing_commas(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut in_string = false;
    let mut escape = false;
    let bytes = src.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if in_string {
            out.push(b as char);
            if escape {
                escape = false;
            } else if b == b'\\' {
                escape = true;
            } else if b == b'"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        if b == b'"' {
            in_string = true;
            out.push('"');
            i += 1;
            continue;
        }
        if b == b',' {
            // Look ahead past whitespace for the next non-ws char.
            let mut j = i + 1;
            while j < bytes.len() && (bytes[j] as char).is_whitespace() {
                j += 1;
            }
            if j < bytes.len() && (bytes[j] == b'}' || bytes[j] == b']') {
                // Drop the comma; continue from the whitespace so the
                // preserved indentation lands in `out` naturally.
                i += 1;
                continue;
            }
        }
        out.push(b as char);
        i += 1;
    }
    out
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
mod tests {
    use super::*;

    // ── extract / cleanup helpers ───────────────────────────────────────

    #[test]
    fn fenced_block_is_preferred_over_raw_braces() {
        let text = "preamble { \"other\": 1 } middle\n```json\n{\n  \"action\": \"drop\",\n  \"reason\": \"test\"\n}\n```\ntrailing notes";
        let slice = extract_json_slice(text).unwrap();
        assert!(slice.contains("\"action\""));
        assert!(slice.contains("\"reason\": \"test\""));
        assert!(!slice.contains("middle"));
    }

    #[test]
    fn bare_brace_object_is_extracted_when_no_fence() {
        let text = "Here is my verdict: { \"action\": \"drop\", \"reason\": \"test\" } — thanks!";
        let slice = extract_json_slice(text).unwrap();
        assert!(slice.contains("\"action\""));
    }

    #[test]
    fn last_of_multiple_braces_wins() {
        let text = "{ \"action\": \"escalate\", \"reason\": \"first\" } and then { \"action\": \"drop\", \"reason\": \"second\" }";
        let slice = extract_json_slice(text).unwrap();
        assert!(slice.contains("\"second\""));
        assert!(!slice.contains("\"first\""));
    }

    #[test]
    fn brace_inside_string_does_not_break_matching() {
        let text = "{ \"action\": \"drop\", \"reason\": \"has } and { chars\" }";
        let slice = extract_json_slice(text).unwrap();
        assert!(slice.contains("has } and { chars"));
    }

    #[test]
    fn trailing_commas_are_stripped() {
        let src = "{ \"a\": 1, \"b\": [1, 2,], }";
        assert_eq!(strip_trailing_commas(src), "{ \"a\": 1, \"b\": [1, 2] }");
    }

    #[test]
    fn trailing_comma_inside_string_is_left_alone() {
        let src = "{ \"reason\": \"a, b, c,\" }";
        assert_eq!(strip_trailing_commas(src), src);
    }

    #[test]
    fn action_value_is_lowercased() {
        let src = "{\"action\": \"Drop\", \"reason\": \"x\"}";
        let out = lowercase_action_value(src);
        assert!(out.contains("\"action\": \"drop\""));
    }

    #[test]
    fn other_string_values_are_not_lowercased() {
        let src = "{\"action\": \"DROP\", \"reason\": \"X Y Z\"}";
        let out = lowercase_action_value(src);
        assert!(out.contains("\"action\": \"drop\""));
        assert!(out.contains("\"reason\": \"X Y Z\""));
    }

    // ── full parse_triage_decision ──────────────────────────────────────

    #[test]
    fn parses_clean_fenced_drop() {
        let reply = "Here's my verdict:\n```json\n{\"action\":\"drop\",\"reason\":\"duplicate event\"}\n```\n";
        let d = parse_triage_decision(reply).unwrap();
        assert_eq!(d.action, TriageAction::Drop);
        assert_eq!(d.reason, "duplicate event");
        assert!(d.target_agent.is_none());
        assert!(d.prompt.is_none());
    }

    #[test]
    fn parses_unfenced_json_with_prose_before() {
        let reply = "I think this one needs human attention.\n\n{\"action\":\"escalate\",\"target_agent\":\"orchestrator\",\"prompt\":\"read the email and draft a reply\",\"reason\":\"complex request\"}";
        let d = parse_triage_decision(reply).unwrap();
        assert_eq!(d.action, TriageAction::Escalate);
        assert_eq!(d.target_agent.as_deref(), Some("orchestrator"));
        assert_eq!(
            d.prompt.as_deref(),
            Some("read the email and draft a reply")
        );
    }

    #[test]
    fn parses_react_with_trailing_comma() {
        let reply = "```\n{\n  \"action\": \"react\",\n  \"target_agent\": \"trigger_reactor\",\n  \"prompt\": \"send ack\",\n  \"reason\": \"one-step ack needed\",\n}\n```";
        let d = parse_triage_decision(reply).unwrap();
        assert_eq!(d.action, TriageAction::React);
        assert_eq!(d.target_agent.as_deref(), Some("trigger_reactor"));
    }

    #[test]
    fn parses_uppercase_action_field() {
        let reply = "{\"action\":\"DROP\",\"reason\":\"noise\"}";
        let d = parse_triage_decision(reply).unwrap();
        assert_eq!(d.action, TriageAction::Drop);
    }

    #[test]
    fn rejects_escalate_without_target_agent() {
        let reply = "{\"action\":\"escalate\",\"reason\":\"complex\"}";
        let err = parse_triage_decision(reply).unwrap_err();
        assert!(matches!(
            err,
            ParseError::MissingTarget { action: "escalate" }
        ));
    }

    #[test]
    fn rejects_react_without_prompt() {
        let reply = "{\"action\":\"react\",\"target_agent\":\"trigger_reactor\",\"reason\":\"x\"}";
        let err = parse_triage_decision(reply).unwrap_err();
        assert!(matches!(err, ParseError::MissingTarget { action: "react" }));
    }

    #[test]
    fn rejects_reply_with_no_json_at_all() {
        let reply = "I don't feel like answering today";
        let err = parse_triage_decision(reply).unwrap_err();
        assert!(matches!(err, ParseError::NoJsonObject));
    }

    #[test]
    fn rejects_non_parseable_json() {
        let reply = "{\"action\": not_a_string}";
        let err = parse_triage_decision(reply).unwrap_err();
        assert!(matches!(err, ParseError::InvalidJson(_)));
    }

    #[test]
    fn prefers_last_fenced_block() {
        let reply = "```json\n{\"action\":\"escalate\",\"target_agent\":\"orchestrator\",\"prompt\":\"first\",\"reason\":\"a\"}\n```\nactually scratch that:\n```json\n{\"action\":\"drop\",\"reason\":\"never mind\"}\n```";
        let d = parse_triage_decision(reply).unwrap();
        assert_eq!(d.action, TriageAction::Drop);
        assert_eq!(d.reason, "never mind");
    }
}
