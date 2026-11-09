//! [`ProfileHandle`]: one open profile, and the calls a user makes on it.

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use openhuman_core::core::envelope::ApiEnvelope;
use openhuman_core::core::runtime::CoreContext;
use openhuman_core::profiles::Profile;
use openhuman_core::threads::{
    AppendConversationMessageRequest, ConversationMessagesRequest, ConversationMessagesResponse,
    ConversationThreadsListResponse,
};
use openhuman_core::web_chat::WebChannelEvent;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

#[cfg(feature = "channels")]
use super::RelayMessage;
use super::{
    ConversationMessageRecord, ConversationThreadSummary, Inner, ProfileError, ProfileId,
    ScopedWebEvent,
};
use crate::call::call_in;
use crate::error::CoreError;

const THREADS_UPSERT: &str = "openhuman.threads_upsert";
const THREADS_MESSAGE_APPEND: &str = "openhuman.threads_message_append";
const THREADS_LIST: &str = "openhuman.threads_list";
const THREADS_MESSAGES_LIST: &str = "openhuman.threads_messages_list";
const CHANNEL_WEB_CHAT: &str = "openhuman.channel_web_chat";
#[cfg(feature = "channels")]
const CHANNEL_RELAY_INBOUND: &str = "openhuman.channel_relay_inbound";

/// An open profile. Every call runs under the profile's own context, as a
/// gateway request for that user would.
///
/// Holding a handle (or any clone of it) keeps the profile **in use**: it is
/// never evicted or released from under you. Drop every handle to let it go
/// idle.
#[derive(Clone)]
pub struct ProfileHandle {
    runtime: Arc<Inner>,
    profile: Arc<Profile>,
    client_id: String,
}

impl std::fmt::Debug for ProfileHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProfileHandle")
            .field("id", &self.profile.id)
            .finish_non_exhaustive()
    }
}

/// The final answer of one [`ProfileHandle::chat`] turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatReply {
    /// The thread the turn ran on.
    pub thread_id: String,
    /// The turn's request id, as its events carry it.
    pub request_id: String,
    /// The assistant's full reply.
    pub text: String,
}

/// What `channel_relay_inbound` answered: the message was accepted (its turn
/// runs in the background) or was a duplicate of one already recorded.
#[cfg(feature = "channels")]
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RelayAccepted {
    /// Whether a turn was started for it.
    #[serde(default)]
    pub accepted: bool,
    /// The message id was already recorded on its thread; nothing ran.
    #[serde(default)]
    pub duplicate: bool,
    /// The `channel:` thread the message is on.
    pub thread_id: String,
    /// The request id the reply events carry (absent for a duplicate).
    #[serde(default)]
    pub request_id: Option<String>,
    /// The client id the reply events are addressed to.
    #[serde(default)]
    pub client_id: Option<String>,
}

/// This profile's live events: chat progress, `chat_done` / `chat_error`,
/// and the `channel_outbound` replies a gateway delivers to hosted chat
/// platforms. Only events stamped with this profile get through, the same
/// filter the gateway's `/events` stream applies.
pub struct ProfileEvents {
    profile: String,
    rx: broadcast::Receiver<ScopedWebEvent>,
}

impl ProfileEvents {
    /// The next event of this profile, or `None` once the bus is gone.
    /// Events the receiver fell too far behind on are skipped.
    pub async fn recv(&mut self) -> Option<ScopedWebEvent> {
        loop {
            match self.rx.recv().await {
                Ok(event) if event.belongs_to_profile(&self.profile) => return Some(event),
                Ok(_) => {}
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    log::warn!(
                        "[embed][profiles] event receiver of profile={} lagged by {skipped}",
                        self.profile
                    );
                }
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    }
}

#[derive(Serialize)]
struct WebChatParams<'a> {
    client_id: &'a str,
    thread_id: &'a str,
    message: &'a str,
}

