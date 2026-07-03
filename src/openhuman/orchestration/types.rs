//! Orchestration domain types.
//!
//! [`SessionEnvelopeV1`] is a hand-written mirror of the tiny.place TypeScript
//! SDK schema `sdk/typescript/src/types/harness.ts`. The Rust SDK does not ship
//! this type on the `1.0.x` line we depend on (it was added on the unreleased
//! `2.x` line — see the SDK port in tinyhumansai/tiny.place#210). Once a crate
//! version that includes it is published *and* openhuman migrates off
//! `tinyplace 1.0.1`, replace this block with
//! `use tinyplace::types::SessionEnvelopeV1;` — field names already match.
//!
//! Wire format is **snake_case** (the CLI wrapper emits a literal snake_case
//! object), unlike the camelCase tiny.place API — so these structs deliberately
//! omit `#[serde(rename_all = "camelCase")]`.

use serde::{Deserialize, Serialize};

/// `envelope_version` discriminator for v1 harness envelopes.
pub const SESSION_ENVELOPE_VERSION_V1: &str = "tinyplace.harness.session.v1";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HarnessBucket {
    #[serde(default)]
    pub unit: String,
    #[serde(default)]
    pub start: String,
    #[serde(default)]
    pub end: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HarnessScope {
    #[serde(rename = "type", default)]
    pub scope_type: String,
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub wrapper_session_id: String,
    #[serde(default)]
    pub harness_session_id: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HarnessInfo {
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub argv: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HarnessEnvelopeMessage {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub line: i64,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub timestamp: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HarnessSource {
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub record_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_role: Option<String>,
}

/// Mirror of the TS `SessionEnvelopeV1` interface.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionEnvelopeV1 {
    #[serde(default)]
    pub envelope_version: String,
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub bucket: HarnessBucket,
    #[serde(default)]
    pub scope: HarnessScope,
    #[serde(default)]
    pub harness: HarnessInfo,
    #[serde(default)]
    pub message: HarnessEnvelopeMessage,
    #[serde(default)]
    pub source: HarnessSource,
}

impl SessionEnvelopeV1 {
    fn is_valid_v1(&self) -> bool {
        self.envelope_version == SESSION_ENVELOPE_VERSION_V1
            && !self.scope.harness_session_id.is_empty()
    }

    /// Parse a decrypted DM body as a v1 session envelope. Returns `None` for
    /// any non-envelope payload (a plain DM) so callers route it to Master.
    pub fn parse(body: &str) -> Option<Self> {
        let envelope: Self = serde_json::from_str(body).ok()?;
        envelope.is_valid_v1().then_some(envelope)
    }
}

/// Which pinned/session window a persisted message belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatKind {
    Master,
    Subconscious,
    Session,
}

impl ChatKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ChatKind::Master => "master",
            ChatKind::Subconscious => "subconscious",
            ChatKind::Session => "session",
        }
    }

    /// Parse the persisted string form back into a [`ChatKind`]. Unknown values
    /// fall back to [`ChatKind::Master`] (the safe, non-session default).
    pub fn from_str(s: &str) -> Self {
        match s {
            "session" => ChatKind::Session,
            "subconscious" => ChatKind::Subconscious,
            _ => ChatKind::Master,
        }
    }
}

/// Durable per-session record. `session_id` is the harness session id for
/// [`ChatKind::Session`], or the literal `"master"` for a peer's Master window.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrchestrationSession {
    pub session_id: String,
    pub agent_id: String,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    pub last_seq: i64,
    pub created_at: String,
    pub last_message_at: String,
}

/// A single persisted message. `body` is DECRYPTED plaintext and therefore
/// workspace-internal only.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrchestrationMessage {
    pub id: String,
    pub agent_id: String,
    pub session_id: String,
    pub chat_kind: ChatKind,
    pub role: String,
    pub body: String,
    pub timestamp: String,
    pub seq: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
        "envelope_version": "tinyplace.harness.session.v1",
        "version": 1,
        "scope": { "type": "session", "key": "repo", "cwd": "/w",
                   "wrapper_session_id": "w1", "harness_session_id": "h1" },
        "harness": { "provider": "claude", "command": "claude", "argv": [] },
        "message": { "id": "m1", "line": 2, "role": "agent", "text": "hi",
                     "timestamp": "2026-07-02T00:00:00Z" },
        "source": { "path": "p", "record_type": "assistant" }
    }"#;

    #[test]
    fn parses_valid_v1_envelope() {
        let env = SessionEnvelopeV1::parse(SAMPLE).expect("valid v1");
        assert_eq!(env.scope.harness_session_id, "h1");
        assert_eq!(env.message.role, "agent");
        assert_eq!(env.harness.provider, "claude");
    }

    #[test]
    fn rejects_non_envelope_and_bad_version() {
        assert!(SessionEnvelopeV1::parse("a plain message").is_none());
        assert!(SessionEnvelopeV1::parse(
            r#"{"envelope_version":"x","scope":{"harness_session_id":"h"}}"#
        )
        .is_none());
        assert!(SessionEnvelopeV1::parse(
            r#"{"envelope_version":"tinyplace.harness.session.v1","scope":{"harness_session_id":""}}"#
        )
        .is_none());
    }
}
