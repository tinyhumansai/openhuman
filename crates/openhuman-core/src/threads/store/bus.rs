//! Event-bus subscriber that mirrors inbound channel messages into the
//! workspace-backed conversation store, so non-web channels (Slack, Telegram,
//! etc.) persist alongside UI-driven threads.
//!
//! # The engine's `bus.rs` is not this file, and did not replace it (#5560)
//!
//! When the conversation store lived in the memory engine it carried a second
//! subscriber of its own. That one existed only because the engine could not
//! name the host's channel layer: it declared a `ConversationEventBus` trait
//! for a host to implement, a self-contained `ChannelEvent` enum standing in
//! for `DomainEvent::ChannelMessage*`, and an inlined copy of
//! `conversation_history_key`'s thread-id derivation — with a comment
//! promising the copy stayed byte-identical to this one.
//!
//! No host ever implemented that trait. This file has always been the live
//! subscriber, wired to `crate::core::bus::BUS` and calling the real
//! [`conversation_history_key`], and moving the store home means the
//! abstraction has nothing left to abstract. So the engine's version stayed
//! where it was and only the store import below changed. Two things follow:
//! there is exactly one thread-id derivation again rather than a copy and a
//! promise, and this file's behaviour is untouched by the move — which matters,
//! because it decides the id under which channel turns are persisted.
use crate::core::runtime::DispatchCtx;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock};

use async_trait::async_trait;
use chrono::Utc;
use serde_json::json;

use crate::core::events::DomainEvent;
use tinybus::EventHandler;
use tinybus::SubscriptionHandle;
use tinychannels_bus::channel::{ChannelRef, ConversationRef, SenderRef};
use tinychannels_bus::ChannelInboundEnvelope;
#[cfg(test)]
use tinychannels_runtime::context::conversation_history_key_parts;

use super::{
    append_message, ensure_thread, get_messages, ConversationMessage, CreateConversationThread,
};

static CONVERSATION_PERSISTENCE_HANDLE: OnceLock<SubscriptionHandle> = OnceLock::new();
static CONVERSATION_PERSISTENCE_WORKSPACE: OnceLock<Arc<RwLock<PathBuf>>> = OnceLock::new();

const LOG_PREFIX: &str = "[memory:conversations:bus]";

/// Register the long-lived channel conversation persistence subscriber.
///
/// This bridges typed channel events onto the workspace-backed JSONL
/// conversation store so non-web channels persist alongside UI threads.
pub fn register_conversation_persistence_subscriber(workspace_dir: PathBuf) {
    // One subscriber bound to one workspace cannot serve many users: in SaaS
    // it would write every user's channel conversations into the operator's
    // workspace. Profiles keep their threads through their own scoped turns.
    if crate::core::runtime::is_saas() {
        log::info!("{LOG_PREFIX} conversation persistence subscriber not registered in SaaS mode");
        return;
    }
    let workspace = CONVERSATION_PERSISTENCE_WORKSPACE
        .get_or_init(|| Arc::new(RwLock::new(workspace_dir.clone())));
    match workspace.write() {
        Ok(mut guard) => {
            *guard = workspace_dir;
        }
        Err(error) => {
            log::warn!("{LOG_PREFIX} failed to update workspace binding: {error}");
        }
    }

    if CONVERSATION_PERSISTENCE_HANDLE.get().is_some() {
        return;
    }

    match crate::core::bus::BUS.subscribe(Arc::new(ConversationPersistenceSubscriber::new_shared(
        Arc::clone(workspace),
    ))) {
        Some(handle) => {
            let _ = CONVERSATION_PERSISTENCE_HANDLE.set(handle);
        }
        None => {
            log::warn!(
                "{LOG_PREFIX} failed to register conversation persistence subscriber — bus not initialized"
            );
        }
    }
}

/// How many claimed channel turns are remembered. The subscriber sees an
/// event within moments of its publication, so a short FIFO is plenty.
const CLAIMED_TURNS_CAPACITY: usize = 1024;

