//! Host-owned approval routing and turn events over the channel module.
use crate::channels::capabilities::capabilities_for;
use crate::channels::runtime::ChannelHandle;
use crate::channels::SendMessage;
use crate::core::events::DomainEvent;
use async_trait::async_trait;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tinybus::module::{ModuleClient, ModuleObjectClient};
use tinybus::EventHandler;
use tinychannels_bus::approvals::ReplyContext;
use tinychannels_bus::module_inbound::ChannelInboundPreparationMode;

/// Captures the account configuration and reply addresses used by approvals.
pub(crate) struct ChannelApprovalSurfaceSubscriber {
    channels: Arc<HashMap<String, Arc<ChannelHandle>>>,
    replies: Mutex<HashMap<String, ReplyContext>>,
    config: Option<Arc<crate::config::Config>>,
    cancellation: tokio_util::sync::CancellationToken,
}
impl ChannelApprovalSurfaceSubscriber {
    pub(crate) fn with_config(
        channels: Arc<HashMap<String, Arc<ChannelHandle>>>,
        config: Arc<crate::config::Config>,
    ) -> Self {
        Self {
            channels,
            replies: Mutex::new(HashMap::new()),
            config: Some(config),
            cancellation: super::super::runtime::session::channel_session(),
        }
    }
    #[cfg(test)]
    pub(crate) fn new(channels: Arc<HashMap<String, Arc<ChannelHandle>>>) -> Self {
        Self {
            channels,
            replies: Mutex::new(HashMap::new()),
            config: None,
            cancellation: tokio_util::sync::CancellationToken::new(),
        }
    }
    async fn client(&self) -> ModuleClient {
        #[cfg(test)]
        if self.config.is_none() {
            return crate::channels::module_fixture::client().await;
        }
        crate::modules::client(self.config.as_deref().cloned().unwrap_or_default())
    }
}
#[async_trait]
impl EventHandler<DomainEvent> for ChannelApprovalSurfaceSubscriber {
    fn name(&self) -> &str {
        "channels::approval_surface"
    }
    fn domains(&self) -> Option<&[&str]> {
        Some(&["channel", "approval"])
    }
    async fn handle(&self, event: &DomainEvent) {
        if self.cancellation.is_cancelled() {
            return;
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
                if !capabilities_for(channel).chat_approvals {
                    return;
                }
                if self
                    .config
                    .as_ref()
                    .is_some_and(|config| config.workspace_dir != *workspace_dir)
                {
                    return;
                }
                let prepared = crate::modules::channel_controller::prepare_inbound(
                    &self.client().await,
                    tinychannels_bus::module_inbound::PrepareChannelInboundRequest {
                        namespace: "main".into(),
                        preparation_mode: ChannelInboundPreparationMode::Typed,
                        session_policy: Default::default(),
                        source: tinychannels_bus::module_inbound::ChannelInboundSource::Envelope {
                            envelope: inbound_envelope.clone().unwrap_or_else(|| {
                                tinychannels_bus::ChannelInboundEnvelope {
                                    channel: tinychannels_bus::channel::ChannelRef {
                                        id: channel.clone(),
                                        account_id: None,
                                    },
                                    message_id: message_id.clone(),
                                    conversation: tinychannels_bus::channel::ConversationRef {
                                        id: reply_target.clone(),
                                        thread_id: thread_ts.clone(),
                                        ..Default::default()
                                    },
                                    sender: tinychannels_bus::channel::SenderRef {
                                        id: sender.clone(),
                                        ..Default::default()
                                    },
                                    text: content.clone(),
                                    ..Default::default()
                                }
                            }),
                            timestamp: 0,
                        },
                    },
                )
                .await;
                if let Ok(prepared) = prepared {
                    self.replies
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .insert(
                            prepared.history_key,
                            ReplyContext {
                                reply_target: reply_target.clone(),
                                thread_ts: thread_ts.clone(),
                            },
                        );
                }
            }
            DomainEvent::ApprovalRequested {
                tool_name,
                action_summary,
                thread_id: Some(thread_id),
                client_id: Some(channel_name),
                ..
            } => {
                if !capabilities_for(channel_name).chat_approvals {
                    return;
                }
                let reply = self
                    .replies
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .get(thread_id)
                    .cloned();
                let (Some(reply), Some(channel)) = (reply, self.channels.get(channel_name)) else {
                    return;
                };
                use tinychannels_bus::module_control::{
                    ChannelControlRequest, ChannelControlResult,
                };
                let response = self
                    .client()
                    .await
                    .call_confidential::<ChannelControlResult>(
                        "tinychannels",
                        tinychannels_bus::methods::PREPARE_CHANNEL_CONTROL,
                        (ChannelControlRequest::Approval {
                            tool_name: tool_name.clone(),
                            action_summary: action_summary.clone(),
                        },),
                    )
                    .await;
                match response {
                    Ok(ChannelControlResult::Approval(body)) => {
                        if self.cancellation.is_cancelled() {
                            return;
                        }
                        let _ = channel
                            .send_with_outbound_intent(
                                &SendMessage::new(body, &reply.reply_target)
                                    .in_thread(reply.thread_ts),
                            )
                            .await;
                    }
                    Ok(_) => {
                        crate::modules::report::malformed_reply("tinychannels");
                    }
                    Err(_) => {}
                }
            }
            _ => {}
        }
    }
}

