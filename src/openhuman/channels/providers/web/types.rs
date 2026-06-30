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
    /// Serialized signature of the active agent profile. The cached `Agent`
    /// bakes in the profile's tool/skill/MCP/connector visibility and SOUL/MEMORY
    /// overrides at build time; switching profiles on the same thread keeps the
    /// same model/agent/provider, so without this the previous profile's
    /// capability surface would leak into the new profile's turns. Any change to
    /// the resolved profile forces a rebuild.
    pub(super) profile_signature: String,
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
    /// Cooperative cancellation for this turn. Cancelling it makes the turn's
    /// `tokio::select!` arm fire and drops the in-flight turn future (which
    /// cancels the in-flight LLM request and releases locks at a safe await
    /// point) — a graceful alternative to the abrupt `handle.abort()`. The
    /// handle is retained only as a hard backstop if cooperative cancellation
    /// does not land within a grace period.
    pub(super) cancel_token: tokio_util::sync::CancellationToken,
}

/// A concurrent, forked (`QueueMode::Parallel`) turn on a thread. Tracked in a
/// separate lane keyed by `request_id` so any number can run alongside the
/// primary in-flight turn without participating in interrupt/steer/queue
/// semantics. Carries `thread_id` so cancellation-by-thread can find it.
#[derive(Debug)]
pub(super) struct ParallelEntry {
    pub(super) thread_id: String,
    pub(super) handle: tokio::task::JoinHandle<()>,
    pub(super) cancel_token: tokio_util::sync::CancellationToken,
}

#[derive(Debug, Clone)]
pub(super) struct WebChatTaskResult {
    pub(super) full_response: String,
    pub(super) citations: Vec<crate::openhuman::agent::memory_loader::MemoryCitation>,
    /// Holistic token/cost/context totals for the turn (parent + sub-agents),
    /// forwarded to the frontend on `chat_done`. `None` for synthetic results
    /// (e.g. budget-exhausted placeholders) that never ran a real turn.
    pub(super) usage: Option<crate::openhuman::agent::harness::turn_subagent_usage::LastTurnUsage>,
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

impl ChatRequestMetadata {
    /// Constructor for messages submitted via the AgentBox `/run` HTTP surface
    /// (`OPENHUMAN_AGENTBOX_MODE=1`). These are background invocations driven
    /// programmatically by a remote marketplace caller — no live UI is
    /// attached to surface TTS or PTT signals — so `speak_reply` and
    /// `session_id` stay `None` and the `source` tag identifies the origin
    /// for analytics / log filtering downstream (mirrors the `"ptt"` /
    /// `"dictation"` / `"type"` convention used by the desktop UI).
    pub fn agentbox() -> Self {
        Self {
            speak_reply: None,
            source: Some("agentbox".to_string()),
            session_id: None,
        }
    }
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