/// Channel turns whose caller persists them itself (the hosted-channel relay,
/// which writes under the caller's own scope and thread id). Keyed by
/// `(channel, message_id)`, oldest first, at most `capacity` long.
#[derive(Debug)]
struct ClaimedTurns {
    capacity: usize,
    keys: std::collections::VecDeque<(String, String)>,
}

impl ClaimedTurns {
    fn new(capacity: usize) -> Self {
        Self {
            capacity,
            keys: std::collections::VecDeque::new(),
        }
    }

    fn claim(&mut self, channel: &str, message_id: &str) {
        if self.contains(channel, message_id) {
            return;
        }
        if self.keys.len() >= self.capacity {
            self.keys.pop_front();
        }
        self.keys
            .push_back((channel.to_string(), message_id.to_string()));
    }

    fn contains(&self, channel: &str, message_id: &str) -> bool {
        self.keys
            .iter()
            .any(|(c, m)| c == channel && m == message_id)
    }
}

static CLAIMED_TURNS: OnceLock<std::sync::Mutex<ClaimedTurns>> = OnceLock::new();

fn claimed_turns() -> std::sync::MutexGuard<'static, ClaimedTurns> {
    CLAIMED_TURNS
        .get_or_init(|| std::sync::Mutex::new(ClaimedTurns::new(CLAIMED_TURNS_CAPACITY)))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// Mark the channel message `(channel, message_id)` as persisted by its
/// caller, so this subscriber does not mirror it a second time under the
/// listener-derived thread id.
///
/// A no-op in SaaS: the subscriber is never registered there, so nothing
/// would read the claim, and its `(channel, message_id)` key carries no
/// tenant — one profile's claim would otherwise shadow another profile's
/// message with the same platform id. Outside SaaS there is one user, and the
/// subscriber (which runs off the bus, with no caller scope to key on) reads
/// the bare key.
pub(crate) fn claim_channel_turn(channel: &str, message_id: &str) {
    claim_channel_turn_in(crate::core::runtime::is_saas(), channel, message_id);
}

/// [`claim_channel_turn`] as a function of the mode.
pub(crate) fn claim_channel_turn_in(saas: bool, channel: &str, message_id: &str) {
    if saas {
        log::trace!("{LOG_PREFIX} SaaS: channel turn claim skipped channel={channel}");
        return;
    }
    claimed_turns().claim(channel, message_id);
}

/// Whether `(channel, message_id)` was claimed by [`claim_channel_turn`].
pub(crate) fn is_claimed_channel_turn(channel: &str, message_id: &str) -> bool {
    claimed_turns().contains(channel, message_id)
}

pub struct ConversationPersistenceSubscriber {
    workspace_dir: Arc<RwLock<PathBuf>>,
}

impl ConversationPersistenceSubscriber {
    pub fn new(workspace_dir: PathBuf) -> Self {
        Self {
            workspace_dir: Arc::new(RwLock::new(workspace_dir)),
        }
    }

    fn new_shared(workspace_dir: Arc<RwLock<PathBuf>>) -> Self {
        Self { workspace_dir }
    }

    fn workspace_dir_snapshot(&self) -> Result<PathBuf, String> {
        self.workspace_dir
            .read()
            .map(|guard| guard.clone())
            .map_err(|error| format!("workspace binding poisoned: {error}"))
    }
}

