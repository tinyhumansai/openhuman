//! Per-sender routing and runtime command handling.

use super::context::{ChannelRouteSelection, ChannelRuntimeContext};
use super::runtime::ChannelHandle;
use super::SendMessage;
use std::sync::Arc;
use tinychannels_bus as traits;
use tinychannels_bus::module_control::{
    ChannelControlRequest, ChannelControlResult, ProviderDescriptor,
};

fn provider_descriptors(config: &crate::config::Config) -> Vec<ProviderDescriptor> {
    config
        .cloud_providers
        .iter()
        .map(|provider| ProviderDescriptor {
            name: provider.slug.clone(),
            aliases: if crate::config::schema::cloud_providers::is_hosted_cloud_identity(
                &provider.slug,
            ) || crate::config::schema::cloud_providers::is_hosted_auth_style(
                provider.auth_style,
            ) {
                vec!["backend".to_string(), "openhuman-backend".to_string()]
            } else {
                Vec::new()
            },
        })
        .collect()
}

#[cfg(test)]
pub(crate) fn register_hosted_route_test_policy() {
    use crate::config::schema::cloud_providers::AuthStyle;
    crate::config::schema::cloud_providers::register_hosted_cloud_provider_policy(
        Arc::new(|slug| slug == "openhuman"),
        Arc::new(|style| style == AuthStyle::OpenhumanJwt),
    );
}

fn default_route_selection(ctx: &ChannelRuntimeContext) -> ChannelRouteSelection {
    ChannelRouteSelection {
        provider: ctx.default_provider.as_str().to_string(),
        model: ctx.model.as_str().to_string(),
    }
}

pub(crate) fn get_route_selection(
    ctx: &ChannelRuntimeContext,
    sender_key: &str,
) -> ChannelRouteSelection {
    ctx.route_overrides
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(sender_key)
        .cloned()
        .unwrap_or_else(|| default_route_selection(ctx))
}

fn set_route_selection(ctx: &ChannelRuntimeContext, sender_key: &str, next: ChannelRouteSelection) {
    let default_route = default_route_selection(ctx);
    let mut routes = ctx
        .route_overrides
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if next == default_route {
        routes.remove(sender_key);
    } else {
        routes.insert(sender_key.to_string(), next);
    }
}

pub(crate) async fn get_or_create_turn_model_source(
    ctx: &ChannelRuntimeContext,
    provider_name: &str,
) -> anyhow::Result<crate::agent::tinyagents::TurnModelSource> {
    if provider_name == ctx.default_provider.as_str() {
        return ctx.turn_model_source.as_ref().cloned().ok_or_else(|| {
            anyhow::anyhow!("no injected channel model source for '{provider_name}'")
        });
    }

    if let Some(existing) = ctx
        .turn_model_source_cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(provider_name)
        .cloned()
    {
        return Ok(existing);
    }

    anyhow::bail!(
        "no injected channel model source for '{provider_name}'; production routes use crate-native model sources"
    )
}

struct RouteCallbacks {
    context: ChannelRuntimeContext,
    core_context: Option<Arc<crate::core::runtime::CoreContext>>,
    tenant: crate::core::runtime::Tenant,
    cancellation: tokio_util::sync::CancellationToken,
    reset_context: Option<crate::channels::host::remote_control::RemoteResetContext>,
}

impl RouteCallbacks {
    fn new(
        context: ChannelRuntimeContext,
        cancellation: tokio_util::sync::CancellationToken,
        reset_context: Option<crate::channels::host::remote_control::RemoteResetContext>,
    ) -> Self {
        let ctx = context.dispatch_ctx();
        Self {
            core_context: ctx.resolved(),
            tenant: crate::core::runtime::tenant::isolated_unless_tenant(
                ctx.tenant(),
                "channel_remote_control",
            ),
            context,
            cancellation,
            reset_context,
        }
    }
}

