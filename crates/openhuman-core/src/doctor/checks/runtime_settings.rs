//! Remaining static runtime-setting warnings from the former aggregate config check.

use std::sync::Arc;

use async_trait::async_trait;

use crate::config::Config;
use crate::doctor::{DoctorCheck, DoctorCtx, DoctorTier, Finding, FindingSeverity};

const CHECK_ID: &str = "config/runtime-settings";

pub(super) fn checks() -> Vec<Arc<dyn DoctorCheck>> {
    vec![Arc::new(RuntimeSettingsCheck)]
}

struct RuntimeSettingsCheck;

#[async_trait]
impl DoctorCheck for RuntimeSettingsCheck {
    fn id(&self) -> &str {
        CHECK_ID
    }

    fn category(&self) -> &str {
        "runtime configuration"
    }

    fn tier(&self) -> DoctorTier {
        DoctorTier::Static
    }

    async fn detect(&self, ctx: &DoctorCtx) -> Vec<Finding> {
        tracing::debug!(target: "doctor", check = CHECK_ID, event = "inspection_started");
        let findings = runtime_settings(ctx.config());
        tracing::debug!(
            target: "doctor",
            check = CHECK_ID,
            finding_count = findings.len(),
            "runtime settings inspected"
        );
        findings
    }
}

fn runtime_settings(config: &Config) -> Vec<Finding> {
    let mut findings = Vec::new();

    if !config.reliability.fallback_providers.is_empty() {
        findings.push(warning(
            "reliability.fallback_providers",
            "The configured fallback-provider list is ignored by the current runtime.",
            "Remove `reliability.fallback_providers` or configure supported provider routes.",
        ));
    }

    for (index, route) in config.model_routes.iter().enumerate() {
        let path = format!("model_routes[{index}]");
        if route.hint.trim().is_empty() {
            findings.push(warning(
                &format!("{path}.hint"),
                "A model route has no workload hint.",
                "Set a non-empty workload hint for this model route.",
            ));
        }
        if route.model.trim().is_empty() {
            findings.push(warning(
                &format!("{path}.model"),
                "A model route has no model configured.",
                "Set a model for this route or remove the unused route.",
            ));
        }
    }

    if config.components.channels {
        if has_configured_channel(config) {
            findings.push(pass(
                "channels_config",
                "At least one messaging channel is configured.",
            ));
        } else {
            findings.push(warning(
                "channels_config",
                "No messaging channel is configured.",
                "Configure a messaging channel in Settings if this runtime should receive messages.",
            ));
        }
    }

    findings
}

fn has_configured_channel(config: &Config) -> bool {
    let channels = &config.channels_config;
    channels.telegram.is_some()
        || channels.discord.is_some()
        || channels.slack.is_some()
        || channels.mattermost.is_some()
        || channels.imessage.is_some()
        || channels.signal.is_some()
        || channels.whatsapp.is_some()
        || channels.linq.is_some()
        || channels.email.is_some()
        || channels.irc.is_some()
        || channels.lark.is_some()
        || channels.dingtalk.is_some()
        || channels.qq.is_some()
        || channels.yuanbao.is_some()
        || channels.relay.is_some()
}

fn warning(config_path: &str, message: &str, fix_hint: &str) -> Finding {
    let mut finding = Finding::new(CHECK_ID, FindingSeverity::Warn, message, fix_hint);
    finding.config_path = Some(config_path.to_owned());
    finding
}

fn pass(config_path: &str, message: &str) -> Finding {
    let mut finding = Finding::pass(CHECK_ID, message);
    finding.config_path = Some(config_path.to_owned());
    finding
}

#[cfg(test)]
#[path = "runtime_settings_tests.rs"]
mod tests;
