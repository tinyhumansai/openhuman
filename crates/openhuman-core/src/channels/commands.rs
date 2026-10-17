//! `doctor channels`: health checks for every configured real-time channel.
//!
//! OpenHuman hydrates credentials and applies its proxy policy. Provider
//! construction and bounded health checks execute in the TinyChannels module;
//! this adapter formats the observations.

use super::runtime::hydrate_channel_credentials;
use crate::config::Config;
use anyhow::Result;
use std::time::Duration;
use tinychannels_bus::{ChannelHealthState, HttpProxyPolicy};

/// Display label for a channel id (`telegram` → `Telegram`).
fn channel_label(channel: &str) -> String {
    tinychannels_bus::controllers::find_channel_definition(channel)
        .map(|definition| definition.display_name.to_string())
        .unwrap_or_else(|| channel.to_string())
}

/// One report line for a channel's health state.
fn report_line(label: &str, state: ChannelHealthState) -> String {
    match state {
        ChannelHealthState::Healthy => format!("  ✅ {label:<9} healthy"),
        ChannelHealthState::Unhealthy => format!("  ❌ {label:<9} unhealthy (auth/config/network)"),
        ChannelHealthState::Timeout => format!("  ⏱️  {label:<9} timed out (>10s)"),
    }
}

/// Run health checks for configured channels.
pub async fn doctor_channels(config: Config) -> Result<()> {
    let configured = configured_providers(&config);
    if configured.is_empty() {
        println!("No real-time channels configured. Configure channels in the web UI.");
        return Ok(());
    }
    let selectors = configured
        .iter()
        .map(|channel| format!("channel.{channel}"))
        .filter(|service| config.proxy.should_apply_to_service(service))
        .collect::<Vec<_>>();
    let proxy = if selectors.is_empty() {
        HttpProxyPolicy::default()
    } else {
        HttpProxyPolicy {
            http_proxy: config.proxy.http_proxy.clone(),
            https_proxy: config.proxy.https_proxy.clone(),
            all_proxy: config.proxy.all_proxy.clone(),
            no_proxy: config.proxy.normalized_no_proxy(),
            service_selectors: selectors,
        }
    };
    let client = crate::modules::client(config.clone())
        .with_timeout(Duration::from_secs(configured.len() as u64 * 10 + 5));
    let channels =
        crate::modules::channel_health::check(&client, hydrate_channel_credentials(&config), proxy)
            .await?;

    if channels.is_empty() {
        println!("No real-time channels configured. Configure channels in the web UI.");
        return Ok(());
    }

    println!("🩺 OpenHuman Channel Doctor");
    println!();

    let (mut healthy, mut unhealthy, mut timeout) = (0_u32, 0_u32, 0_u32);
    for report in channels {
        let state = report.state;
        match state {
            ChannelHealthState::Healthy => healthy += 1,
            ChannelHealthState::Unhealthy => unhealthy += 1,
            ChannelHealthState::Timeout => timeout += 1,
        }
        println!("{}", report_line(&channel_label(&report.channel), state));
    }

    if config.channels_config.webhook.is_some() {
        println!("  ℹ️  Webhook   ensure your webhook endpoint is reachable");
    }

    println!();
    println!("Summary: {healthy} healthy, {unhealthy} unhealthy, {timeout} timed out");
    Ok(())
}

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
