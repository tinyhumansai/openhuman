//! Host-owned payload for a message queued while an agent turn is active.
//!
//! TinyAgents owns the queue mechanics and its [`QueueLane`](tinyagents_harness::run_queue::QueueLane)
//! selects how this payload is consumed. OpenHuman retains the web and
//! orchestration metadata needed to dispatch a deferred follow-up turn.

use std::sync::Arc;
use tinyagents_harness::turn_runner::{QueueDisposition, RequeuePayload, ScopeKey, TurnRequest};

/// Opaque attachment refs supplied for a user row that is already persisted.
/// Paths and URLs are intentionally absent; the trusted turn resolves the
/// parts again against the row and current dispatch scope before use.
#[derive(Clone, Debug, Default)]
pub(crate) struct StoredAttachmentInput {
    pub(crate) persisted_message_id: String,
    pub(crate) parts: Vec<tinychannels_bus::ContentPart>,
}

/// Opaque refs staged for a queued follow-up. The user row is persisted only
/// when the turn is admitted, preserving transcript order behind the active
/// assistant reply.
#[derive(Clone, Debug, Default)]
pub(crate) struct PendingAttachmentInput {
    pub(crate) parts: Vec<tinychannels_bus::ContentPart>,
}

/// The host metadata one OpenHuman turn carries through TinyAgents' turn
/// runner and its run queue: which client asked, the per-message model and
/// locale choices, and the context the turn runs under.
///
/// This is the `P` of [`QueuedTurn`]. A turn the runner queues (steer,
/// follow-up, collect) keeps all of it, so a deferred turn dispatches exactly
/// as the user asked.
#[derive(Clone, Default)]
pub struct QueuedPayload {
    /// The context the turn runs under. `None` on a placeholder the harness
    /// rebuilt from a bare id and text; the runner fills it from the running
    /// turn ([`RequeuePayload::adopt`]).
    pub ctx: Option<crate::core::runtime::DispatchCtx>,
    pub client_id: String,
    pub model_override: Option<String>,
    pub temperature: Option<f64>,
    pub locale: Option<String>,
    pub metadata: tinychannels_bus::web::ChatRequestMetadata,
    pub queued_at_ms: u64,
    /// Already-resolved internal agent authority and durable channel identity.
    /// Requeued placeholders inherit it from their running turn; public chat
    /// metadata never supplies or overrides this value.
    pub(crate) trusted_turn: Option<Arc<super::turn_host::TrustedAgentTurn>>,
    /// Follow-up submissions defer their user row until the trusted runner
    /// admits them, so transcript order follows the preceding assistant turn.
    pub(crate) deferred_user_message: bool,
    pub(crate) stored_attachments: Option<StoredAttachmentInput>,
    pub(crate) pending_attachments: Option<PendingAttachmentInput>,
    /// Typed native-channel input resolved from the exact module envelope for
    /// this queued request. Synthetic queue placeholders deliberately do not
    /// inherit it from the currently running turn.
    pub(crate) native_message_input: Option<tinyinference_llm::message::Message>,
    /// Host-created staged refs authorized for this exact external turn.
    /// Requeued synthetic placeholders never inherit this grant.
    pub(crate) native_staged_parts: Vec<tinychannels_bus::ContentPart>,
    /// Final-owner cleanup for uncommitted native refs, shared by all queue
    /// clones and disarmed only after durable claim persistence.
    pub(crate) native_staged_lease: Option<crate::threads::attachments::NativeStagedLease>,
    pub(crate) native_message_id: Option<String>,
    /// A preparation failure is sent through the shared runner's terminal
    /// path without invoking the model.
    pub(crate) native_preparation_error: Option<String>,
}

impl std::fmt::Debug for QueuedPayload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QueuedPayload")
            .field("client_id", &self.client_id)
            .field("model_override", &self.model_override)
            .field("temperature", &self.temperature)
            .field("locale", &self.locale)
            .field("queued_at_ms", &self.queued_at_ms)
            .field("has_trusted_turn", &self.trusted_turn.is_some())
            .field("deferred_user_message", &self.deferred_user_message)
            .field("has_stored_attachments", &self.stored_attachments.is_some())
            .field(
                "has_pending_attachments",
                &self.pending_attachments.is_some(),
            )
            .field(
                "has_native_message_input",
                &self.native_message_input.is_some(),
            )
            .field("native_staged_parts", &self.native_staged_parts.len())
            .field(
                "has_native_staged_lease",
                &self.native_staged_lease.is_some(),
            )
            .field(
                "has_native_preparation_error",
                &self.native_preparation_error.is_some(),
            )
            .finish_non_exhaustive()
    }
}

