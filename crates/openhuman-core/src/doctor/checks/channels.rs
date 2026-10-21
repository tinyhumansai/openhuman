//! Read-only health checks for the configured TinyChannels providers.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tinychannels_bus::{ChannelHealthReport, ChannelHealthState, ChannelsConfig, HttpProxyPolicy};

use crate::config::Config;
use crate::doctor::{ComponentId, DoctorCheck, DoctorCtx, DoctorTier, Finding};

#[cfg(all(feature = "modules", feature = "channels"))]
const MODULE_ID: &str = "tinychannels";
const PROVIDER_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ChannelHealthFailure {
    Unavailable,
    Failed,
}

#[async_trait]
pub(super) trait ChannelHealthProbe: Send + Sync {
    async fn check(
        &self,
        config: &Config,
        provider: &str,
        proxy: HttpProxyPolicy,
    ) -> Result<Option<Vec<ChannelHealthReport>>, ChannelHealthFailure>;
}

struct ExistingModuleProbe;

#[async_trait]
impl ChannelHealthProbe for ExistingModuleProbe {
    async fn check(
        &self,
        config: &Config,
        provider: &str,
        proxy: HttpProxyPolicy,
    ) -> Result<Option<Vec<ChannelHealthReport>>, ChannelHealthFailure> {
        // Check readiness before reading credentials. `check_existing` repeats
        // this immediately before using the captured proxy, and never resolves
        // a module artifact or starts a runtime.
        if !module_is_ready(config) {
            return Ok(None);
        }

        #[cfg(all(feature = "modules", feature = "channels"))]
        let channels = health_config(config, provider);
        #[cfg(all(feature = "modules", feature = "channels"))]
        return crate::modules::channel_health::check_existing(
            config,
            channels,
            proxy,
            PROVIDER_TIMEOUT,
        )
        .await
        .map_err(|error| match error {
            crate::modules::channel_health::ExistingHealthError::Unavailable => {
                ChannelHealthFailure::Unavailable
            }
            crate::modules::channel_health::ExistingHealthError::Failed => {
                ChannelHealthFailure::Failed
            }
        });
        #[cfg(not(all(feature = "modules", feature = "channels")))]
        {
            let _ = (provider, proxy);
            Ok(None)
        }
    }
}

pub(super) fn for_context(ctx: &DoctorCtx) -> Vec<Arc<dyn DoctorCheck>> {
    configured_provider_ids(ctx.config())
        .into_iter()
        .map(|provider| {
            Arc::new(ChannelHealthCheck::configured(
                provider,
                Arc::new(ExistingModuleProbe),
            )) as Arc<dyn DoctorCheck>
        })
        .collect()
}

pub(super) struct ChannelHealthCheck {
    id: String,
    provider: String,
    probe: Arc<dyn ChannelHealthProbe>,
}

impl ChannelHealthCheck {
    fn configured(provider: &str, probe: Arc<dyn ChannelHealthProbe>) -> Self {
        Self {
            id: format!("channels/{provider}"),
            provider: provider.to_owned(),
            probe,
        }
    }

    #[cfg(test)]
    fn with_probe(provider: &str, probe: Arc<dyn ChannelHealthProbe>) -> Self {
        Self::configured(provider, probe)
    }

    async fn finding(&self, config: &Config) -> Finding {
        let proxy = proxy_policy(config, &self.provider);
        match self.probe.check(config, &self.provider, proxy).await {
            Ok(None) | Err(ChannelHealthFailure::Unavailable) => Finding::skipped(
                &self.id,
                format!(
                    "The {} channel module is not already serving for this run.",
                    channel_label(&self.provider)
                ),
                "Start or reconnect the channel through its normal settings, then rerun doctor.",
            ),
            Err(ChannelHealthFailure::Failed) => Finding::error(
                &self.id,
                format!(
                    "The {} channel health request failed.",
                    channel_label(&self.provider)
                ),
                "Check the channel configuration and its local module status, then rerun doctor.",
            ),
            Ok(Some(reports)) => {
                let Some(report) = reports
                    .iter()
                    .find(|report| report.channel == self.provider)
                else {
                    return Finding::error(
                        &self.id,
                        "The channel module did not report this configured provider.",
                        "Reconnect the provider through channel settings, then rerun doctor.",
                    );
                };
                let label = channel_label(&self.provider);
                match report.state {
                    ChannelHealthState::Healthy => Finding::pass(&self.id, format!("{label} is healthy.")),
                    ChannelHealthState::Unhealthy => Finding::error(
                        &self.id,
                        format!("{label} is unhealthy (auth/config/network)."),
                        "Review the provider credentials and connection settings, then rerun doctor.",
                    ),
                    ChannelHealthState::Timeout => Finding::error(
                        &self.id,
                        format!("{label} health check timed out."),
                        "Check the provider connection and network policy, then rerun doctor.",
                    ),
                }
            }
        }
    }
}

