//! Small helpers shared across the connect/disconnect/status operations.

pub(crate) async fn channel_config_connected(
    config: &crate::config::Config,
    channel: &str,
    mode: tinychannels_bus::ChannelAuthMode,
) -> Result<bool, String> {
    let channels = &config.channels_config;
    let configured = [
        ("telegram", channels.telegram.is_some()),
        ("discord", channels.discord.is_some()),
        ("slack", channels.slack.is_some()),
        ("mattermost", channels.mattermost.is_some()),
        ("imessage", channels.imessage.is_some()),
        ("matrix", channels.matrix.is_some()),
        ("signal", channels.signal.is_some()),
        ("whatsapp", channels.whatsapp.is_some()),
        ("linq", channels.linq.is_some()),
        ("email", channels.email.is_some()),
        ("irc", channels.irc.is_some()),
        ("lark", channels.lark.is_some()),
        ("dingtalk", channels.dingtalk.is_some()),
        ("qq", channels.qq.is_some()),
        ("yuanbao", channels.yuanbao.is_some()),
    ]
    .into_iter()
    .filter(|(_, present)| *present)
    .map(|(name, _)| name.to_owned())
    .collect();
    #[cfg(test)]
    let client = crate::channels::module_fixture::client().await;
    #[cfg(not(test))]
    let client = crate::modules::client(config.clone());
    use tinychannels_bus::module_control::{ChannelControlRequest, ChannelControlResult};
    match client
        .call_confidential(
            "tinychannels",
            tinychannels_bus::methods::PREPARE_CHANNEL_CONTROL,
            (ChannelControlRequest::Connected {
                configured,
                channel: channel.into(),
                mode,
            },),
        )
        .await
        .map_err(|e| e.to_string())?
    {
        ChannelControlResult::Connected(connected) => Ok(connected),
        _ => {
            crate::modules::report::malformed_reply("tinychannels");
            Err(tinybus::module::ModuleCallError::ModuleFault.to_string())
        }
    }
}

/// Merge a channel's live supervised-listener health into its credential/config
/// derived `connected` flag (issue #3712).
///
/// Only listener-backed modes (those that materialise a TOML config block —
/// `has_config`) have a `channel:<id>` health component, kept current by the
/// supervisor's `ChannelConnected`/`ChannelDisconnected` events. For those, a
/// live `error` overrides the optimistic presence-based `connected` and carries
/// the failure reason to the UI; an `ok` confirms it. While the listener is
/// still `starting` (or has no component yet) we keep the presence-based value
/// so a freshly-configured channel isn't reported as broken before its first
/// connect attempt. Modes without a runtime listener (e.g. managed-DM) are left
/// untouched. Returns `(connected, error)`.
pub(crate) fn merge_listener_health(
    presence_connected: bool,
    has_config: bool,
    health_status: Option<&str>,
    health_last_error: Option<&str>,
) -> (bool, Option<String>) {
    if !has_config {
        return (presence_connected, None);
    }
    match health_status {
        Some("error") => (false, health_last_error.map(str::to_string)),
        Some("ok") => (true, None),
        _ => (presence_connected, None),
    }
}

pub(crate) use crate::security::credentials::channel_credential_provider as credential_provider;