#[tinybus::interface(name = "ai.tinyhumans.tinychannels.ChannelsHost")]
impl RouteCallbacks {
    async fn dispatch_controller_backend(
        &self,
        request: tinychannels_bus::ChannelBackendRequest,
    ) -> tinybus::Result<tinychannels_bus::ChannelBackendResult> {
        match request {
            tinychannels_bus::ChannelBackendRequest::RemoteControl(request) => {
                // The captured context is the carrier; without one the host's
                // default context (none in SaaS, which fails closed).
                let dispatch_ctx = match &self.core_context {
                    Some(core) => crate::core::runtime::DispatchCtx::new(Arc::clone(core)),
                    None => crate::core::runtime::DispatchCtx::host_default(),
                };
                let dispatch = async {
                    tokio::select! {
                        biased;
                        _ = self.cancellation.cancelled() => {
                            Err(tinybus::Error::failed("channel session ended"))
                        }
                        result = super::host::remote_control::dispatch_callback(
                            &dispatch_ctx,
                            &self.context,
                            &self.tenant,
                            &self.cancellation,
                            self.reset_context.as_ref(),
                            request,
                        ) => result.map(tinychannels_bus::ChannelBackendResult::RemoteControl),
                    }
                };
                match &self.core_context {
                    Some(core_context) => {
                        crate::core::runtime::CoreContext::scope(Arc::clone(core_context), dispatch)
                            .await
                    }
                    None => dispatch.await,
                }
            }
            _ => Err(tinybus::Error::failed("channel callback unavailable")),
        }
    }
}

pub(crate) async fn handle_runtime_command_with_reset<F, Fut>(
    ctx: &ChannelRuntimeContext,
    msg: &traits::ChannelMessage,
    target_channel: Option<&Arc<ChannelHandle>>,
    sender_key: &str,
    reset_scope: Option<tinychannels_bus::module_remote::RemoteSessionScope>,
    durable_session_key: Option<String>,
    reset_message: &traits::ChannelMessage,
    reset_history: F,
) -> bool
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<(), String>>,
{
    // This is a host dispatch hint; the module alone parses and recognizes
    // commands. Ordinary messages avoid creating a private callback object.
    if !msg.content.trim_start().starts_with('/') {
        return false;
    }
    let current = get_route_selection(ctx, sender_key);
    // Telegram's legacy `thread_ts` is the inbound provider message id used
    // for reply targeting; forum topic routing is already encoded in
    // `reply_target` and must not be reused as a reply-to id.
    let reply_thread = msg.thread_ts.clone();
    let reply_recipient = reset_scope
        .as_ref()
        .map(|scope| scope.conversation.id.clone())
        .unwrap_or_else(|| msg.reply_target.clone());
    let reply_topic = reset_scope
        .as_ref()
        .and_then(|scope| scope.conversation.topic_id.clone())
        .or_else(|| msg.topic_id.clone());
    let reply_message = |content: &str| {
        let mut message = SendMessage::new(content, reply_recipient.clone());
        message.thread_ts = reply_thread.clone();
        message.topic_id = reply_topic.clone();
        message.reply_to_id = msg.reply_to_id.clone();
        message
    };
    let history_len =
        durable_channel_history_len(ctx, &current, &msg.channel, durable_session_key.as_deref())
            .await;
    let configured_channel = configured_control_channel(ctx, reset_scope.as_ref());
    let client = super::context::module_client(ctx).await;
    let operation = async {
        let lease = crate::modules::channel_runtime::ChannelRuntimeLease::open(
            client,
            RouteCallbacks::new(
                ctx.clone(),
                super::runtime::session::channel_session(),
                reset_scope
                    .clone()
                    .zip(durable_session_key)
                    .map(|(scope, durable_session_key)| {
                        crate::channels::host::remote_control::RemoteResetContext {
                            scope,
                            durable_session_key,
                            message: reset_message.clone(),
                            history_key: sender_key.to_owned(),
                        }
                    }),
            ),
        )
        .await?;
        let response: Result<ChannelControlResult, _> = lease
            .object()
            .call_confidential(
                tinychannels_bus::methods::PREPARE_CHANNEL_CONTROL,
                (ChannelControlRequest::Route {
                    workspace_dir: ctx.workspace_dir.to_string_lossy().into_owned(),
                    message: Box::new(msg.clone()),
                    capabilities: target_channel
                        .map(|channel| Box::new(channel.capabilities()))
                        .or_else(|| {
                            configured_channel
                                .as_ref()
                                .map(|configured| Box::new(configured.capabilities.clone()))
                        }),
                    route: tinychannels_bus::module_remote::RemoteRoute {
                        provider: current.provider,
                        model: current.model,
                    },
                    history_len,
                    session_scope: reset_scope.map(Box::new),
                    configured_channel,
                    providers: ctx
                        .config
                        .as_deref()
                        .map(provider_descriptors)
                        .unwrap_or_default(),
                },),
            )
            .await;
        let close = lease.close().await;
        let response = response?;
        close?;
        Ok::<_, tinybus::module::ModuleCallError>(response)
    }
    .await;
    let (reply, next_route, clear_history) = match operation {
        Ok(ChannelControlResult::Route {
            reply,
            next_route,
            clear_history,
        }) => (reply, next_route, clear_history),
        Ok(_) => {
            crate::modules::report::malformed_reply("tinychannels");
            return true;
        }
        Err(_) => {
            if let Some(channel) = target_channel {
                let reply = crate::channels::failure_copy::generic_failure(ctx.config.as_deref());
                let _ = channel
                    .send_with_outbound_intent(&reply_message(&reply))
                    .await;
            }
            return true;
        }
    };
    if clear_history {
        if let Err(error) = reset_history().await {
            tracing::warn!(%error, "channel durable history reset failed");
            if let Some(channel) = target_channel {
                let copy = crate::channels::failure_copy::generic_failure(ctx.config.as_deref());
                let _ = channel
                    .send_with_outbound_intent(&reply_message(&copy))
                    .await;
            }
            return true;
        }
    }
    // Apply a route change only after its requested durable reset has
    // succeeded, so a failed reset cannot partially alter command state.
    if let Some(route) = next_route {
        set_route_selection(
            ctx,
            sender_key,
            ChannelRouteSelection {
                provider: route.provider,
                model: route.model,
            },
        );
    }
    let Some(reply) = reply else {
        return false;
    };
    if let Some(channel) = target_channel {
        if channel
            .send_with_outbound_intent(&reply_message(&reply))
            .await
            .is_err()
        {
            tracing::warn!("channel command response failed");
        }
    }
    true
}