impl RequeuePayload for QueuedPayload {
    fn requeued(_thread_label: &str, queued_at_ms: u64) -> Self {
        Self {
            queued_at_ms,
            ..Self::default()
        }
    }

    fn adopt(&mut self, running: &Self) {
        if self.ctx.is_none() {
            self.ctx.clone_from(&running.ctx);
        }
        if self.trusted_turn.is_none() {
            self.trusted_turn.clone_from(&running.trusted_turn);
        }
        if self.client_id.is_empty() {
            self.client_id.clone_from(&running.client_id);
            self.model_override.clone_from(&running.model_override);
            self.temperature = running.temperature;
            self.locale.clone_from(&running.locale);
            self.metadata.clone_from(&running.metadata);
        }
    }
}

/// A queued OpenHuman turn input: TinyAgents' [`TurnRequest`] carrying
/// OpenHuman's [`QueuedPayload`]. The request id doubles as the queued item's
/// id (what `channel.web_queue_remove` targets); the lane is chosen at the
/// host boundary when the item is pushed into the runner's `RunQueue`.
pub type QueuedTurn = TurnRequest<QueuedPayload>;

/// A steering message for a running sub-agent, attributed to `client_id`.
#[must_use]
pub fn steer_message(
    id: impl Into<String>,
    text: impl Into<String>,
    client_id: &str,
    thread_id: &str,
) -> QueuedTurn {
    let queued_at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    TurnRequest {
        scope: ScopeKey {
            agent_id: String::new(),
            profile_id: None,
        },
        thread_id: thread_id.to_owned(),
        request_id: id.into(),
        text: text.into(),
        payload: QueuedPayload {
            client_id: client_id.to_owned(),
            queued_at_ms,
            ..QueuedPayload::default()
        },
        host_authored: false,
        disposition: QueueDisposition::Steer,
        deadline: None,
        idle_timeout: None,
    }
}

/// Preview a queued message's caption or attachment names, without exposing
/// inline upload payloads. This also matches previews made before staging by
/// the web composer, so cancellation finds the same pending follow-up.
#[must_use]
pub fn text_preview(text: &str) -> String {
    use std::sync::LazyLock;

    static MARKERS: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"\[(IMAGE|FILE|ATTACHMENT):([^\]]+)\]")
            .expect("queue attachment pattern is valid")
    });
    static SPACES: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(" {2,}").expect("space pattern is valid"));

    let mut names = Vec::new();
    let caption = MARKERS.replace_all(text, |capture: &regex::Captures<'_>| {
        if &capture[1] == "ATTACHMENT" {
            let (_, files) = crate::attachments::parse(&capture[0]);
            let Some(file) = files.into_iter().next() else {
                return capture[0].to_owned();
            };
            names.push(file.name);
        } else {
            // Decode only the header's name parameter; the payload stays opaque.
            let header = capture[2]
                .strip_prefix("data:")
                .map(|source| source.split(',').next().unwrap_or_default())
                .unwrap_or_default();
            let name = header
                .split(';')
                .find_map(|parameter| parameter.strip_prefix("name="))
                .and_then(|encoded| {
                    let parameter = format!("name={encoded}");
                    url::form_urlencoded::parse(parameter.as_bytes())
                        .next()
                        .map(|(_, value)| value.into_owned())
                })
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| "attachment".into());
            names.push(name);
        }
        String::new()
    });
    let caption = SPACES.replace_all(&caption, " ");
    let caption = caption.trim();
    let preview = if caption.is_empty() {
        names.join(", ")
    } else {
        caption.to_owned()
    };
    crate::core::events::clip_to_chars(&preview, 80)
}

#[cfg(test)]
#[path = "queued_turn_tests.rs"]
mod tests;