#[async_trait]
impl DoctorCheck for ChannelHealthCheck {
    fn id(&self) -> &str {
        &self.id
    }

    fn category(&self) -> &str {
        "channels"
    }

    fn tier(&self) -> DoctorTier {
        DoctorTier::Connect
    }

    fn component_gate(&self) -> Option<ComponentId> {
        Some(ComponentId::Channels)
    }

    async fn detect(&self, ctx: &DoctorCtx) -> Vec<Finding> {
        vec![self.finding(ctx.config()).await]
    }
}

fn channel_label(channel: &str) -> String {
    tinychannels_bus::controllers::find_channel_definition(channel)
        .map(|definition| definition.display_name.to_string())
        .unwrap_or_else(|| channel.to_string())
}

fn module_is_ready(config: &Config) -> bool {
    #[cfg(all(feature = "modules", feature = "channels"))]
    {
        config.modules.enabled
            && crate::modules::runtime::existing_runtime().is_some()
            && crate::modules::ops::state_of(MODULE_ID) == crate::modules::types::ModuleState::Ready
    }
    #[cfg(not(all(feature = "modules", feature = "channels")))]
    {
        let _ = config;
        false
    }
}

fn proxy_policy(config: &Config, provider: &str) -> HttpProxyPolicy {
    let service = format!("channel.{provider}");
    let selectors = config
        .proxy
        .should_apply_to_service(&service)
        .then_some(service)
        .into_iter()
        .collect::<Vec<_>>();
    if selectors.is_empty() {
        HttpProxyPolicy::default()
    } else {
        HttpProxyPolicy {
            http_proxy: config.proxy.http_proxy.clone(),
            https_proxy: config.proxy.https_proxy.clone(),
            all_proxy: config.proxy.all_proxy.clone(),
            no_proxy: config.proxy.normalized_no_proxy(),
            service_selectors: selectors,
        }
    }
}

fn health_config(config: &Config, provider: &str) -> ChannelsConfig {
    let mut channels = config.channels_config.clone();
    #[cfg(feature = "channels")]
    {
        if provider == "email" {
            if let Some(email) = channels.email.take() {
                channels.email = Some(
                    crate::channels::credentials::resolve_email_password_read_only(email, config),
                );
            }
        }
        if provider == "yuanbao" {
            if let Some(yuanbao) = channels.yuanbao.take() {
                channels.yuanbao = Some(
                    crate::channels::credentials::resolve_yuanbao_app_secret_read_only(
                        yuanbao, config,
                    ),
                );
            }
        }
    }

    channels.cli = false;
    channels.webhook = None;
    channels.relay = None;
    channels.active_channel = None;
    channels.telegram = (provider == "telegram")
        .then(|| config.channels_config.telegram.clone())
        .flatten();
    channels.discord = (provider == "discord")
        .then(|| config.channels_config.discord.clone())
        .flatten();
    channels.slack = (provider == "slack")
        .then(|| config.channels_config.slack.clone())
        .flatten();
    channels.mattermost = (provider == "mattermost")
        .then(|| config.channels_config.mattermost.clone())
        .flatten();
    channels.imessage = (provider == "imessage")
        .then(|| config.channels_config.imessage.clone())
        .flatten();
    channels.matrix = (provider == "matrix")
        .then(|| config.channels_config.matrix.clone())
        .flatten();
    channels.signal = (provider == "signal")
        .then(|| config.channels_config.signal.clone())
        .flatten();
    channels.whatsapp = (provider == "whatsapp")
        .then(|| config.channels_config.whatsapp.clone())
        .flatten();
    channels.linq = (provider == "linq")
        .then(|| config.channels_config.linq.clone())
        .flatten();
    channels.email = (provider == "email")
        .then(|| channels.email.clone())
        .flatten();
    channels.irc = (provider == "irc")
        .then(|| config.channels_config.irc.clone())
        .flatten();
    channels.lark = (provider == "lark")
        .then(|| config.channels_config.lark.clone())
        .flatten();
    channels.dingtalk = (provider == "dingtalk")
        .then(|| config.channels_config.dingtalk.clone())
        .flatten();
    channels.qq = (provider == "qq")
        .then(|| config.channels_config.qq.clone())
        .flatten();
    channels.yuanbao = (provider == "yuanbao")
        .then(|| channels.yuanbao.clone())
        .flatten();
    channels
}

fn configured_provider_ids(config: &Config) -> Vec<&'static str> {
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
    .filter_map(|(provider, configured)| configured.then_some(provider))
    .collect()
}

#[cfg(test)]
#[path = "channels_tests.rs"]
mod tests;
