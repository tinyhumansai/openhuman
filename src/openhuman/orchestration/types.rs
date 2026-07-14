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

/// Sentinel counterpart for a **local** Master-chat cycle — the human asking the
/// OpenHuman agent itself (W2), as opposed to a real external peer. When the wake
/// graph sees this as the counterpart it must NOT send an outbound tiny.place DM;
/// the reply belongs in the Master window. Contains a `:` so it can never collide
/// with a real base58 tiny.place address.
pub const LOCAL_MASTER_AGENT: &str = "openhuman:local";

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

    /// The single per-pair session id to bucket an inbound message under. Every
    /// message — whoever sent it — carries exactly one id: the shared conversation
    /// id in `scope.wrapper_session_id`. Both peers put the SAME id there for a given
    /// thread (the peer reuses it on reply), so it is the sole routing key. Falls
    /// back to `harness_session_id` only for a legacy envelope that predates the
    /// per-pair id.
    pub fn session_key(&self) -> String {
        if !self.scope.wrapper_session_id.is_empty() {
            return self.scope.wrapper_session_id.clone();
        }
        self.scope.harness_session_id.clone()
    }

    /// Build an outgoing v1 session envelope carrying `body` under `session_id`,
    /// so a compliant peer harness threads its reply under the same session.
    pub fn outgoing(session_id: &str, body: &str, message_id: &str, timestamp: &str) -> Self {
        SessionEnvelopeV1 {
            envelope_version: SESSION_ENVELOPE_VERSION_V1.to_string(),
            version: 1,
            scope: HarnessScope {
                scope_type: "session".to_string(),
                wrapper_session_id: session_id.to_string(),
                harness_session_id: session_id.to_string(),
                ..Default::default()
            },
            message: HarnessEnvelopeMessage {
                id: message_id.to_string(),
                role: "owner".to_string(),
                text: body.to_string(),
                timestamp: timestamp.to_string(),
                ..Default::default()
            },
            ..Default::default()
        }
    }
}

// ── v2 envelope: `tinyplace.harness.session.v2` ──────────────────────────────
//
// Hand-rolled mirror of the tiny.place v2 harness wire (snake_case, from the
// TypeScript source), added beside the v1 mirror above. v2 replaces v1's single
// `message` object with a typed `event` carrying a `kind` discriminator + a
// per-kind `payload`, plus a richer `status` run-state. Same
// `scope.wrapper_session_id` routing key as v1. See the port in
// tinyhumansai/tiny.place#210; fold in `tinyplace::types::SessionEnvelopeV2` once
// the vendored SDK ships it.

/// `envelope_version` discriminator for v2 harness envelopes.
pub const SESSION_ENVELOPE_VERSION_V2: &str = "tinyplace.harness.session.v2";

/// The typed payload of a v2 `event`, keyed by `event.kind`. Adjacently tagged on
/// the wire's `kind` (discriminator) + `payload` (content) fields, decoded via
/// [`HarnessEvent::decoded`]. `snake_case` matches the wire kind strings.
///
/// `tool_kind` inside [`ToolCallPayload`] is intentionally a `String` (not an
/// enum) so an unrecognised tool family does not fail the whole event decode.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub enum HarnessEventKind {
    /// `session_info` — the session intro/announce, emitted once a wrapped agent
    /// session is initialised (spec §2). Enrichment, not a prerequisite: a session
    /// is lazy-created on the first event of any kind regardless.
    SessionInfo(SessionInfoPayload),
    UserPrompt(UserPromptPayload),
    AgentMessage(TextPayload),
    AgentThinking(TextPayload),
    ToolCall(ToolCallPayload),
    ToolResult(ToolResultPayload),
    ApprovalRequest(ApprovalRequestPayload),
    Status(StatusPayload),
    Lifecycle(LifecyclePayload),
    Error(ErrorPayload),
    /// `unknown` wire kind (`{ "raw": any }`) OR any forward-incompatible kind we
    /// cannot decode — folded here rather than hard-failing the parse.
    Unknown(UnknownPayload),
}

