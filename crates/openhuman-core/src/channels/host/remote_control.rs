//! Captured host conversation and session state supplied to the channel module.

use crate::channels::context::ChannelRuntimeContext;
use crate::core::runtime::DispatchCtx;
use crate::threads::store::{self as conversations, CreateConversationThread};

pub(crate) struct RemoteResetContext {
    pub(crate) scope: tinychannels_bus::module_remote::RemoteSessionScope,
    /// Host-derived tenant-scoped identity used by TinyAgents. This is kept
    /// separate from the module's opaque route key so remote commands cannot
    /// choose or broaden session identity.
    pub(crate) durable_session_key: String,
    pub(crate) message: tinychannels_bus::ChannelMessage,
    pub(crate) history_key: String,
}

/// Serves product-owned conversation state to the captured module runtime.
pub(crate) async fn dispatch_callback(
    dispatch: &DispatchCtx,
    ctx: &ChannelRuntimeContext,
    tenant: &crate::core::runtime::Tenant,
    cancellation: &tokio_util::sync::CancellationToken,
    reset_context: Option<&RemoteResetContext>,
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
        Request::ClearHistory { session_scope } => {
            if cancellation.is_cancelled() {
                return Err(tinybus::Error::failed("channel session ended"));
            }
            let Some(reset) = reset_context else {
                return Err(tinybus::Error::failed(
                    "remote reset has no captured ingress scope",
                ));
            };
            if session_scope != reset.scope
                || session_scope.session_key.trim().is_empty()
                || session_scope.conversation.id.trim().is_empty()
            {
                return Err(tinybus::Error::failed(
                    "remote reset scope does not match captured ingress",
                ));
            }
            let account = session_scope
                .channel
                .account_id
                .as_deref()
                .ok_or_else(|| tinybus::Error::failed("remote reset account is unavailable"))?;
            let configured = ctx.channels_by_name.get(account).is_some_and(|channel| {
                tinychannels_bus::provider_id(channel.name()) == session_scope.channel.id
            });
            if !configured {
                return Err(tinybus::Error::failed(
                    "remote reset account is not configured",
                ));
            }
            crate::channels::runtime::dispatch::reset_durable_channel_session(
                ctx,
                &reset.message,
                &reset.durable_session_key,
                &reset.history_key,
            )
            .await
            .map_err(|_| tinybus::Error::failed("durable conversation reset failed"))?;
            if cancellation.is_cancelled() {
                return Err(tinybus::Error::failed("channel session ended"));
            }
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
