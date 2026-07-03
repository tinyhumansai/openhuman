//! Shared state threaded through the orchestration wake graph (stage 4).
//!
//! `OrchestrationState` is the spec's single `StateGraph` state object: one
//! value flows through the whole wake path (normalize → frontend → execute →
//! frontend → send_dm → context_guard → END) and is checkpointed at every
//! super-step boundary by [`SqlRunLedgerCheckpointer`](crate::openhuman::tinyagents::SqlRunLedgerCheckpointer)
//! under the thread id `orchestration:<session_id>`.
//!
//! Every field is serde-serializable so a mid-cycle crash can resume from the
//! last persisted boundary with an identical state. Fields the later stages own
//! (`compressed_history`, `world_state_diff`, `subconscious_steering`) are
//! carried here now so the checkpoint schema is stable — stages 5/6 fill them.

use serde::{Deserialize, Serialize};

use super::super::types::OrchestrationMessage;

/// One 20:1-compressed history entry (stage 5 fills these via the compress
/// node). Carried in state now so the checkpoint schema does not change shape
/// when stage 5 lands.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompressedEntry {
    /// The compressed summary text.
    pub summary: String,
    /// How many raw messages this entry folded (drives the 20:1 budget check).
    pub covered_messages: u32,
}

/// A single append-only world-state-diff entry (stage 5 fills these via the
/// world_diff node). The timeline is append-only from genesis — never wiped per
/// cycle (global invariant).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorldDiffEntry {
    /// Monotonic sequence within the diff timeline.
    pub seq: u64,
    /// Human-readable mutation note.
    pub note: String,
}

/// The append-only world-state diff carried through the cycle. Stage 5 appends
/// one entry per execution cycle; this stage keeps it empty.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorldDiff {
    pub entries: Vec<WorldDiffEntry>,
}

/// The single state object for one orchestration wake cycle.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OrchestrationState {
    /// The harness session id this cycle is waking for (`"master"` for a peer's
    /// Master window).
    pub session_id: String,
    /// Stable id for this wake cycle. Derived deterministically from
    /// `session_id` + the latest message seq so a resumed run reuses it and the
    /// compressed-history / world-diff store writes stay idempotent.
    pub cycle_id: String,
    /// The tiny.place `@handle` of the counterpart the reply DM goes back to.
    pub counterpart_agent_id: String,
    /// Windowed recent messages the `normalize` node folded in from the store.
    pub messages: Vec<OrchestrationMessage>,

    /// Front-end pass-1 output: macro-instructions for the reasoning core.
    pub agent_instructions: Option<String>,
    /// Reasoning-core output: the answer the front end compiles into a channel
    /// reply on pass 2.
    pub agent_reply: Option<String>,
    /// Raw execution trace captured by the `execute` node (assistant text +
    /// tool/sub-agent activity) — the input the `compress` node condenses 20:1.
    pub execution_trace: String,
    /// Front-end pass-2 output: the finished text sent back over the DM channel.
    /// Its presence is the router's terminate predicate (spec §5).
    pub channel_response: Option<String>,
    /// Steering directive injected by the subconscious (read in stages 5/6).
    pub subconscious_steering: Option<String>,

    /// 20:1 compressed history (stage 5 fills).
    pub compressed_history: Vec<CompressedEntry>,
    /// Append-only world-state diff (stage 5 fills).
    pub world_state_diff: WorldDiff,
    /// Fraction of the model context window in use (0.0–1.0), set by the
    /// `context_guard` node before END.
    pub context_utilization: f32,

    /// Front-end pass counter — bumped each time the front-end node runs. Used
    /// for the loop-continuity backstop and `[orchestration]` pass logging.
    pub pass: u32,
    /// Set true by `send_dm` the instant the outbound DM is dispatched, so a
    /// resumed or re-entered cycle can never double-send.
    pub dm_sent: bool,
}

impl OrchestrationState {
    /// Seed a fresh cycle for `session_id`, replying to `counterpart_agent_id`,
    /// over the windowed `messages`.
    pub fn seed(
        session_id: impl Into<String>,
        counterpart_agent_id: impl Into<String>,
        messages: Vec<OrchestrationMessage>,
    ) -> Self {
        let session_id = session_id.into();
        // Deterministic cycle id: session + the latest seq in this window. A
        // resumed run over the same window recomputes the same id, so the
        // per-cycle store writes (compressed_history, world_diff) dedupe.
        let latest_seq = messages.iter().map(|m| m.seq).max().unwrap_or(0);
        let cycle_id = format!("{session_id}#{latest_seq}");
        Self {
            session_id,
            cycle_id,
            counterpart_agent_id: counterpart_agent_id.into(),
            messages,
            ..Self::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_round_trips_through_serde() {
        let mut s = OrchestrationState::seed("h1", "@peer", Vec::new());
        s.agent_instructions = Some("summarize the diff".into());
        s.agent_reply = Some("done".into());
        s.channel_response = Some("all set".into());
        s.compressed_history.push(CompressedEntry {
            summary: "s".into(),
            covered_messages: 20,
        });
        s.world_state_diff.entries.push(WorldDiffEntry {
            seq: 1,
            note: "genesis".into(),
        });
        s.context_utilization = 0.42;
        s.pass = 2;
        s.dm_sent = true;

        let json = serde_json::to_string(&s).expect("serialize");
        let back: OrchestrationState = serde_json::from_str(&json).expect("deserialize");

        // Identical state after a serialize → resume round-trip.
        assert_eq!(back.session_id, "h1");
        assert_eq!(back.counterpart_agent_id, "@peer");
        assert_eq!(
            back.agent_instructions.as_deref(),
            Some("summarize the diff")
        );
        assert_eq!(back.agent_reply.as_deref(), Some("done"));
        assert_eq!(back.channel_response.as_deref(), Some("all set"));
        assert_eq!(back.compressed_history.len(), 1);
        assert_eq!(back.compressed_history[0].covered_messages, 20);
        assert_eq!(back.world_state_diff.entries.len(), 1);
        assert!((back.context_utilization - 0.42).abs() < f32::EPSILON);
        assert_eq!(back.pass, 2);
        assert!(back.dm_sent);
    }
}