/// `threads.upsert` params. The controller's schema lists only these three
/// as required and refuses an explicit `null` for the optional ones.
#[derive(Serialize)]
struct UpsertThread<'a> {
    id: &'a str,
    title: &'a str,
    created_at: String,
}

#[derive(Deserialize)]
struct WebChatAccepted {
    request_id: String,
}

impl ProfileHandle {
    pub(super) fn new(runtime: Arc<Inner>, profile: Arc<Profile>) -> Self {
        let client_id = format!("embed-{}", uuid::Uuid::new_v4().simple());
        Self {
            runtime,
            profile,
            client_id,
        }
    }

    /// The profile's id.
    pub fn id(&self) -> &ProfileId {
        &self.profile.id
    }

    /// The profile's workspace (threads, memory, transcripts, ledgers).
    pub fn workspace_dir(&self) -> &Path {
        &self.profile.layout.workspace_dir
    }

    /// The client id this handle's web-chat turns are addressed to.
    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    /// The context every call on this handle runs under.
    pub fn context(&self) -> &Arc<CoreContext> {
        self.profile.context()
    }

    /// Run `fut` as this profile: anything the core does inside it resolves
    /// this user's config, workspace, storage scope and policy.
    pub async fn scope<F: Future>(&self, fut: F) -> F::Output {
        CoreContext::scope(Arc::clone(self.profile.context()), fut).await
    }

