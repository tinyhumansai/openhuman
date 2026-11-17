//! Thin adapters for module-owned channel controller operations.
use serde::de::DeserializeOwned;
use tinybus::module::{ModuleCallError, ModuleClient};
use tinychannels_bus::{
    methods, ChannelControllerRequest, ChannelControllerResult, HttpProxyPolicy,
};

#[derive(Clone, Copy)]
pub(crate) enum DiscordReply {
    Guilds,
    Channels,
    Permissions,
}

pub(crate) async fn discord<R: DeserializeOwned>(
    client: &ModuleClient,
    request: ChannelControllerRequest,
    expected: DiscordReply,
) -> Result<R, ModuleCallError> {
    let response = client
        .call_confidential("tinychannels", methods::CHANNEL_CONTROLLER, (request,))
        .await?;
    decode_discord(response, expected)
}

fn decode_discord<R: DeserializeOwned>(
    response: ChannelControllerResult,
    expected: DiscordReply,
) -> Result<R, ModuleCallError> {
    let raw = match (expected, response) {
        (DiscordReply::Guilds, ChannelControllerResult::DiscordGuilds(result)) => result.raw,
        (DiscordReply::Channels, ChannelControllerResult::DiscordChannels(result)) => result.raw,
        (DiscordReply::Permissions, ChannelControllerResult::DiscordPermissions(result)) => {
            result.raw
        }
        _ => None,
    };
    raw.and_then(|value| serde_json::from_value(value).ok())
        .ok_or_else(|| {
            crate::modules::report::malformed_reply("tinychannels");
            ModuleCallError::ModuleFault
        })
}

/// Prepare a verified connection without running provider behavior in the host.
pub(crate) async fn prepare_connection(
    client: &ModuleClient,
    request: ChannelControllerRequest,
) -> Result<tinychannels_bus::module_controller::PreparedChannelConnection, ModuleCallError> {
    let response = client
        .call_confidential("tinychannels", methods::CHANNEL_CONTROLLER, (request,))
        .await?;
    match response {
        ChannelControllerResult::PreparedConnect(prepared) => Ok(*prepared),
        _ => {
            crate::modules::report::malformed_reply("tinychannels");
            Err(ModuleCallError::ModuleFault)
        }
    }
}

/// Normalize rich outbound content and derive its idempotency key inside the module.
pub(crate) async fn prepare_message(
    client: &ModuleClient,
    channel: &str,
    message: serde_json::Value,
) -> Result<tinychannels_bus::module_runtime::PreparedChannelIntent, ModuleCallError> {
    client
        .call_confidential(
            "tinychannels",
            methods::PREPARE_OUTBOUND_INTENT,
            (
                channel,
                tinychannels_bus::module_runtime::ChannelMessagePreparation::Legacy(message),
            ),
        )
        .await
}

/// Derive persisted hosted-ingress identifiers inside the channel module.
pub(crate) async fn prepare_identity(
    client: &ModuleClient,
    request: tinychannels_bus::module_inbound::PrepareChannelIdentityRequest,
) -> Result<tinychannels_bus::module_inbound::PreparedChannelIdentity, ModuleCallError> {
    client
        .call_confidential(
            "tinychannels",
            methods::PREPARE_CHANNEL_IDENTITY,
            (request,),
        )
        .await
}

/// Prepare provider projections and persisted keys without a linked implementation.
pub(crate) async fn prepare_inbound(
    client: &ModuleClient,
    request: tinychannels_bus::module_inbound::PrepareChannelInboundRequest,
) -> Result<tinychannels_bus::module_inbound::PreparedChannelInbound, ModuleCallError> {
    client
        .call_confidential("tinychannels", methods::PREPARE_CHANNEL_INBOUND, (request,))
        .await
}

/// Resolve OpenHuman's proxy scope before transmitting provider configuration.
pub(crate) fn discord_proxy(config: &crate::config::Config) -> HttpProxyPolicy {
    let proxy = &config.proxy;
    if !proxy.should_apply_to_service("channel.discord") {
        return HttpProxyPolicy::default();
    }
    HttpProxyPolicy {
        http_proxy: proxy.http_proxy.clone(),
        https_proxy: proxy.https_proxy.clone(),
        all_proxy: proxy.all_proxy.clone(),
        no_proxy: proxy.normalized_no_proxy(),
        service_selectors: vec!["channel.discord".into()],
    }
}

#[cfg(test)]
#[path = "channel_controller_tests.rs"]
mod tests;