#[async_trait]
impl EventHandler<DomainEvent> for ConversationPersistenceSubscriber {
    fn name(&self) -> &str {
        "threads::store::persistence"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["channel"])
    }

    async fn handle(&self, event: &DomainEvent) {
        if let DomainEvent::ChannelMessageReceived {
            channel,
            message_id,
            ..
        }
        | DomainEvent::ChannelMessageProcessed {
            channel,
            message_id,
            ..
        } = event
        {
            if is_claimed_channel_turn(channel, message_id) {
                log::debug!(
                    "{LOG_PREFIX} skipping channel turn persisted by its caller channel={channel}"
                );
                return;
            }
        }
        match event {
            DomainEvent::ChannelMessageReceived {
                channel,
                message_id,
                sender,
                reply_target,
                content,
                thread_ts,
                inbound_envelope,
                workspace_dir,
            } => {
                let my_workspace = match self.workspace_dir_snapshot() {
                    Ok(d) => d,
                    Err(error) => {
                        log::warn!("{LOG_PREFIX} failed to resolve workspace: {error}");
                        return;
                    }
                };
                if *workspace_dir != my_workspace {
                    log::debug!(
                        "{LOG_PREFIX} dropping stale-workspace event \
                         event_ws={} self_ws={}",
                        workspace_dir.display(),
                        my_workspace.display()
                    );
                    return;
                }
                if let Err(error) = persist_channel_turn(
                    &DispatchCtx::host_default(),
                    &my_workspace,
                    ChannelTurnDescriptor {
                        channel,
                        message_id,
                        sender,
                        reply_target,
                        thread_ts: thread_ts.as_deref(),
                        inbound_envelope: inbound_envelope.as_ref(),
                        content,
                        role: "user",
                        success: None,
                        elapsed_ms: None,
                        model_provider: None,
                        model: None,
                        source: "channel_received",
                    },
                )
                .await
                {
                    log::warn!(
                        "{LOG_PREFIX} failed to persist inbound channel message channel={} message_id={} error={}",
                        channel,
                        message_id,
                        error
                    );
                }
            }
            DomainEvent::ChannelMessageProcessed {
                channel,
                message_id,
                sender,
                reply_target,
                thread_ts,
                response,
                provider,
                model,
                elapsed_ms,
                success,
                workspace_dir,
                ..
            } => {
                let my_workspace = match self.workspace_dir_snapshot() {
                    Ok(d) => d,
                    Err(error) => {
                        log::warn!("{LOG_PREFIX} failed to resolve workspace: {error}");
                        return;
                    }
                };
                if *workspace_dir != my_workspace {
                    log::debug!(
                        "{LOG_PREFIX} dropping stale-workspace event \
                         event_ws={} self_ws={}",
                        workspace_dir.display(),
                        my_workspace.display()
                    );
                    return;
                }
                if let Err(error) = persist_channel_turn(
                    &DispatchCtx::host_default(),
                    &my_workspace,
                    ChannelTurnDescriptor {
                        channel,
                        message_id,
                        sender,
                        reply_target,
                        thread_ts: thread_ts.as_deref(),
                        inbound_envelope: None,
                        content: response,
                        role: "assistant",
                        success: Some(*success),
                        elapsed_ms: Some(*elapsed_ms),
                        model_provider: Some(provider),
                        model: Some(model),
                        source: "channel_processed",
                    },
                )
                .await
                {
                    log::warn!(
                        "{LOG_PREFIX} failed to persist processed channel message channel={} message_id={} error={}",
                        channel,
                        message_id,
                        error
                    );
                }
            }
            _ => {}
        }
    }
}

struct ChannelTurnDescriptor<'a> {
    channel: &'a str,
    message_id: &'a str,
    sender: &'a str,
    reply_target: &'a str,
    thread_ts: Option<&'a str>,
    inbound_envelope: Option<&'a ChannelInboundEnvelope>,
    content: &'a str,
    role: &'a str,
    success: Option<bool>,
    elapsed_ms: Option<u64>,
    model_provider: Option<&'a str>,
    model: Option<&'a str>,
    source: &'a str,
}

