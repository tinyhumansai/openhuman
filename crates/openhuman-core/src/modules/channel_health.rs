//! Thin confidential adapter for TinyChannels' provider health checks.
use super::client::{ModuleCallError, ModuleClient};
use tinychannels_bus::{ChannelHealthReport, ChannelsConfig, HttpProxyPolicy};

pub(crate) async fn check(
    client: &ModuleClient,
    config: ChannelsConfig,
    proxy: HttpProxyPolicy,
) -> Result<Vec<ChannelHealthReport>, ModuleCallError> {
    client
        .call_confidential(
            "tinychannels",
            tinychannels_bus::methods::CHECK_CHANNEL_HEALTH,
            (config, proxy, 10_000_u64),
        )
        .await
}

#[cfg(test)]
#[path = "channel_health_tests.rs"]
mod tests;
