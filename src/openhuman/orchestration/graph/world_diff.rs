//! World-state-diff mechanics for the `world_diff` node (stage 5).
//!
//! Each cycle appends **one** entry to an append-only timeline (spec §4). The
//! entry's fields are derived from the terminal `OrchestrationState`; the
//! monotonic `seq` + store persistence live in the store
//! ([`super::super::store::append_world_diff`]) and the production runtime.
//!
//! Global invariant: the timeline is append-only from genesis — never rewritten.

use super::OrchestrationState;

/// A compact signature of what this cycle observed (inputs + whether it replied).
pub fn event_signature(state: &OrchestrationState) -> String {
    format!(
        "cycle={} messages={} reply={}",
        state.cycle_id,
        state.messages.len(),
        if state.agent_reply.is_some() {
            "yes"
        } else {
            "no"
        }
    )
}

/// A one-line description of the world mutation this cycle produced — the
/// reasoning reply, or a no-op marker when the cycle produced nothing.
pub fn world_mutation(state: &OrchestrationState) -> String {
    match state.agent_reply.as_deref() {
        Some(reply) if !reply.trim().is_empty() => {
            // Keep the timeline note compact — one line, bounded length.
            let first_line = reply.lines().next().unwrap_or(reply).trim();
            first_line.chars().take(200).collect()
        }
        _ => "(no reply)".to_string(),
    }
}

/// The delta payload: the compressed-history summary added this cycle, if any.
pub fn delta(state: &OrchestrationState) -> String {
    state
        .compressed_history
        .last()
        .map(|e| e.summary.clone())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_and_mutation_reflect_terminal_state() {
        let mut s = OrchestrationState::seed("h1", "@peer", Vec::new());
        assert!(event_signature(&s).contains("cycle=h1#0"));
        assert!(event_signature(&s).contains("reply=no"));
        assert_eq!(world_mutation(&s), "(no reply)");

        s.agent_reply = Some("shipped the fix\nand more detail".into());
        assert!(event_signature(&s).contains("reply=yes"));
        assert_eq!(world_mutation(&s), "shipped the fix", "one compact line");
    }
}