/// `session_info` payload — the session intro/announce (spec §2b). Thin: the
/// provider / cwd / session-ids ride on the envelope frame, so this carries only
/// what the frame lacks (identity confirmation, capabilities, UI metadata). Field
/// names are byte-identical to the TS `SessionInfoPayload` emitter (spec §2a).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SessionInfoPayload {
    /// base58 wallet identity — confirms `envelope.from`.
    #[serde(default)]
    pub agent_address: String,
    /// `@handle`, if registered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handle: Option<String>,
    /// Human-friendly session title for the UI header.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// git remote/slug, if in a repo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// git branch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// Active model (may also ride on `event.model`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// plugin/CLI version — compat gating.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness_version: Option<String>,
    /// Advertised event kinds (subset of the wire `kind` strings) this session
    /// will emit — feature-gates the UI.
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// `false` = fresh spawn, `true` = reconnect/resume (drives idempotent upsert
    /// rather than a duplicate record).
    #[serde(default)]
    pub resumed: bool,
    /// ISO-8601 session start.
    #[serde(default)]
    pub started_at: String,
}

/// `user_prompt` payload.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct UserPromptPayload {
    #[serde(default)]
    pub text: String,
    /// `"human"` | `"openhuman_inject"`.
    #[serde(default)]
    pub source: String,
}

/// Shared payload for the text-only kinds (`agent_message`, `agent_thinking`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TextPayload {
    #[serde(default)]
    pub text: String,
}

/// `tool_call` payload.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ToolCallPayload {
    #[serde(default)]
    pub call_id: String,
    #[serde(default)]
    pub tool_name: String,
    /// `shell|file_read|file_write|edit|search|web|mcp|task|other` — kept as a
    /// free `String` for forward-compatibility (see [`HarnessEventKind`]).
    #[serde(default)]
    pub tool_kind: String,
    #[serde(default)]
    pub display: String,
    #[serde(default)]
    pub input: serde_json::Value,
}

/// `tool_result` payload.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ToolResultPayload {
    #[serde(default)]
    pub call_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ok: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i64>,
    #[serde(default)]
    pub is_error: bool,
    #[serde(default)]
    pub output: String,
    #[serde(default)]
    pub output_bytes: i64,
}

/// `approval_request` payload.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ApprovalRequestPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
    #[serde(default)]
    pub tool_name: String,
    #[serde(default)]
    pub display: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// `status` payload — the harness run-state.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct StatusPayload {
    /// `running|running_tool|waiting_approval|idle|stopped|errored`.
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub detail: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_call_id: Option<String>,
}

/// `lifecycle` payload.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct LifecyclePayload {
    /// `session_start|session_end|turn_start|turn_end|compact`.
    #[serde(default)]
    pub phase: String,
}

/// `error` payload.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ErrorPayload {
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub fatal: bool,
}

/// `unknown` payload (`{ "raw": any }`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct UnknownPayload {
    #[serde(default)]
    pub raw: serde_json::Value,
}

/// A v2 `event`: common envelope fields + the `kind`/`payload` pair. `kind` and
/// `payload` are kept raw here (a discriminator string + arbitrary JSON) and
/// decoded on demand by [`HarnessEvent::decoded`] so an unknown/garbled event
/// never fails the whole envelope parse (it folds to
/// [`HarnessEventKind::Unknown`]).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HarnessEvent {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub seq: i64,
    #[serde(default)]
    pub ts: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// `owner` (iff `kind == user_prompt`) else `agent`.
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub payload: serde_json::Value,
}

impl HarnessEvent {
    /// Decode `(kind, payload)` into a typed [`HarnessEventKind`]. Forward-safe:
    /// an unrecognised `kind` or a payload that fails its kind's schema folds to
    /// [`HarnessEventKind::Unknown`] carrying the raw payload, so a future/garbled
    /// event never hard-fails the parse (which would silently route it to Master).
    pub fn decoded(&self) -> HarnessEventKind {
        let tagged = serde_json::json!({ "kind": self.kind, "payload": self.payload });
        serde_json::from_value(tagged).unwrap_or_else(|_| {
            HarnessEventKind::Unknown(UnknownPayload {
                raw: self.payload.clone(),
            })
        })
    }
}

