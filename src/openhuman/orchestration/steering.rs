//! Subconscious steering directives (stage 6).
//!
//! The subconscious tick reviews the orchestration layer's compressed history +
//! cumulative world-state diff and emits a short, dense **steering directive**
//! that the reasoning `execute` node injects into its system prompt on later
//! cycles. This module holds the pure, testable pieces: the read type, the
//! synthesis prompt, and the structured-output parser. The store lives in
//! [`super::store`]; the tick that runs the LLM lives in the subconscious engine.

use serde::{Deserialize, Serialize};

/// Hard cap on directive length (~150–200 tokens). Enforced on parse.
pub const MAX_STEERING_CHARS: usize = 900;

/// Default lifetime of a directive, in reasoning cycles, when the model omits or
/// mis-formats the machine field.
pub const DEFAULT_EXPIRES_AFTER_CYCLES: u32 = 20;

/// A persisted steering directive (store read shape).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SteeringDirective {
    pub id: i64,
    pub text: String,
    pub created_at: String,
    pub expires_after_cycles: u32,
    /// The reasoning-cycle counter value at creation — expiry is measured
    /// against the live counter (`created_cycle + expires_after_cycles`).
    pub created_cycle: i64,
}

/// The parsed structured output of a steering synthesis turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedSteering {
    pub text: String,
    pub expires_after_cycles: u32,
}

/// Parse the synthesis model output. Expects a `STEERING_DIRECTIVE:` line (the
/// imperative directive) and an optional `expires_after_cycles:` machine field.
/// Returns `None` (idle — no directive this tick) when the model declines with
/// `NONE` / an empty directive, or when the contract is violated (no directive
/// line) — the caller retries once, then skips the tick.
pub fn parse_steering_output(raw: &str) -> Option<ParsedSteering> {
    let mut directive: Option<String> = None;
    let mut expires = DEFAULT_EXPIRES_AFTER_CYCLES;

    for line in raw.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("STEERING_DIRECTIVE:") {
            let d = rest.trim();
            if !d.is_empty() {
                directive = Some(d.to_string());
            }
        } else if let Some(rest) = t.strip_prefix("expires_after_cycles:") {
            if let Ok(n) = rest.trim().parse::<u32>() {
                if n > 0 {
                    expires = n;
                }
            }
        }
    }

    let text = directive?;
    if text.eq_ignore_ascii_case("none") || text.eq_ignore_ascii_case("no directive") {
        return None;
    }
    // Enforce the length cap deterministically (the prompt asks for ~150 tokens).
    let text = if text.chars().count() > MAX_STEERING_CHARS {
        text.chars().take(MAX_STEERING_CHARS).collect()
    } else {
        text
    };
    Some(ParsedSteering {
        text,
        expires_after_cycles: expires,
    })
}

/// True when the model explicitly declined with `STEERING_DIRECTIVE: NONE` — a
/// valid idle response (no retry), distinct from a contract violation (retry).
pub fn is_explicit_none(raw: &str) -> bool {
    raw.lines().any(|line| {
        line.trim()
            .strip_prefix("STEERING_DIRECTIVE:")
            .map(|d| {
                let d = d.trim();
                d.eq_ignore_ascii_case("none") || d.eq_ignore_ascii_case("no directive")
            })
            .unwrap_or(false)
    })
}

/// Build the steering-synthesis prompt from the unreviewed compressed-history
/// summaries and the cumulative world-diff mutations. The model reads macro
/// trends (spec §3.2: filter localized variance) and emits one directive.
pub fn build_steering_prompt(
    compressed_summaries: &[String],
    world_mutations: &[String],
) -> String {
    let mut p = String::with_capacity(2048);
    p.push_str(
        "You are the offline subconscious of an AI orchestrator. You never talk to anyone and \
         never take external actions. Your only job right now is to reflect on how the \
         orchestrator's world has been trending and emit ONE short steering directive that will \
         be injected into the reasoning core's prompt on future cycles.\n\n",
    );

    p.push_str("## Cumulative world-state diff (macro timeline — newest last)\n\n");
    if world_mutations.is_empty() {
        p.push_str("(empty)\n");
    } else {
        for (i, m) in world_mutations.iter().enumerate() {
            p.push_str(&format!("{}. {}\n", i + 1, m));
        }
    }
    p.push('\n');

    p.push_str("## Recent compressed execution history (unreviewed, oldest first)\n\n");
    if compressed_summaries.is_empty() {
        p.push_str("(none)\n");
    } else {
        for (i, s) in compressed_summaries.iter().enumerate() {
            p.push_str(&format!("--- entry {} ---\n{}\n", i + 1, s));
        }
    }
    p.push('\n');

    p.push_str(
        "## Output contract\n\n\
         Reflect on the MACRO trend, not one-off variance. If nothing meaningful has shifted, \
         reply exactly `STEERING_DIRECTIVE: NONE`. Otherwise emit at most ~150 tokens, imperative, \
         model-agnostic. Format EXACTLY:\n\n\
         STEERING_DIRECTIVE: <one dense imperative directive>\n\
         expires_after_cycles: <integer, default 20>\n\n\
         Examples:\n\
         STEERING_DIRECTIVE: The user has pivoted to shipping the billing migration; prioritize \
         correctness and rollback-safety over new features, and confirm destructive DB steps.\n\
         expires_after_cycles: 15\n\n\
         STEERING_DIRECTIVE: Repeated auth failures indicate a stale token; prefer re-checking \
         credentials before retrying downstream calls.\n\
         expires_after_cycles: 10\n\n\
         STEERING_DIRECTIVE: NONE\n",
    );
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_directive_and_expiry() {
        let raw = "STEERING_DIRECTIVE: prioritize the billing migration\nexpires_after_cycles: 15";
        let p = parse_steering_output(raw).expect("parsed");
        assert_eq!(p.text, "prioritize the billing migration");
        assert_eq!(p.expires_after_cycles, 15);
    }

    #[test]
    fn defaults_expiry_when_absent_or_invalid() {
        let p = parse_steering_output("STEERING_DIRECTIVE: do the thing").expect("parsed");
        assert_eq!(p.expires_after_cycles, DEFAULT_EXPIRES_AFTER_CYCLES);
        let p2 = parse_steering_output("STEERING_DIRECTIVE: x\nexpires_after_cycles: nope")
            .expect("parsed");
        assert_eq!(p2.expires_after_cycles, DEFAULT_EXPIRES_AFTER_CYCLES);
    }

    #[test]
    fn none_and_contract_violation_yield_no_directive() {
        assert!(parse_steering_output("STEERING_DIRECTIVE: NONE").is_none());
        assert!(parse_steering_output("STEERING_DIRECTIVE:   ").is_none());
        assert!(parse_steering_output("i did not follow the format").is_none());
    }

    #[test]
    fn over_long_directive_is_capped() {
        let long = "x".repeat(MAX_STEERING_CHARS + 500);
        let raw = format!("STEERING_DIRECTIVE: {long}");
        let p = parse_steering_output(&raw).expect("parsed");
        assert_eq!(p.text.chars().count(), MAX_STEERING_CHARS);
    }

    #[test]
    fn prompt_includes_both_sources_and_the_contract() {
        let p = build_steering_prompt(
            &["compressed summary A".to_string()],
            &["world moved to v2".to_string()],
        );
        assert!(p.contains("compressed summary A"));
        assert!(p.contains("world moved to v2"));
        assert!(p.contains("STEERING_DIRECTIVE:"));
    }
}
