use serde::Deserialize;

use crate::openhuman::agent::Agent;

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
}

pub(super) struct SessionEntry {
    pub(super) agent: Agent,
    pub(super) fingerprint: SessionCacheFingerprint,
}

#[derive(Debug)]
pub(super) struct InFlightEntry {
    pub(super) request_id: String,
    pub(super) handle: tokio::task::JoinHandle<()>,
    pub(super) run_queue: std::sync::Arc<crate::openhuman::agent::harness::run_queue::RunQueue>,
}

#[derive(Debug, Clone)]
pub(super) struct WebChatTaskResult {
    pub(super) full_response: String,
    pub(super) citations: Vec<crate::openhuman::agent::memory_loader::MemoryCitation>,
}

/// Per-request metadata carried alongside a chat send. Currently used by the
/// PTT flow (Task 4 wires it to `voice::reply_speech`); other voice surfaces
/// can populate it the same way.
#[derive(Debug, Default, Clone)]
pub struct ChatRequestMetadata {
    pub speak_reply: Option<bool>,
    pub source: Option<String>,
    pub session_id: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct WebChatParams {
    pub(super) client_id: String,
    pub(super) thread_id: String,
    pub(super) message: String,
    pub(super) model_override: Option<String>,
    pub(super) temperature: Option<f64>,
    pub(super) profile_id: Option<String>,
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
}

#[derive(Debug, Deserialize)]
pub(super) struct WebQueueParams {
    pub(super) thread_id: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct WebCancelParams {
    pub(super) client_id: String,
    pub(super) thread_id: String,
}