/// Mirror of the TS `SessionEnvelopeV2` interface. Shares the v1 `bucket`/`scope`/
/// `harness`/`source` blocks; swaps v1's `message` for a typed [`HarnessEvent`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionEnvelopeV2 {
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
    pub event: HarnessEvent,
    #[serde(default)]
    pub source: HarnessSource,
}

impl SessionEnvelopeV2 {
    fn is_valid_v2(&self) -> bool {
        self.envelope_version == SESSION_ENVELOPE_VERSION_V2
            && !self.scope.harness_session_id.is_empty()
    }

    /// Parse a decrypted DM body as a v2 session envelope. Returns `None` for any
    /// non-v2 payload (a v1 envelope or a plain DM) so the classifier falls
    /// through to v1 then Master. Discriminates purely on `envelope_version`, so a
    /// v1 body never matches here (and vice-versa).
    pub fn parse(body: &str) -> Option<Self> {
        let envelope: Self = serde_json::from_str(body).ok()?;
        envelope.is_valid_v2().then_some(envelope)
    }

    /// The per-pair routing key — identical semantics to v1: the shared
    /// `scope.wrapper_session_id`, falling back to `harness_session_id` for a
    /// legacy envelope with no per-pair id.
    pub fn session_key(&self) -> String {
        if !self.scope.wrapper_session_id.is_empty() {
            return self.scope.wrapper_session_id.clone();
        }
        self.scope.harness_session_id.clone()
    }
}

/// Which pinned/session window a persisted message belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatKind {
    /// Safe default (matches [`ChatKind::from_str`]'s unknown fallback).
    #[default]
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
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
    /// v2 harness run-state from `status.state`
    /// (`running|running_tool|waiting_approval|idle|stopped|errored`). `None` for
    /// v1/legacy sessions — [`crate`]'s `derive_status` then falls back to recency.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_state: Option<String>,
    /// v2 `status.detail` — the current-activity line surfaced on the roster.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_detail: Option<String>,
    /// v2 `status.active_call_id` — the in-flight tool call while running a tool.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_call_id: Option<String>,
    // ── session_info enrichment (spec §4) ────────────────────────────────────
    // Populated from a `session_info` event; `None`/empty until one arrives (a
    // session is lazy-created on the first event of any kind regardless).
    /// `session_info.title` — human-friendly session title for the UI header.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// `session_info.model` — the active model advertised at session start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// `session_info.handle` — the agent's `@handle`, if registered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handle: Option<String>,
    /// `session_info.repo` — git remote/slug the session runs in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// `session_info.branch` — git branch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// `session_info.capabilities` — advertised event kinds, for feature-gating
    /// the UI. Empty for v1/legacy sessions and until a `session_info` arrives.
    #[serde(default)]
    pub capabilities: Vec<String>,
}

