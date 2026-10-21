//! Confidential, existing-module-only adapter for TinyChannels health checks.
use std::time::Duration;

use tinybus::module::{ModuleCallError, ModuleClient};
use tinychannels_bus::{ChannelHealthReport, ChannelsConfig, HttpProxyPolicy};

/// A safe failure class for one already-serving TinyChannels health probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExistingHealthError {
    Unavailable,
    Failed,
}

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

/// Ask an already-serving TinyChannels module to inspect the supplied config.
///
/// This path never constructs a `ModuleClient`, resolves an artifact, starts
/// the module runtime, or opens a provider listener. The selected provider
/// configuration and proxy policy are passed in the confidential health-call
/// arguments, so a module first loaded under another config cannot substitute
/// its ambient provider credentials.
pub(crate) async fn check_existing(
    config: &crate::config::Config,
    channels: ChannelsConfig,
    proxy_policy: HttpProxyPolicy,
    timeout: Duration,
) -> Result<Option<Vec<ChannelHealthReport>>, ExistingHealthError> {
    #[cfg(feature = "modules")]
    {
        use crate::modules::types::ModuleState;

        if !config.modules.enabled
            || crate::modules::ops::state_of("tinychannels") != ModuleState::Ready
        {
            return Ok(None);
        }
        let Some(runtime) = crate::modules::runtime::existing_runtime() else {
            return Ok(None);
        };
        let Some(record) = crate::modules::registry::find("tinychannels") else {
            return Err(ExistingHealthError::Unavailable);
        };
        let proxy = runtime
            .proxy(record.bus_name, record.object_path)
            .map_err(|_| ExistingHealthError::Unavailable)?
            .with_timeout(timeout);
        return call_existing_proxy(&proxy, channels, proxy_policy, timeout)
            .await
            .map(Some);
    }

    #[cfg(not(feature = "modules"))]
    {
        let _ = (config, channels, proxy_policy, timeout);
        Ok(None)
    }
}

async fn call_existing_proxy(
    proxy: &tinybus::Proxy,
    channels: ChannelsConfig,
    proxy_policy: HttpProxyPolicy,
    timeout: Duration,
) -> Result<Vec<ChannelHealthReport>, ExistingHealthError> {
    let args = health_check_args(channels, proxy_policy, timeout);
    tokio::time::timeout(
        timeout,
        proxy
            .clone()
            .with_timeout(timeout)
            .call_confidential(tinychannels_bus::methods::CHECK_CHANNEL_HEALTH, args),
    )
    .await
    .map_err(|_| ExistingHealthError::Failed)?
    .map_err(|_| ExistingHealthError::Failed)
}

fn health_check_args(
    channels: ChannelsConfig,
    proxy_policy: HttpProxyPolicy,
    timeout: Duration,
) -> (ChannelsConfig, HttpProxyPolicy, u64) {
    (
        channels,
        proxy_policy,
        timeout.as_millis().min(u64::MAX as u128) as u64,
    )
}

#[cfg(all(test, feature = "modules"))]
#[path = "channel_health_tests.rs"]
mod tests;
