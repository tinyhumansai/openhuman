//! Shared state and RPC parameter types for the web chat turn: the cached
//! `SessionEntry`/`SessionCacheFingerprint`, the turn result
//! (`WebChatTaskResult`), and the `channel.web_*` request param structs.

use serde::Deserialize;

use crate::agent::OpenHumanSessionHost;

/// All inputs that the cached `SessionEntry`'s `Agent` was built from,
/// captured at build time. The cache-hit predicate is a single
/// `entry.fingerprint == current_fingerprint` comparison — pulling the
/// fields into a named struct (instead of inlining four `&&`s) makes
/// the predicate testable in isolation and makes "what invalidates the
/// cache?" answerable in one place.
///
/// Adding a new dimension that should force a rebuild = add a field
/// here and populate it both at insert time and at the call-site
/// fingerprint construction.
#[derive(PartialEq, Debug, Clone)]
pub(crate) struct SessionCacheFingerprint {
    pub(super) model_override: Option<String>,
    /// Resolved model selected by the effective configuration, including a
    /// persisted managed default when no per-turn override was sent.
    pub(super) effective_model: String,
    pub(super) temperature: Option<f64>,
    pub(super) target_agent_id: String,
    pub(super) provider_binding: String,
    pub(super) autonomy_signature: String,
    /// Signature of `config.model_registry`. The cached `Agent` stores a
    /// build-time `model_vision` bool; toggling a model's "Supports vision" flag
    /// keeps the same model id (so neither `model_override` nor `provider_binding`
    /// change) — without this the stale session would be reused. Mirrors
    /// [`Self::autonomy_signature`].
    pub(super) model_registry_signature: String,
    /// The workspace the agent was built against. A cached agent carries its
    /// workspace's transcripts, memory binding and identity; reusing it after
    /// the workspace changed (a different user signed in, or another embedded
    /// agent asked for the same thread id) would hand one owner another's
    /// conversation.
    pub(super) workspace_dir: std::path::PathBuf,
}

pub(super) struct SessionEntry {
    pub(super) agent: OpenHumanSessionHost,
    pub(super) fingerprint: SessionCacheFingerprint,
}

#[derive(Debug, Clone)]
pub(super) struct WebChatTaskResult {
    pub(super) full_response: String,
    pub(super) citations: Vec<crate::web_chat::TurnCitation>,
    /// Holistic token/cost/context totals for the turn (parent + sub-agents),
    /// forwarded to the frontend on `chat_done`. `None` for synthetic results
    /// (e.g. budget-exhausted placeholders) that never ran a real turn.
    pub(super) usage: Option<crate::agent::tinyagents::host::LastTurnUsage>,
    /// The workspace this turn actually ran in, carried to delivery so the
    /// reply is stored there before it is announced (#6034).
    ///
    /// Taken from the config the turn resolved rather than re-read at delivery
    /// time: a sign-out or account switch moves `workspace_dir`, and a reply
    /// re-resolved afterwards would be filed under whoever is signed in when
    /// the turn happens to finish.
    pub(super) workspace_dir: std::path::PathBuf,
    /// The bridge's `TurnTiming` snapshot (first-token/first-tool/total ms),
    /// read from `ProgressBridgeHandle::timing_snapshot()` after
    /// `wait_drained` — i.e. after the bridge has seen `TurnCompleted`.
    /// `None` for a synthetic result (budget-exhausted placeholder) that
    /// never ran a bridge, or a turn that errored before completing a round.
    pub(super) timing: Option<tinychannels_bus::web::TurnTimingSnapshot>,
}

/// Per-request metadata carried alongside a chat send (the PTT flow, the voice
/// notes, trace attribution); the type is the web contract's.
pub use tinychannels_bus::web::ChatRequestMetadata;

#[derive(Debug, Deserialize)]
pub(crate) struct WebChatParams {
    pub(super) client_id: String,
    pub(super) thread_id: String,
    pub(super) message: String,
    /// Typed media refs returned for a previously persisted user row. The
    /// trusted runner resolves these refs against that exact row at execution.
    #[serde(default)]
    pub(super) attachments: Vec<tinychannels_bus::ContentPart>,
    /// Persisted user message whose opaque attachment refs accompany this turn.
    #[serde(default)]
    pub(super) persisted_message_id: Option<String>,
    pub(super) model_override: Option<String>,
    pub(super) temperature: Option<f64>,
    /// BCP-47 locale of the frontend UI (e.g. `ar`, `zh-CN`). When set
    /// and not English, the system prompt is augmented to ask the
    /// agent to reply in that language. `None` keeps the agent's
    /// default language (English) so existing integrations don't
    /// silently change behaviour.
    pub(super) locale: Option<String>,
    /// When `true`, the agent's final reply should be spoken via TTS
    /// (for PTT and similar background voice flows). Accepted and
    /// stored here; wired to TTS in Task 4.
    #[serde(default)]
    pub(super) speak_reply: Option<bool>,
    /// Origin of the message: `"ptt"` | `"dictation"` | `"type"` | other.
    /// Used for analytics and downstream metadata.
    #[serde(default)]
    pub(super) source: Option<String>,
    /// Optional caller-provided correlation id (PTT session id).
    #[serde(default)]
    pub(super) session_id: Option<u64>,
    /// Queue mode for concurrent messages: `interrupt` (default), `steer`,
    /// `followup`, or `collect`.
    #[serde(default)]
    pub(super) queue_mode: Option<String>,
    /// Optional `"plan"` | `"build"` — lets the caller start this turn with
    /// the thread already in the requested run mode, mirroring the socket
    /// `chat:start` payload's `run_mode` field. Unrecognized values are
    /// ignored (logged), not rejected.
    #[serde(default)]
    pub(super) run_mode: Option<String>,
    /// Optional reasoning effort for this thread (`inference::reasoning`).
    /// Omitted leaves the thread's prior choice; `"default"` clears it.
    #[serde(default)]
    pub(super) reasoning_effort: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct WebQueueParams {
    pub(super) thread_id: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct WebQueueRemoveParams {
    pub(super) client_id: String,
    pub(super) thread_id: String,
    pub(super) item_id: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct WebCancelParams {
    pub(super) client_id: String,
    pub(super) thread_id: String,
    /// The `request_id` this cancel targets. When present, the cancel is scoped
    /// to that exact turn, so a stale cancel for a superseded request can't kill
    /// the newer turn on the thread (#4760). Absent = stop whatever is running.
    #[serde(default)]
    pub(super) request_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct VoiceNoteStoreParams {
    pub(super) note_id: String,
    pub(super) audio_base64: String,
    #[serde(default)]
    pub(super) mime: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct VoiceNoteReadParams {
    pub(super) note_id: String,
}

#[cfg(test)]
#[path = "types_tests.rs"]
mod tests;
