//! OpenHuman's [`RemoteControlHost`]: the host state behind `/status`,
//! `/sessions` and `/new` on every chat channel.
//!
//! Command parsing, rendering and the chat → thread bindings live in
//! `tinychannels::remote`. This adapter answers from the channel runtime
//! context (model routes, in-memory history), the conversation store and the
//! web-chat session cache.

use async_trait::async_trait;
use tinychannels::remote::{NewRemoteThread, RemoteControlHost, RemoteRoute, RemoteThreadSummary};

use crate::channels::context::{
    clear_sender_history, ChannelRouteSelection, ChannelRuntimeContext,
};
use crate::threads::store::{self as conversations, CreateConversationThread};

/// Remote-control host state for one command, borrowed from the runtime.
pub(crate) struct RuntimeRemoteControl<'a> {
    pub(crate) ctx: &'a ChannelRuntimeContext,
}

#[async_trait]
impl RemoteControlHost for RuntimeRemoteControl<'_> {
    fn route(&self, sender_key: &str) -> RemoteRoute {
        let selection = self
            .ctx
            .route_overrides
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(sender_key)
            .cloned()
            .unwrap_or_else(|| ChannelRouteSelection {
                provider: self.ctx.default_provider.as_str().to_string(),
                model: self.ctx.model.as_str().to_string(),
            });
        RemoteRoute {
            provider: selection.provider,
            model: selection.model,
        }
    }

    fn history_len(&self, sender_key: &str) -> usize {
        self.ctx
            .conversation_histories
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(sender_key)
            .map(Vec::len)
            .unwrap_or(0)
    }

    fn clear_history(&self, sender_key: &str) {
        clear_sender_history(self.ctx, sender_key);
    }

    async fn list_threads(&self) -> anyhow::Result<Vec<RemoteThreadSummary>> {
        let threads = conversations::list_threads(self.ctx.workspace_dir.to_path_buf())
            .map_err(anyhow::Error::msg)?;
        Ok(threads
            .into_iter()
            .map(|thread| RemoteThreadSummary {
                id: thread.id,
                title: thread.title,
                message_count: thread.message_count,
                last_message_at: thread.last_message_at,
            })
            .collect())
    }

    async fn create_thread(&self, thread: NewRemoteThread) -> anyhow::Result<()> {
        conversations::ensure_thread(
            self.ctx.workspace_dir.to_path_buf(),
            CreateConversationThread {
                id: thread.id,
                title: thread.title,
                created_at: thread.created_at,
                parent_thread_id: None,
                labels: Some(thread.labels),
                personality_id: None,
            },
        )
        .map(drop)
        .map_err(anyhow::Error::msg)
    }

    async fn invalidate_thread(&self, thread_id: &str) {
        crate::web_chat::invalidate_thread_sessions(thread_id).await;
    }
}