/// A single persisted message. `body` is DECRYPTED plaintext and therefore
/// workspace-internal only.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
    /// v2 `event.kind` (`user_prompt`/`agent_message`/`tool_call`/…). `None` for
    /// v1 and pinned master/subconscious rows, which carry no per-event kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_kind: Option<String>,
    /// v2 tool identifier for `tool_call` / `approval_request` rows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    /// v2 correlation id linking a `tool_result` back to its `tool_call`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
    /// v2 `tool_result.ok` — whether the tool call succeeded. `None` on every row
    /// that is not a `tool_result` (so the renderer can distinguish a failed run
    /// from a successful one instead of both reading as plain output).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ok: Option<bool>,
    /// v2 `tool_result.is_error` — the harness flagged the result as an error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
    /// v2 `tool_result.exit_code` — process exit code when the tool was a command.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i64>,
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
    fn outgoing_builds_a_parseable_v1_envelope() {
        let env = SessionEnvelopeV1::outgoing("h9", "reply body", "m9", "2026-07-04T00:00:00Z");
        let wire = serde_json::to_string(&env).expect("encode");
        let parsed = SessionEnvelopeV1::parse(&wire).expect("valid v1");
        assert_eq!(parsed.scope.harness_session_id, "h9");
        assert_eq!(parsed.scope.wrapper_session_id, "h9");
        assert_eq!(parsed.message.text, "reply body");
        assert_eq!(parsed.message.role, "owner");
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

    // ── v2 envelope ─────────────────────────────────────────────────────────

    /// Build a v2 envelope wire string with the given `kind` + `payload` JSON.
    fn v2_wire(kind: &str, payload: &str) -> String {
        format!(
            r#"{{
                "envelope_version": "tinyplace.harness.session.v2",
                "version": 2,
                "bucket": {{ "unit": "minute", "start": "s", "end": "e" }},
                "scope": {{ "type": "folder", "key": "repo", "cwd": "/w",
                           "wrapper_session_id": "w2", "harness_session_id": "h2" }},
                "harness": {{ "provider": "claude", "command": "claude", "argv": [] }},
                "event": {{ "id": "e1", "seq": 4, "ts": "2026-07-05T00:00:00Z",
                           "turn_id": "t1", "model": "opus", "role": "agent",
                           "kind": "{kind}", "payload": {payload} }},
                "source": {{ "path": "p", "record_type": "assistant" }}
            }}"#
        )
    }

    #[test]
    fn parses_valid_v2_envelope_and_common_event_fields() {
        let wire = v2_wire("agent_message", r#"{ "text": "hi there" }"#);
        let env = SessionEnvelopeV2::parse(&wire).expect("valid v2");
        assert_eq!(env.envelope_version, SESSION_ENVELOPE_VERSION_V2);
        assert_eq!(env.scope.wrapper_session_id, "w2");
        assert_eq!(env.harness.provider, "claude");
        assert_eq!(env.event.seq, 4);
        assert_eq!(env.event.turn_id.as_deref(), Some("t1"));
        assert_eq!(env.event.model.as_deref(), Some("opus"));
        assert_eq!(env.event.role, "agent");
        assert_eq!(env.session_key(), "w2");
    }

    #[test]
    fn v2_decodes_every_event_kind() {
        use HarnessEventKind::*;

        let up = SessionEnvelopeV2::parse(&v2_wire(
            "user_prompt",
            r#"{ "text": "do it", "source": "human" }"#,
        ))
        .unwrap();
        assert_eq!(
            up.event.decoded(),
            UserPrompt(UserPromptPayload {
                text: "do it".into(),
                source: "human".into()
            })
        );

        let am =
            SessionEnvelopeV2::parse(&v2_wire("agent_message", r#"{ "text": "ok" }"#)).unwrap();
        assert_eq!(
            am.event.decoded(),
            AgentMessage(TextPayload { text: "ok".into() })
        );

        let th =
            SessionEnvelopeV2::parse(&v2_wire("agent_thinking", r#"{ "text": "hmm" }"#)).unwrap();
        assert_eq!(
            th.event.decoded(),
            AgentThinking(TextPayload { text: "hmm".into() })
        );

        let tc = SessionEnvelopeV2::parse(&v2_wire(
            "tool_call",
            r#"{ "call_id": "c1", "tool_name": "bash", "tool_kind": "shell",
                 "display": "ls -la", "input": { "cmd": "ls" } }"#,
        ))
        .unwrap();
        match tc.event.decoded() {
            ToolCall(p) => {
                assert_eq!(p.call_id, "c1");
                assert_eq!(p.tool_name, "bash");
                assert_eq!(p.tool_kind, "shell");
                assert_eq!(p.display, "ls -la");
                assert_eq!(p.input["cmd"], "ls");
            }
            other => panic!("expected tool_call, got {other:?}"),
        }

        let tr = SessionEnvelopeV2::parse(&v2_wire(
            "tool_result",
            r#"{ "call_id": "c1", "ok": true, "exit_code": 0, "is_error": false,
                 "output": "done", "output_bytes": 4 }"#,
        ))
        .unwrap();
        assert_eq!(
            tr.event.decoded(),
            ToolResult(ToolResultPayload {
                call_id: "c1".into(),
                ok: Some(true),
                exit_code: Some(0),
                is_error: false,
                output: "done".into(),
                output_bytes: 4,
            })
        );

        let tr_unknown = SessionEnvelopeV2::parse(&v2_wire(
            "tool_result",
            r#"{ "call_id": "c1", "is_error": false, "output": "done" }"#,
        ))
        .unwrap();
        match tr_unknown.event.decoded() {
            ToolResult(p) => assert_eq!(p.ok, None),
            other => panic!("expected tool_result, got {other:?}"),
        }

        let ar = SessionEnvelopeV2::parse(&v2_wire(
            "approval_request",
            r#"{ "call_id": "c9", "tool_name": "rm", "display": "rm -rf x", "reason": "destructive" }"#,
        ))
        .unwrap();
        assert_eq!(
            ar.event.decoded(),
            ApprovalRequest(ApprovalRequestPayload {
                call_id: Some("c9".into()),
                tool_name: "rm".into(),
                display: "rm -rf x".into(),
                reason: Some("destructive".into()),
            })
        );

        let st = SessionEnvelopeV2::parse(&v2_wire(
            "status",
            r#"{ "state": "running_tool", "detail": "compiling", "active_call_id": "c1" }"#,
        ))
        .unwrap();
        assert_eq!(
            st.event.decoded(),
            Status(StatusPayload {
                state: "running_tool".into(),
                detail: "compiling".into(),
                active_call_id: Some("c1".into()),
            })
        );

        let lc = SessionEnvelopeV2::parse(&v2_wire("lifecycle", r#"{ "phase": "session_end" }"#))
            .unwrap();
        assert_eq!(
            lc.event.decoded(),
            Lifecycle(LifecyclePayload {
                phase: "session_end".into()
            })
        );

        let er =
            SessionEnvelopeV2::parse(&v2_wire("error", r#"{ "message": "boom", "fatal": true }"#))
                .unwrap();
        assert_eq!(
            er.event.decoded(),
            Error(ErrorPayload {
                message: "boom".into(),
                fatal: true
            })
        );

        let uk = SessionEnvelopeV2::parse(&v2_wire("unknown", r#"{ "raw": { "x": 1 } }"#)).unwrap();
        match uk.event.decoded() {
            Unknown(p) => assert_eq!(p.raw["x"], 1),
            other => panic!("expected unknown, got {other:?}"),
        }
    }

    #[test]
    fn v2_decodes_session_info_and_round_trips_wire_field_names() {
        let wire = v2_wire(
            "session_info",
            r#"{ "agent_address": "ELQUJvq27tYx", "handle": "@alice",
                 "title": "myrepo · feat/x", "repo": "org/myrepo", "branch": "feat/x",
                 "model": "claude-opus-4-8", "harness_version": "1.4.2",
                 "capabilities": ["agent_message", "tool_call"],
                 "resumed": false, "started_at": "2026-07-08T00:00:00Z" }"#,
        );
        let env = SessionEnvelopeV2::parse(&wire).expect("valid v2");
        let decoded = env.event.decoded();
        let expected = SessionInfoPayload {
            agent_address: "ELQUJvq27tYx".into(),
            handle: Some("@alice".into()),
            title: Some("myrepo · feat/x".into()),
            repo: Some("org/myrepo".into()),
            branch: Some("feat/x".into()),
            model: Some("claude-opus-4-8".into()),
            harness_version: Some("1.4.2".into()),
            capabilities: vec!["agent_message".into(), "tool_call".into()],
            resumed: false,
            started_at: "2026-07-08T00:00:00Z".into(),
        };
        assert_eq!(decoded, HarnessEventKind::SessionInfo(expected.clone()));

        // Re-encode the adjacently-tagged kind and decode again: a full round-trip
        // proves the `kind`/`payload` framing survives serialize → deserialize.
        let reencoded = serde_json::to_string(&HarnessEventKind::SessionInfo(expected.clone()))
            .expect("encode kind");
        let back: HarnessEventKind = serde_json::from_str(&reencoded).expect("decode kind");
        assert_eq!(back, HarnessEventKind::SessionInfo(expected.clone()));

        // The payload's wire field names must be byte-identical to the TS emitter
        // (spec §2a): snake_case keys, optionals present only when set.
        let payload_json = serde_json::to_value(&expected).unwrap();
        for key in [
            "agent_address",
            "handle",
            "title",
            "repo",
            "branch",
            "model",
            "harness_version",
            "capabilities",
            "resumed",
            "started_at",
        ] {
            assert!(payload_json.get(key).is_some(), "missing wire key `{key}`");
        }

        // Optionals are omitted when absent (`skip_serializing_if`), and defaults
        // fill missing wire fields so a thin `session_info` never fails to decode.
        let minimal =
            SessionEnvelopeV2::parse(&v2_wire("session_info", r#"{ "agent_address": "X" }"#))
                .expect("valid v2");
        match minimal.event.decoded() {
            HarnessEventKind::SessionInfo(p) => {
                assert_eq!(p.agent_address, "X");
                assert_eq!(p.handle, None);
                assert!(p.capabilities.is_empty());
                assert!(!p.resumed);
            }
            other => panic!("expected session_info, got {other:?}"),
        }
        let minimal_json = serde_json::to_value(SessionInfoPayload {
            agent_address: "X".into(),
            ..Default::default()
        })
        .unwrap();
        assert!(
            minimal_json.get("handle").is_none(),
            "unset optionals must be skipped on the wire"
        );
    }

    #[test]
    fn v2_unrecognised_kind_folds_to_unknown_not_a_parse_error() {
        // A future kind the receiver doesn't model must not fail the envelope
        // parse (which would silently route the DM to Master); it folds to Unknown
        // carrying the raw payload.
        let env = SessionEnvelopeV2::parse(&v2_wire("quantum_teleport", r#"{ "flux": 42 }"#))
            .expect("still a valid v2 envelope");
        match env.event.decoded() {
            HarnessEventKind::Unknown(p) => assert_eq!(p.raw["flux"], 42),
            other => panic!("expected unknown fold, got {other:?}"),
        }
    }

    #[test]
    fn v2_rejects_v1_body_and_plain_and_bad_version() {
        // A v1 envelope must NOT parse as v2 (discriminated on envelope_version).
        assert!(SessionEnvelopeV2::parse(SAMPLE).is_none());
        assert!(SessionEnvelopeV2::parse("a plain message").is_none());
        // Right shape, wrong version string.
        assert!(SessionEnvelopeV2::parse(
            r#"{"envelope_version":"tinyplace.harness.session.v3","scope":{"harness_session_id":"h"}}"#
        )
        .is_none());
        // Correct version but empty harness id → invalid.
        assert!(SessionEnvelopeV2::parse(
            r#"{"envelope_version":"tinyplace.harness.session.v2","scope":{"harness_session_id":""}}"#
        )
        .is_none());
        // Conversely a v2 body is not a v1 envelope.
        let v2 = v2_wire("agent_message", r#"{ "text": "x" }"#);
        assert!(SessionEnvelopeV1::parse(&v2).is_none());
    }

    #[test]
    fn v2_session_key_falls_back_to_harness_id() {
        let env = SessionEnvelopeV2::parse(
            r#"{
                "envelope_version": "tinyplace.harness.session.v2",
                "scope": { "harness_session_id": "h-only" },
                "event": { "kind": "agent_message", "payload": { "text": "x" } }
            }"#,
        )
        .expect("valid v2");
        assert_eq!(env.session_key(), "h-only");
    }

    #[test]
    fn session_key_is_the_shared_wrapper_id_then_harness_fallback() {
        // The single per-pair id lives in `wrapper_session_id`.
        assert_eq!(
            SessionEnvelopeV1::parse(SAMPLE).unwrap().session_key(),
            "w1"
        );
        // Legacy envelope with no per-pair id: fall back to the harness id.
        let env = SessionEnvelopeV1::parse(
            r#"{
                "envelope_version": "tinyplace.harness.session.v1",
                "scope": { "harness_session_id": "h-only" }
            }"#,
        )
        .expect("valid v1");
        assert_eq!(env.session_key(), "h-only");
    }
}