    /// Dispatch an RPC `method` as this user. Only the reviewed user surface
    /// (`profiles::surface::USER_METHODS`) is reachable; anything else
    /// answers as an unknown method, exactly as through the gateway.
    pub async fn call<P, R>(&self, method: &'static str, params: P) -> Result<R, CoreError>
    where
        P: Serialize,
        R: DeserializeOwned,
    {
        call_in(
            &self.runtime.core,
            Arc::clone(self.profile.context()),
            method,
            params,
        )
        .await
    }

    /// Subscribe to this profile's events. Subscribe before starting the
    /// work whose events you want: earlier events are not replayed.
    pub fn events(&self) -> ProfileEvents {
        ProfileEvents {
            profile: self.profile.id.to_string(),
            rx: openhuman_core::web_chat::subscribe_web_channel_events(),
        }
    }

    /// Send `text` on `thread_id` (created if it does not exist yet) and wait
    /// for the turn's final reply. A thread id is unique per profile: two
    /// users' `t1` are two threads.
    ///
    /// Like the web app, it records the user's message on the thread itself
    /// (`threads_message_append`) before starting the turn; the core records
    /// the reply.
    ///
    /// Waits for as long as the turn runs; wrap it in
    /// `tokio::time::timeout` to bound it. For live progress, read
    /// [`events`](Self::events) alongside.
    pub async fn chat(&self, thread_id: &str, text: &str) -> Result<ChatReply, ProfileError> {
        self.ensure_thread(thread_id).await?;
        self.append_user_message(thread_id, text).await?;
        let mut events = self.events();
        let accepted: WebChatAccepted = self
            .call(
                CHANNEL_WEB_CHAT,
                WebChatParams {
                    client_id: &self.client_id,
                    thread_id,
                    message: text,
                },
            )
            .await?;
        let request_id = accepted.request_id;
        log::debug!(
            "[embed][profiles] chat started profile={} thread_id={thread_id} request_id={request_id}",
            self.profile.id
        );
        loop {
            let Some(event) = events.recv().await else {
                return Err(ProfileError::EventsClosed);
            };
            let context = event.context();
            if context.thread_id != thread_id || context.request_id != request_id {
                continue;
            }
            match event.wire_event() {
                WebChannelEvent::ChatDone { full_response, .. } => {
                    log::debug!(
                        "[embed][profiles] chat done profile={} request_id={request_id}",
                        self.profile.id
                    );
                    return Ok(ChatReply {
                        thread_id: thread_id.to_string(),
                        request_id,
                        text: chat_reply_text(full_response.clone()),
                    });
                }
                WebChannelEvent::ChatError {
                    message,
                    error_type,
                    ..
                } => {
                    log::debug!(
                        "[embed][profiles] chat failed profile={} request_id={request_id} error_type={:?}",
                        self.profile.id,
                        error_type
                    );
                    return Err(ProfileError::Turn {
                        message: chat_failure_message(message.clone()),
                        error_type: error_type.clone(),
                    });
                }
                WebChannelEvent::ChatCancelled { .. } => {
                    return Err(ProfileError::Cancelled);
                }
                _ => {}
            }
        }
    }

    /// Hand the core a message the host received for this user on a hosted
    /// chat platform (Telegram, iMessage, ...). It lands on the user's
    /// `channel:<channel>/<sender>/<chat>` thread and its turn runs in the
    /// background; the reply arrives on [`events`](Self::events) as a
    /// `channel_outbound` event for the host to deliver. A message id already
    /// recorded is acknowledged as a duplicate and runs nothing.
    #[cfg(feature = "channels")]
    pub async fn relay_inbound(
        &self,
        message: RelayMessage,
    ) -> Result<RelayAccepted, ProfileError> {
        let accepted: RelayAccepted = self.call(CHANNEL_RELAY_INBOUND, message).await?;
        log::debug!(
            "[embed][profiles] relay profile={} accepted={} duplicate={}",
            self.profile.id,
            accepted.accepted,
            accepted.duplicate
        );
        Ok(accepted)
    }

    /// The profile's threads, web and relayed alike.
    pub async fn threads(&self) -> Result<Vec<ConversationThreadSummary>, ProfileError> {
        let listed: ApiEnvelope<ConversationThreadsListResponse> =
            self.call(THREADS_LIST, serde_json::json!({})).await?;
        Ok(listed.data.map(|data| data.threads).unwrap_or_default())
    }

    /// The messages on this profile's `thread_id`.
    pub async fn messages(
        &self,
        thread_id: &str,
    ) -> Result<Vec<ConversationMessageRecord>, ProfileError> {
        let listed: ApiEnvelope<ConversationMessagesResponse> = self
            .call(
                THREADS_MESSAGES_LIST,
                ConversationMessagesRequest {
                    thread_id: thread_id.to_string(),
                },
            )
            .await?;
        Ok(listed.data.map(|data| data.messages).unwrap_or_default())
    }

    /// Record `text` as the user's message on `thread_id`.
    async fn append_user_message(&self, thread_id: &str, text: &str) -> Result<(), ProfileError> {
        let _appended: ApiEnvelope<ConversationMessageRecord> = self
            .call(
                THREADS_MESSAGE_APPEND,
                AppendConversationMessageRequest {
                    thread_id: thread_id.to_string(),
                    message: ConversationMessageRecord {
                        id: format!("msg_{}", uuid::Uuid::new_v4()),
                        content: text.to_string(),
                        attachments: Vec::new(),
                        message_type: "text".to_string(),
                        extra_metadata: serde_json::json!({}),
                        sender: "user".to_string(),
                        created_at: chrono::Utc::now().to_rfc3339(),
                    },
                    attachments: Vec::new(),
                },
            )
            .await?;
        Ok(())
    }

    /// Create `thread_id` on this profile unless it exists (`threads_upsert`
    /// keeps an existing thread as it is).
    async fn ensure_thread(&self, thread_id: &str) -> Result<(), ProfileError> {
        let _summary: ApiEnvelope<ConversationThreadSummary> = self
            .call(
                THREADS_UPSERT,
                UpsertThread {
                    id: thread_id,
                    title: thread_id,
                    created_at: chrono::Utc::now().to_rfc3339(),
                },
            )
            .await?;
        Ok(())
    }
}

fn chat_reply_text(full_response: Option<String>) -> String {
    full_response.unwrap_or_default()
}

fn chat_failure_message(message: Option<String>) -> String {
    message.unwrap_or_else(|| "The turn failed without an error message.".to_string())
}

#[cfg(test)]
#[path = "handle_tests.rs"]
mod tests;
