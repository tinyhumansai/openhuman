//! The turn origin a triage dispatch runs under.
//!
//! [`apply_decision`](super::apply_decision) reaches the approval gate, and the
//! gate decides what an escalation may do from the [`AgentTurnOrigin`] scoped
//! around the call. `AGENT_TURN_ORIGIN` is a `tokio::task_local`, and every
//! triage caller either spawns or is itself a fresh entry point, so there is no
//! ambient origin to inherit — an unscoped call reads `Unknown` and the gate
//! fails closed (openhuman#5634). Propagation helpers cannot fix that: there is
//! nothing to propagate, so inheriting would stay `Unknown`.
//!
//! Which label a caller scopes is decided by **provenance of the payload**, not
//! by how the code got there, and the two answers live here so the distinction
//! is one grep rather than six judgement calls:
//!
//! - [`local_trigger_origin`] — the caller and the payload are both local.
//! - [`remote_trigger_origin`] — the payload arrived from outside and is
//!   attacker-influenceable.
//!
//! Deliberately *not* offered: a single blanket label. A webhook body and a
//! desktop notification are not the same trust proposition, and one label for
//! both would have to be the weaker of the two.

use crate::openhuman::agent::turn_origin::{AgentTurnOrigin, TrustedAutomationSource};

use super::envelope::TriggerEnvelope;

/// The origin for a triage dispatch the local machine initiated.
///
/// [`AgentTurnOrigin::Cli`] is a trust root: the gate allows without prompting
/// and persists no audit row. That is correct here and only here — these
/// callers are the desktop's own RPC surface acting on data the machine already
/// holds, so the escalation gets exactly the authority the caller already had.
/// A payload that came in over the network must not use this; see
/// [`remote_trigger_origin`].
#[must_use]
pub fn local_trigger_origin() -> AgentTurnOrigin {
    AgentTurnOrigin::Cli
}

/// The origin for a triage dispatch driven by a payload from outside.
///
/// `TrustedAutomation { source: Workflow { require_approval: true } }`: the
/// dispatch itself is legitimate automation, but the content steering it is
/// remote, so the gate must not auto-allow. That variant parks every
/// external-effect call, publishes `ApprovalRequested`, and writes the
/// `pending_approvals` audit row.
///
/// **This does not restore remote escalation.** With no surface able to decide
/// a park raised from a background trigger, these parks TTL-deny — the same
/// outcome as before, now reached with an audit trail and a visible pending
/// approval instead of silently as `Unknown`. The decider surface is
/// openhuman#5746.
///
/// Explicitly not [`AgentTurnOrigin::Cli`]: that would hand a full trust root,
/// with no audit row, to attacker-influenceable content — the posture
/// [`TrustedAutomationSource::SubconsciousTainted`] exists to prevent.
///
/// `job_id` is the envelope's kind slug and correlation id, which is what an
/// operator reading a pending row needs to find the trigger. Both are
/// system-generated (Composio's `metadata.uuid`, a webhook tunnel id, a task
/// source id) — never payload text, which the gate would otherwise surface at
/// `info` as `flow_id`.
#[must_use]
pub fn remote_trigger_origin(envelope: &TriggerEnvelope) -> AgentTurnOrigin {
    AgentTurnOrigin::TrustedAutomation {
        job_id: format!("{}:{}", envelope.source.slug(), envelope.external_id),
        source: TrustedAutomationSource::Workflow {
            require_approval: true,
        },
    }
}

#[cfg(test)]
#[path = "origin_tests.rs"]
mod tests;
