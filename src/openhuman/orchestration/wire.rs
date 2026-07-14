//! Sanitized wire DTOs for the hosted orchestration brain.
//!
//! # Security choke point
//!
//! This file is the trust-boundary allowlist between the device and the backend.
//! Every field that crosses to `POST /orchestration/v1/events` is constructed
//! **explicitly** here — there is no `#[derive(Serialize)]` over an internal
//! state struct that could leak a newly-added field. Signal key material,
//! credentials, local filesystem paths, and workspace locations are NEVER part
//! of these structs, and the golden key-set test below asserts the exact JSON
//! shape so a future field addition fails loudly in review.

use serde::Serialize;
use serde_json::Value;

/// Wire-protocol version this client speaks. Must fall within the backend's
/// advertised `[min, max]`; a mismatch yields `409 ORCH_PROTOCOL_MISMATCH`.
pub const ORCH_WIRE_PROTOCOL: u8 = 1;

/// The inner `event` object of an ingest upload. Field-for-field the backend's
/// `orchestrationEventSchema`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OrchestrationEventWire {
    /// Store-assigned monotonic per-session ordinal (the idempotency key).
    pub seq: i64,
    /// One of `user` | `assistant` | `system` (clamped on build).
    pub role: String,
    /// The event's originator — for an inbound DM, the counterpart agent id.
    pub sender: String,
    /// Decrypted plaintext body. Never key material or a local path.
    pub body: String,
    /// Client event timestamp, epoch milliseconds.
    pub ts: i64,
    /// Event kind (`dm`, a v2 harness `event.kind`, …). Free-form, ≤64 chars.
    pub kind: String,
}

/// The full `POST /orchestration/v1/events` request body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrchestrationEventEnvelopeWire {
    pub protocol: u8,
    /// The paired agent this session is with. Not a credential — a public handle.
    pub counterpart_agent_id: String,
    pub session_id: String,
    pub event: OrchestrationEventWire,
}

/// Clamp an arbitrary role string to the backend's accepted enum. Unknown roles
/// (or empty) default to `user` so a malformed harness role never trips a 400.
fn sanitize_role(role: &str) -> String {
    match role {
        "user" | "assistant" | "system" => role.to_string(),
        _ => "user".to_string(),
    }
}

impl OrchestrationEventEnvelopeWire {
    /// Build a sanitized envelope from primitive fields. Constructing from
    /// primitives (rather than an internal state struct) is deliberate: it keeps
    /// the crossed field set an explicit, auditable allowlist.
    #[allow(clippy::too_many_arguments)]
    pub fn build(
        counterpart_agent_id: &str,
        session_id: &str,
        seq: i64,
        role: &str,
        sender: &str,
        body: &str,
        ts: i64,
        kind: &str,
    ) -> Self {
        Self {
            protocol: ORCH_WIRE_PROTOCOL,
            counterpart_agent_id: counterpart_agent_id.to_string(),
            session_id: session_id.to_string(),
            event: OrchestrationEventWire {
                seq: seq.max(0),
                role: sanitize_role(role),
                sender: sender.to_string(),
                body: body.to_string(),
                ts: ts.max(0),
                kind: if kind.is_empty() {
                    "message".to_string()
                } else {
                    kind.to_string()
                },
            },
        }
    }

    /// Serialize to a JSON value for the HTTP body.
    pub fn to_value(&self) -> Value {
        // Infallible for this closed struct of primitives.
        serde_json::to_value(self).expect("orchestration wire envelope serializes")
    }
}

/// Parse an RFC3339 timestamp into epoch milliseconds. Pure; returns `None` on
/// a malformed input so the caller can fall back (e.g. to the ingest clock).
pub fn parse_ts_ms(ts: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(ts)
        .ok()
        .map(|dt| dt.timestamp_millis())
}

/// One world-state-diff entry crossing to `POST /orchestration/v1/world-diff`.
/// `note` is a short derived observation (never a local path or key material).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorldDiffEntryWire {
    pub seq: i64,
    pub note: String,
    pub ts: i64,
}

impl WorldDiffEntryWire {
    pub fn build(seq: i64, note: &str, ts: i64) -> Self {
        Self {
            seq: seq.max(0),
            note: note.to_string(),
            ts: ts.max(0),
        }
    }
}

/// The full `POST /orchestration/v1/world-diff` request body. Same allowlist
/// discipline as the event envelope — only these fields are constructed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorldDiffBatchWire {
    pub protocol: u8,
    pub session_id: String,
    pub entries: Vec<WorldDiffEntryWire>,
}

impl WorldDiffBatchWire {
    pub fn build(session_id: &str, entries: Vec<WorldDiffEntryWire>) -> Self {
        Self {
            protocol: ORCH_WIRE_PROTOCOL,
            session_id: session_id.to_string(),
            entries,
        }
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).expect("orchestration world-diff batch serializes")
    }
}

// The wire allowlist is guarded by the golden key-set test in
// `tests/orchestration_shadow_push_e2e.rs` (integration crate): the root
// crate's `cfg(test)` build is currently blocked by unrelated stale test
// modules at this checkout, so the security-critical assertions live where they
// can actually run — against the compiled lib.
