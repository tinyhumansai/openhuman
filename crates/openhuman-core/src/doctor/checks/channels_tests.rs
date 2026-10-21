use super::*;
use crate::doctor::contract::{DoctorCtx, FindingSeverity};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tinychannels_bus::{ChannelHealthReport, ChannelHealthState, HttpProxyPolicy};

struct FixedHealth {
    result: Result<Option<Vec<ChannelHealthReport>>, ChannelHealthFailure>,
    calls: Arc<AtomicUsize>,
    readiness_checks: Arc<AtomicUsize>,
    policy: std::sync::Mutex<Option<HttpProxyPolicy>>,
    workspace: std::sync::Mutex<Option<std::path::PathBuf>>,
}

#[async_trait::async_trait]
impl ChannelHealthProbe for FixedHealth {
    async fn check(
        &self,
        config: &crate::config::Config,
        _provider: &str,
        policy: HttpProxyPolicy,
    ) -> Result<Option<Vec<ChannelHealthReport>>, ChannelHealthFailure> {
        self.readiness_checks.fetch_add(1, Ordering::SeqCst);
        if matches!(&self.result, Ok(Some(_))) {
            self.calls.fetch_add(1, Ordering::SeqCst);
        }
        *self.workspace.lock().unwrap() = Some(config.workspace_dir.clone());
        *self.policy.lock().unwrap() = Some(policy);
        self.result.clone()
    }
}

fn config_in(root: &std::path::Path) -> crate::config::Config {
    let mut config = crate::config::Config::default();
    config.workspace_dir = root.join("workspace");
    config.action_dir = root.join("actions");
    config.config_path = root.join("config.toml");
    config
}

fn telegram() -> tinychannels_bus::config::TelegramConfig {
    tinychannels_bus::config::TelegramConfig {
        bot_token: "doctor-test-token".into(),
        chat_id: None,
        allowed_users: vec![],
        stream_mode: Default::default(),
        draft_update_interval_ms: 1_000,
        silent_streaming: true,
        mention_only: false,
    }
}

#[tokio::test]
async fn a_health_timeout_is_an_error_for_the_configured_provider() {
    let scratch = tempfile::tempdir().expect("create isolated config fixture");
    let mut config = config_in(scratch.path());
    config.channels_config.telegram = Some(telegram());
    config.proxy.enabled = true;
    config.proxy.scope = crate::config::schema::ProxyScope::Services;
    config.proxy.services = vec!["channel.telegram".into(), "provider.openai".into()];
    config.proxy.http_proxy = Some("http://proxy.example.invalid:3128".into());
    config.proxy.no_proxy = vec!["localhost".into(), ".example.test".into()];

    let calls = Arc::new(AtomicUsize::new(0));
    let probe = Arc::new(FixedHealth {
        result: Ok(Some(vec![ChannelHealthReport {
            channel: "telegram".into(),
            state: ChannelHealthState::Timeout,
        }])),
        calls: Arc::clone(&calls),
        readiness_checks: Arc::new(AtomicUsize::new(0)),
        policy: Default::default(),
        workspace: Default::default(),
    });
    let check = ChannelHealthCheck::with_probe("telegram", probe.clone());

    let finding = check
        .detect(&DoctorCtx::from_config(config))
        .await
        .into_iter()
        .next()
        .expect("timeout produces a finding");

    assert_eq!(finding.check_id, "channels/telegram");
    assert_eq!(finding.severity, FindingSeverity::Error);
    assert!(finding.message.contains("timed out"));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(probe.readiness_checks.load(Ordering::SeqCst), 1);
    assert_eq!(
        probe
            .policy
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .service_selectors,
        vec!["channel.telegram"]
    );
    assert_eq!(
        probe.policy.lock().unwrap().as_ref().unwrap().no_proxy,
        vec![".example.test", "localhost"]
    );
}