/// Projects host turn lifecycle facts into module-owned remote status storage.
///
/// Turn state is written through the configured module object that owns the
/// running providers. A process-wide client reaches a callback-only runtime
/// with no running provider state, which would not report `remote_control` and
/// so would silently drop every mark.
pub struct ChannelTurnStateSubscriber {
    workspace_dir: PathBuf,
    objects: Option<Arc<HashMap<String, ModuleObjectClient>>>,
    cancellation: tokio_util::sync::CancellationToken,
    #[cfg(test)]
    test_marks: Option<TurnStateMarks>,
}

/// Test record of each turn-state mark: channel, reply target, busy flag, and
/// whether the module accepted the dispatch.
#[cfg(test)]
type TurnStateMarks = Arc<Mutex<Vec<(String, String, bool, bool)>>>;
impl ChannelTurnStateSubscriber {
    pub fn with_objects(
        workspace_dir: PathBuf,
        objects: Arc<HashMap<String, ModuleObjectClient>>,
    ) -> Self {
        Self {
            workspace_dir,
            objects: Some(objects),
            cancellation: super::super::runtime::session::channel_session(),
            #[cfg(test)]
            test_marks: None,
        }
    }
    #[cfg(test)]
    pub fn new(workspace_dir: PathBuf) -> Self {
        Self {
            workspace_dir,
            objects: None,
            cancellation: tokio_util::sync::CancellationToken::new(),
            #[cfg(test)]
            test_marks: None,
        }
    }
    async fn mark(&self, channel: &str, reply_target: &str, event_ws: &PathBuf, busy: bool) {
        if self.cancellation.is_cancelled()
            || !capabilities_for(channel).remote_control
            || *event_ws != self.workspace_dir
        {
            return;
        }
        let args = (tinychannels_bus::module_remote::RemoteOperation::MarkTurn {
            workspace_dir: self.workspace_dir.to_string_lossy().into_owned(),
            channel: channel.into(),
            reply_target: reply_target.into(),
            busy,
        },);
        let result: Result<tinychannels_bus::module_remote::RemoteOperationResult, _> =
            match &self.objects {
                Some(objects) => {
                    let Some(object) = objects
                        .get(channel)
                        .or_else(|| objects.get(tinychannels_bus::provider_id(channel)))
                    else {
                        tracing::debug!(
                            channel,
                            "[channels] turn state skipped: no configured module object"
                        );
                        return;
                    };
                    object
                        .call_confidential(tinychannels_bus::methods::EXECUTE_REMOTE, args)
                        .await
                }
                None => {
                    #[cfg(test)]
                    {
                        crate::channels::module_fixture::client()
                            .await
                            .call_confidential(
                                "tinychannels",
                                tinychannels_bus::methods::EXECUTE_REMOTE,
                                args,
                            )
                            .await
                    }
                    #[cfg(not(test))]
                    {
                        return;
                    }
                }
            };
        #[cfg(test)]
        if let Some(marks) = &self.test_marks {
            marks
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .push((
                    channel.to_owned(),
                    reply_target.to_owned(),
                    busy,
                    result.is_ok(),
                ));
        }
        if result.is_err() {
            tracing::debug!("channel turn status unavailable");
        }
    }
}
#[async_trait]
impl EventHandler<DomainEvent> for ChannelTurnStateSubscriber {
    fn name(&self) -> &str {
        "channels::turn_state"
    }
    fn domains(&self) -> Option<&[&str]> {
        Some(&["channel"])
    }
    async fn handle(&self, event: &DomainEvent) {
        match event {
            DomainEvent::ChannelMessageReceived {
                channel,
                reply_target,
                workspace_dir,
                ..
            } => self.mark(channel, reply_target, workspace_dir, true).await,
            DomainEvent::ChannelMessageProcessed {
                channel,
                reply_target,
                workspace_dir,
                ..
            } => self.mark(channel, reply_target, workspace_dir, false).await,
            _ => {}
        }
    }
}
#[cfg(test)]
#[path = "channel_events_tests.rs"]
mod tests;
