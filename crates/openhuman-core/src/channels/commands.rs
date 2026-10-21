//! Select configured listeners for startup orchestration and native doctor checks.

use crate::config::Config;

/// Selecting configured providers is host configuration orchestration. The
/// module owns construction, platform support, health I/O and timeout handling.
pub(crate) fn configured_providers(config: &Config) -> Vec<&'static str> {
    let channels = &config.channels_config;
    [
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
    .filter_map(|(channel, configured)| configured.then_some(channel))
    .collect()
}

/// Whether config contains a channel or relay that needs the native module
/// runtime. Push webhooks remain host-server work and do not start listeners.
pub(crate) fn has_listener_configuration(config: &Config) -> bool {
    !configured_providers(config).is_empty()
        || config
            .channels_config
            .relay
            .as_ref()
            .is_some_and(|relay| !relay.url.trim().is_empty() && !relay.identities.is_empty())
}

#[cfg(test)]
#[path = "commands_tests.rs"]
mod tests;
