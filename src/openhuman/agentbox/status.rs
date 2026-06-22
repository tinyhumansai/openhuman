//! Read-only AgentBox status snapshot for the desktop control panel.
//!
//! Derives everything from process env (the same source the runtime mount and
//! provider registration read) so the panel reflects exactly what the running
//! core sees. **Never surfaces the GMI API key.**

use crate::rpc::RpcOutcome;

use super::env::{agentbox_mode_enabled, collect_gmi_config, GMI_MAAS_SLUG};
use super::types::{AgentBoxProviderInfo, AgentBoxStatus};

/// Build the current AgentBox status from process env.
pub fn agentbox_status() -> RpcOutcome<AgentBoxStatus> {
    let mode_enabled = agentbox_mode_enabled();
    let provider = match collect_gmi_config(|k| std::env::var(k).ok()) {
        Ok(cfg) => Some(AgentBoxProviderInfo {
            slug: GMI_MAAS_SLUG.to_string(),
            base_url: cfg.base_url,
            model: cfg.model,
        }),
        Err(_) => None,
    };
    let provider_configured = provider.is_some();

    log::debug!(
        "[agentbox] status mode_enabled={mode_enabled} provider_configured={provider_configured}"
    );

    RpcOutcome::single_log(
        AgentBoxStatus {
            mode_enabled,
            provider_configured,
            provider,
        },
        format!(
            "agentbox status: mode_enabled={mode_enabled} provider_configured={provider_configured}"
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::agentbox::env::AGENTBOX_MODE_ENV_VAR;

    // Env vars are process-global; hold the AgentBox test env lock so these
    // tests do not race each other or other AgentBox mode tests in the full
    // lib suite.
    fn with_clean_env<F: FnOnce()>(f: F) {
        let _lock = super::super::test_support::test_env_lock();
        let prior_mode = std::env::var(AGENTBOX_MODE_ENV_VAR).ok();
        let prior_url = std::env::var("GMI_MAAS_BASE_URL").ok();
        let prior_key = std::env::var("GMI_MAAS_API_KEY").ok();
        let prior_models = std::env::var("GMI_MODELS").ok();

        std::env::remove_var(AGENTBOX_MODE_ENV_VAR);
        std::env::remove_var("GMI_MAAS_BASE_URL");
        std::env::remove_var("GMI_MAAS_API_KEY");
        std::env::remove_var("GMI_MODELS");

        f();

        let restore = |k: &str, v: Option<String>| match v {
            Some(v) => std::env::set_var(k, v),
            None => std::env::remove_var(k),
        };
        restore(AGENTBOX_MODE_ENV_VAR, prior_mode);
        restore("GMI_MAAS_BASE_URL", prior_url);
        restore("GMI_MAAS_API_KEY", prior_key);
        restore("GMI_MODELS", prior_models);
    }

    #[test]
    fn status_reports_disabled_and_unconfigured_by_default() {
        with_clean_env(|| {
            let status = agentbox_status().value;
            assert!(!status.mode_enabled);
            assert!(!status.provider_configured);
            assert!(status.provider.is_none());
        });
    }

    #[test]
    fn status_reports_provider_without_leaking_key() {
        with_clean_env(|| {
            std::env::set_var(AGENTBOX_MODE_ENV_VAR, "1");
            std::env::set_var("GMI_MAAS_BASE_URL", "https://api.gmi-serving.com");
            std::env::set_var("GMI_MAAS_API_KEY", "sk-secret-should-not-appear");
            std::env::set_var("GMI_MODELS", "deepseek-ai/DeepSeek-V4-Pro");

            let outcome = agentbox_status();
            let status = outcome.value.clone();
            assert!(status.mode_enabled);
            assert!(status.provider_configured);
            let provider = status.provider.expect("provider populated");
            assert_eq!(provider.slug, GMI_MAAS_SLUG);
            assert_eq!(provider.base_url, "https://api.gmi-serving.com");
            assert_eq!(provider.model, "deepseek-ai/DeepSeek-V4-Pro");

            // Defense-in-depth: the serialized status must never carry the key.
            let json = serde_json::to_string(&outcome.value).unwrap();
            assert!(!json.contains("sk-secret-should-not-appear"));
        });
    }
}