/// Persist one channel message under the trusted dispatch scope and return the
/// OpenHuman row ID. User rows bind the provider receipt only after the row is
/// durable; assistant rows deliberately remain outside the provider receipt
/// index because no provider delivery receipt is available here.
pub(crate) async fn persist_channel_message(
    ctx: &DispatchCtx,
    workspace_dir: &Path,
    channel: &tinychannels_bus::ChannelRef,
    conversation: &tinychannels_bus::ConversationRef,
    thread_id: &str,
    title: &str,
    provider_message_id: &str,
    role: &str,
    content: &str,
    mut extra_metadata: serde_json::Value,
) -> Result<String, String> {
    if !matches!(role, "user" | "assistant") {
        return Err("channel message role must be user or assistant".into());
    }
    if workspace_dir.as_os_str().is_empty()
        || thread_id.is_empty()
        || provider_message_id.is_empty()
    {
        return Err("channel message scope and provider ID must be non-empty".into());
    }
    if !extra_metadata.is_object() {
        extra_metadata = serde_json::json!({});
    }
    let metadata = extra_metadata.as_object_mut().expect("object established");
    metadata.insert("scope".into(), serde_json::json!("channel"));
    metadata.insert(
        "sourceMessageId".into(),
        serde_json::json!(provider_message_id),
    );
    // Preserve the established channel-row ID contract for retry idempotence;
    // the provider receipt is still bound separately to this persisted row.
    let row_id = format!("{role}:{provider_message_id}");
    let now = chrono::Utc::now().to_rfc3339();
    let workspace = workspace_dir.to_path_buf();
    ensure_thread(
        ctx,
        workspace.clone(),
        CreateConversationThread {
            id: thread_id.to_string(),
            title: title.to_string(),
            created_at: now.clone(),
            parent_thread_id: None,
            labels: Some(vec!["general".to_string()]),
            personality_id: None,
            working_dir: None,
        },
    )?;
    let exists = get_messages(ctx, workspace.clone(), thread_id)?
        .iter()
        .any(|message| message.id == row_id);
    if !exists {
        append_message(
            ctx,
            workspace,
            thread_id,
            ConversationMessage {
                id: row_id.clone(),
                content: content.to_string(),
                message_type: "text".into(),
                extra_metadata: serde_json::Value::Object(metadata.clone()),
                sender: role.to_string(),
                created_at: now,
            },
        )?;
    }
    if role == "user" {
        crate::threads::channel_projection::bind_row(
            ctx,
            channel,
            conversation,
            provider_message_id,
            thread_id,
            &row_id,
        )?;
    }
    Ok(row_id)
}

/// Project the older event fields into the v22 envelope used by the module
/// persistence contract. Match TinyChannels' legacy projection, including its
/// Telegram topic interpretation, while keeping deprecated wire DTOs out of
/// the host call.
fn legacy_descriptor_envelope(descriptor: &ChannelTurnDescriptor<'_>) -> ChannelInboundEnvelope {
    let thread_ts = descriptor
        .thread_ts
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let provider = descriptor.channel.split(':').next().unwrap_or_default();
    let (thread_id, topic_id) = if matches!(provider, "telegram" | "tg") {
        (None, thread_ts)
    } else {
        (thread_ts, None)
    };
    ChannelInboundEnvelope {
        channel: ChannelRef {
            id: descriptor.channel.to_owned(),
            account_id: None,
        },
        message_id: descriptor.message_id.to_owned(),
        conversation: ConversationRef {
            id: descriptor.reply_target.to_owned(),
            thread_id,
            topic_id,
            ..Default::default()
        },
        sender: SenderRef {
            id: descriptor.sender.to_owned(),
            ..Default::default()
        },
        text: descriptor.content.to_owned(),
        ..Default::default()
    }
}