#[tokio::test]
async fn an_unavailable_module_is_skipped_without_opening_it() {
    let scratch = tempfile::tempdir().expect("create isolated config fixture");
    let mut selected_b = config_in(scratch.path().join("b").as_path());
    selected_b.channels_config.telegram = Some(telegram());

    let calls = Arc::new(AtomicUsize::new(0));
    let probe = Arc::new(FixedHealth {
        result: Ok(None),
        calls: Arc::clone(&calls),
        readiness_checks: Arc::new(AtomicUsize::new(0)),
        policy: Default::default(),
        workspace: Default::default(),
    });
    let check = ChannelHealthCheck::with_probe("telegram", probe.clone());
    let finding = check
        .detect(&DoctorCtx::from_config(selected_b))
        .await
        .into_iter()
        .next()
        .expect("missing existing binding is represented");

    assert_eq!(finding.check_id, "channels/telegram");
    assert_eq!(finding.severity, FindingSeverity::Skipped);
    assert_eq!(
        finding.message,
        "The Telegram channel module is not already serving for this run."
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(probe.readiness_checks.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_ready_probe_receives_selected_config_not_ambient_config() {
    let ambient = tempfile::tempdir().expect("create ambient config fixture");
    let selected = tempfile::tempdir().expect("create selected config fixture");
    let mut config_a = config_in(ambient.path());
    config_a.channels_config.imessage = Some(tinychannels_bus::config::IMessageConfig {
        allowed_contacts: vec![],
    });
    let mut config_b = config_in(selected.path());
    config_b.channels_config.telegram = Some(telegram());
    config_b.proxy.enabled = true;
    config_b.proxy.scope = crate::config::schema::ProxyScope::Services;
    config_b.proxy.services = vec!["channel.telegram".into()];
    config_b.proxy.http_proxy = Some("http://selected-b.invalid:8080".into());
    config_b.proxy.no_proxy = vec!["b.internal".into()];

    let calls = Arc::new(AtomicUsize::new(0));
    let probe = Arc::new(FixedHealth {
        result: Ok(Some(vec![ChannelHealthReport {
            channel: "telegram".into(),
            state: ChannelHealthState::Healthy,
        }])),
        calls: Arc::clone(&calls),
        readiness_checks: Arc::new(AtomicUsize::new(0)),
        policy: Default::default(),
        workspace: Default::default(),
    });
    let check = ChannelHealthCheck::with_probe("telegram", probe.clone());
    let ambient_context = crate::core::runtime::CoreContext::for_test_with_config(
        crate::core::runtime::DomainSet::full(),
        config_a,
    );
    let finding = crate::core::runtime::CoreContext::scope(ambient_context, async {
        let selected_context = DoctorCtx::from_config(config_b.clone()).with_components(
            crate::core::runtime::ComponentSet::from_config(&config_b.components),
        );
        check
            .detect(&selected_context)
            .await
            .into_iter()
            .next()
            .expect("healthy selected provider produces a finding")
    })
    .await;

    assert_eq!(finding.check_id, "channels/telegram");
    assert_eq!(finding.severity, FindingSeverity::Pass);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(probe.readiness_checks.load(Ordering::SeqCst), 1);
    assert_eq!(
        *probe.workspace.lock().unwrap(),
        Some(config_b.workspace_dir.clone())
    );
    let policy = probe.policy.lock().unwrap().clone().unwrap();
    assert_eq!(
        policy.http_proxy.as_deref(),
        Some("http://selected-b.invalid:8080")
    );
    assert_eq!(policy.service_selectors, vec!["channel.telegram"]);
    assert_eq!(policy.no_proxy, vec!["b.internal"]);
}

#[test]
fn channel_labels_use_definition_display_names_and_fall_back_for_unknown_ids() {
    assert_eq!(channel_label("telegram"), "Telegram");
    assert_eq!(channel_label("not-a-channel"), "not-a-channel");
}

#[tokio::test]
async fn configured_channel_ids_come_from_selected_b_not_ambient_a() {
    let ambient = tempfile::tempdir().expect("create ambient config fixture");
    let selected = tempfile::tempdir().expect("create selected config fixture");
    let mut config_a = config_in(ambient.path());
    config_a.channels_config.imessage = Some(tinychannels_bus::config::IMessageConfig {
        allowed_contacts: vec![],
    });
    let mut config_b = config_in(selected.path());
    config_b.channels_config.telegram = Some(telegram());
    config_b.components.channels = false;
    let selected_components = crate::core::runtime::ComponentSet::from_config(&config_b.components);

    let ambient_context = crate::core::runtime::CoreContext::for_test_with_config(
        crate::core::runtime::DomainSet::full(),
        config_a,
    );
    let ids = crate::core::runtime::CoreContext::scope(ambient_context, async {
        let selected_ctx =
            DoctorCtx::from_config(config_b.clone()).with_components(selected_components);
        for_context(&selected_ctx)
            .iter()
            .map(|check| check.id().to_owned())
            .collect::<Vec<_>>()
    })
    .await;

    assert_eq!(ids, vec!["channels/telegram"]);
}

#[tokio::test]
async fn no_configured_channels_register_no_channel_checks() {
    let scratch = tempfile::tempdir().expect("create isolated config fixture");
    let config = config_in(scratch.path());
    assert!(for_context(&DoctorCtx::from_config(config)).is_empty());
}

#[cfg(feature = "modules")]
#[tokio::test]
async fn disabled_module_configuration_skips_without_starting_or_loading_the_module() {
    let scratch = tempfile::tempdir().expect("create isolated config fixture");
    let mut config = config_in(scratch.path());
    config.modules.enabled = false;
    config.channels_config.imessage = Some(tinychannels_bus::config::IMessageConfig {
        allowed_contacts: vec![],
    });
    let was_started = crate::modules::runtime::is_started();
    let result = ExistingModuleProbe
        .check(&config, "imessage", proxy_policy(&config, "imessage"))
        .await;

    assert_eq!(result.unwrap(), None);
    assert_eq!(crate::modules::runtime::is_started(), was_started);
}
