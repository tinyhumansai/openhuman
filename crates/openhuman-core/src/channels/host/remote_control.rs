//! Captured host conversation and session state supplied to the channel module.

use crate::channels::context::{clear_sender_history, ChannelRuntimeContext};
use crate::core::runtime::DispatchCtx;
use crate::threads::store::{self as conversations, CreateConversationThread};

/// Serves product-owned conversation state to the captured module runtime.
pub(crate) async fn dispatch_callback(
    dispatch: &DispatchCtx,
    ctx: &ChannelRuntimeContext,
    tenant: &crate::core::runtime::Tenant,
    cancellation: &tokio_util::sync::CancellationToken,
    request: tinychannels_bus::module_remote::RemoteHostRequest,
) -> tinybus::Result<tinychannels_bus::module_remote::RemoteHostResult> {
    use tinychannels_bus::module_remote::{
        RemoteHostRequest as Request, RemoteHostResult as Response,
    };
    if cancellation.is_cancelled() {
        return Err(tinybus::Error::failed("channel session ended"));
    }
    match request {
        Request::ListThreads => {
            let threads = conversations::list_threads(dispatch, ctx.workspace_dir.to_path_buf())
                .map_err(|_| tinybus::Error::failed("conversation state unavailable"))?;
            Ok(Response::Threads(
                threads
                    .into_iter()
                    .map(
                        |thread| tinychannels_bus::module_remote::RemoteThreadSummary {
                            id: thread.id,
                            title: thread.title,
                            message_count: thread.message_count,
                            last_message_at: thread.last_message_at,
                        },
                    )
                    .collect(),
            ))
        }
        Request::CreateThread(thread) => {
            if cancellation.is_cancelled() {
                return Err(tinybus::Error::failed("channel session ended"));
            }
            conversations::ensure_thread(
                dispatch,
                ctx.workspace_dir.to_path_buf(),
                CreateConversationThread {
                    id: thread.id,
                    title: thread.title,
                    created_at: thread.created_at,
                    parent_thread_id: None,
                    labels: Some(thread.labels),
                    personality_id: None,
                    working_dir: None,
                },
            )
            .map_err(|_| tinybus::Error::failed("conversation state unavailable"))?;
            Ok(Response::Completed)
        }
        Request::ClearHistory { sender_key } => {
            if cancellation.is_cancelled() {
                return Err(tinybus::Error::failed("channel session ended"));
            }
            clear_sender_history(&ctx.conversation_histories, &sender_key);
            Ok(Response::Completed)
        }
        Request::InvalidateThread { thread_id } => {
            let _removed = crate::web_chat::invalidate_thread_sessions_for_tenant(
                dispatch,
                tenant,
                &thread_id,
                cancellation,
            )
            .await;
            if cancellation.is_cancelled() {
                return Err(tinybus::Error::failed("channel session ended"));
            }
            Ok(Response::Completed)
        }
    }
}