/// Resolve the configured instance only from the account identity prepared by
/// the owning channel module. Missing or mismatched identities stay fail-closed.
fn configured_control_channel(
    ctx: &ChannelRuntimeContext,
    scope: Option<&tinychannels_bus::module_remote::RemoteSessionScope>,
) -> Option<Box<tinychannels_bus::module_control::ConfiguredControlChannel>> {
    let scope = scope?;
    let account = scope.channel.account_id.as_deref()?;
    let channel = ctx.channels_by_name.get(account)?;
    if tinychannels_bus::provider_id(channel.name()) != scope.channel.id {
        return None;
    }
    let configured = Box::new(tinychannels_bus::module_control::ConfiguredControlChannel {
        name: channel.name().to_owned(),
        capabilities: channel.capabilities(),
    });
    Some(configured)
}

async fn durable_channel_history_len(
    ctx: &ChannelRuntimeContext,
    route: &ChannelRouteSelection,
    channel: &str,
    session_key: Option<&str>,
) -> usize {
    let Some(session_key) = session_key else {
        return 0;
    };
    let agent_id = match crate::channels::runtime::dispatch::host_agent::route(ctx, channel) {
        crate::channels::runtime::dispatch::host_agent::HostRoute::Agent(host) => {
            host.definition.id.clone()
        }
        crate::channels::runtime::dispatch::host_agent::HostRoute::Missing(_) => return 0,
        crate::channels::runtime::dispatch::host_agent::HostRoute::Unbound => {
            match crate::channels::runtime::dispatch::host_agent::configured_agent(ctx, route).await
            {
                Ok(host) => host.definition.id,
                Err(error) => {
                    tracing::debug!(%error, "channel history count unavailable");
                    return 0;
                }
            }
        }
    };
    let dispatch = ctx.dispatch_ctx();
    let Some(core) = dispatch.resolved() else {
        return 0;
    };
    let store_agent_id = crate::storage::sessions::current_agent_key_or(&dispatch, &agent_id);
    let locator = crate::storage::sessions::transcripts_or_files_in(
        &core,
        &store_agent_id,
        &ctx.workspace_dir,
    );
    // The runtime session addresses its transcript by the tenant-scoped store
    // key (the session identity resolver's agent), not the bare definition id.
    let session = tinyagents_session::transcript::SessionRef::scoped(session_key, &store_agent_id);
    let head = locator.head_generation(&session);
    let Some(read) = locator.read_session_transcript(&head) else {
        return 0;
    };
    let Ok(Some(transcript)) = read.read_session() else {
        return 0;
    };
    transcript
        .messages
        .iter()
        .filter(|message| message.role == "user" || message.role == "assistant")
        .count()
}

#[cfg(test)]
pub(crate) async fn handle_runtime_command_if_needed(
    ctx: &ChannelRuntimeContext,
    msg: &traits::ChannelMessage,
    target_channel: Option<&Arc<ChannelHandle>>,
    sender_key: &str,
) -> bool {
    handle_runtime_command_with_reset(
        ctx,
        msg,
        target_channel,
        sender_key,
        None,
        None,
        msg,
        || async { Ok(()) },
    )
    .await
}

#[cfg(test)]
#[path = "routes_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "routes_remote_reset_tests.rs"]
mod remote_reset_tests;