async fn persist_channel_turn(
    _ctx: &DispatchCtx,
    workspace_dir: &Path,
    descriptor: ChannelTurnDescriptor<'_>,
) -> Result<(), String> {
    if let Some(envelope) = descriptor.inbound_envelope {
        if envelope.channel.id != descriptor.channel
            || envelope.message_id != descriptor.message_id
            || envelope.sender.id != descriptor.sender
            || envelope.conversation.id != descriptor.reply_target
        {
            return Err("inbound envelope identity does not match message".into());
        }
    }
    // The fixture exists only where both features compile it (channels/mod.rs).
    #[cfg(all(test, feature = "channels", feature = "modules"))]
    let client = crate::channels::module_fixture::client().await;
    #[cfg(not(all(test, feature = "channels", feature = "modules")))]
    let client = {
        let config = _ctx.load_config().await?;
        if config.workspace_dir != workspace_dir {
            return Err("channel workspace changed".into());
        }
        crate::modules::client(config)
    };
    let prepared: tinychannels_bus::module_inbound::PreparedChannelInbound = client
        .call_confidential(
            "tinychannels",
            tinychannels_bus::methods::PREPARE_CHANNEL_INBOUND,
            (
                tinychannels_bus::module_inbound::PrepareChannelInboundRequest {
                    namespace: "main".into(),
                    preparation_mode: Default::default(),
                    session_policy: Default::default(),
                    source: tinychannels_bus::module_inbound::ChannelInboundSource::Envelope {
                        envelope: descriptor
                            .inbound_envelope
                            .cloned()
                            .unwrap_or_else(|| legacy_descriptor_envelope(&descriptor)),
                        // The old host-side projection used timestamp 0; keep that value when
                        // reconstructing the module's prepared legacy message.
                        timestamp: 0,
                    },
                },
            ),
        )
        .await
        .map_err(|error| error.to_string())?;
    let thread_id = format!("channel:{}", prepared.history_key);
    let history_key_ignores_thread =
        tinychannels_bus::capabilities::capabilities_for(descriptor.channel)
            .history_key_ignores_thread;
    let session_key = descriptor.inbound_envelope.map(|_| prepared.session_key);
    let title = channel_thread_title(
        descriptor.channel,
        descriptor.sender,
        descriptor.reply_target,
        descriptor.thread_ts,
        history_key_ignores_thread,
    );
    let created_at = Utc::now().to_rfc3339();

    ensure_thread(
        &DispatchCtx::host_default(),
        workspace_dir.to_path_buf(),
        CreateConversationThread {
            id: thread_id.clone(),
            title,
            created_at: created_at.clone(),
            parent_thread_id: None,
            labels: Some(vec!["general".to_string()]),
            personality_id: None,
            working_dir: None,
        },
    )?;

    let persisted_message_id = format!("{}:{}", descriptor.role, descriptor.message_id);
    if get_messages(
        &DispatchCtx::host_default(),
        workspace_dir.to_path_buf(),
        &thread_id,
    )?
    .iter()
    .any(|message| message.id == persisted_message_id)
    {
        log::debug!(
            "{LOG_PREFIX} skipping duplicate persisted turn thread_id={} message_id={}",
            thread_id,
            persisted_message_id
        );
        return Ok(());
    }

    append_message(
        &DispatchCtx::host_default(),
        workspace_dir.to_path_buf(),
        &thread_id,
        ConversationMessage {
            id: persisted_message_id.clone(),
            content: descriptor.content.to_string(),
            message_type: "text".to_string(),
            extra_metadata: json!({
                "scope": "channel",
                "channel": descriptor.channel,
                "channelSender": descriptor.sender,
                "replyTarget": descriptor.reply_target,
                "threadTs": descriptor.thread_ts,
                "tinychannelsSessionKey": session_key,
                "sourceEvent": descriptor.source,
                "success": descriptor.success,
                "elapsedMs": descriptor.elapsed_ms,
                "modelProvider": descriptor.model_provider,
                "model": descriptor.model,
                "sourceMessageId": descriptor.message_id,
            }),
            sender: descriptor.role.to_string(),
            created_at,
        },
    )?;

    log::debug!(
        "{LOG_PREFIX} persisted channel turn thread_id={} message_id={} role={}",
        thread_id,
        persisted_message_id,
        descriptor.role
    );
    Ok(())
}

#[cfg(test)]
fn persisted_channel_thread_id(
    channel: &str,
    sender: &str,
    reply_target: &str,
    thread_ts: Option<&str>,
) -> String {
    let key = conversation_history_key_parts(channel, sender, reply_target, thread_ts);
    format!("channel:{key}")
}

fn channel_thread_title(
    channel: &str,
    sender: &str,
    reply_target: &str,
    thread_ts: Option<&str>,
    history_key_ignores_thread: bool,
) -> String {
    match thread_ts.and_then(non_empty_trimmed) {
        // Providers whose thread_ts is only a reply target (Telegram topics)
        // share one conversation, so the title leaves it out too.
        Some(thread_ts) if !history_key_ignores_thread => {
            format!("{channel} · {sender} · {reply_target} · thread {thread_ts}")
        }
        _ => format!("{channel} · {sender} · {reply_target}"),
    }
}

fn non_empty_trimmed(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

#[cfg(test)]
#[path = "bus_tests.rs"]
mod tests;
