use super::super::host_agent::{self, HostRoute};
use crate::channels::context::ChannelRuntimeContext;

pub(crate) async fn reset_command(
    ctx: std::sync::Arc<ChannelRuntimeContext>,
    message: tinychannels_bus::ChannelMessage,
    session_key: String,
    history_key: String,
) -> Result<(), String> {
    reset_durable_channel_session(&ctx, &message, &session_key, &history_key)
        .await
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// Reset the durable session selected for this channel conversation. The
/// module still owns command parsing; this host adapter resolves trusted agent
/// authority and serializes the generation transition with turns.
pub(crate) async fn reset_durable_channel_session(
    ctx: &ChannelRuntimeContext,
    msg: &tinychannels_bus::ChannelMessage,
    session_key: &str,
    history_key: &str,
) -> anyhow::Result<bool> {
    let host = match host_agent::route(ctx, &msg.channel) {
        HostRoute::Agent(host) => *host,
        HostRoute::Missing(agent_id) => {
            anyhow::bail!("configured channel agent {agent_id} is unavailable")
        }
        HostRoute::Unbound => host_agent::configured_agent(
            ctx,
            &crate::channels::routes::get_route_selection(ctx, history_key),
        )
        .await
        .map_err(anyhow::Error::msg)?,
    };
    let dispatch = ctx.dispatch_ctx();
    let tenant = dispatch.tenant().map_err(anyhow::Error::msg)?;
    let lease = crate::agent::session_lease::SessionLease::acquire(
        crate::agent::session_lease::SessionLeaseKey::new(
            crate::core::runtime::tenant::tenant_key(&tenant, ""),
            host.definition.id.clone(),
            session_key.to_owned(),
        ),
        None,
    )
    .await
    .map_err(|()| anyhow::anyhow!("channel session reset was cancelled"))?;
    let reset = host
        .reset_external_channel_history(session_key, history_key, &msg.channel)
        .await;
    drop(lease);
    reset
}
