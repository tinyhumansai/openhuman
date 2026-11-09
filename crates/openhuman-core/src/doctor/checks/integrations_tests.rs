use super::*;
use crate::doctor::contract::{DoctorCtx, FindingSeverity};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

struct ExistingList {
    expected_workspace: std::path::PathBuf,
    result: Result<Option<()>, ()>,
    calls: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl ComposioProbe for ExistingList {
    async fn list_existing(&self, config: &crate::config::Config) -> Result<Option<()>, ()> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert_eq!(config.workspace_dir, self.expected_workspace);
        self.result
    }
}

fn config_in(root: &std::path::Path) -> crate::config::Config {
    let mut config = crate::config::Config::default();
    config.workspace_dir = root.join("workspace");
    config.action_dir = root.join("actions");
    config.config_path = root.join("config.toml");
    config.composio.host_credential = Some(
        crate::config::schema::ComposioHostCredential::direct("doctor-fake-key")
            .entity_id("doctor-fake-entity"),
    );
    config
}

#[tokio::test]
async fn an_existing_composio_connection_list_passes_without_exposing_credentials() {
    let scratch = tempfile::tempdir().expect("create isolated config fixture");
    let config = config_in(scratch.path());
    let calls = Arc::new(AtomicUsize::new(0));
    let check = ComposioCheck::with_probe(Arc::new(ExistingList {
        expected_workspace: config.workspace_dir.clone(),
        result: Ok(Some(())),
        calls: Arc::clone(&calls),
    }));

    let finding = check
        .detect(&DoctorCtx::from_config(config))
        .await
        .remove(0);
    assert_eq!(finding.check_id, "integrations/composio");
    assert_eq!(finding.severity, FindingSeverity::Pass);
    assert!(!finding.message.contains("doctor-fake-key"));
    assert!(!finding.message.contains("doctor-fake-entity"));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn selected_b_does_not_reuse_a_ready_or_cached_a_connection_list() {
    let ambient = tempfile::tempdir().expect("create ambient fixture");
    let selected = tempfile::tempdir().expect("create selected fixture");
    let config_a = config_in(ambient.path());
    let config_b = config_in(selected.path());
    let calls = Arc::new(AtomicUsize::new(0));
    let check = ComposioCheck::with_probe(Arc::new(ExistingList {
        expected_workspace: config_b.workspace_dir.clone(),
        result: Ok(None),
        calls: Arc::clone(&calls),
    }));
    let ambient_context = crate::core::runtime::CoreContext::for_test_with_config(
        crate::core::runtime::DomainSet::full(),
        config_a,
    );
    let finding = crate::core::runtime::CoreContext::scope(ambient_context, async {
        check
            .detect(&DoctorCtx::from_config(config_b))
            .await
            .remove(0)
    })
    .await;

    assert_eq!(finding.check_id, "integrations/composio");
    assert_eq!(finding.severity, FindingSeverity::Skipped);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn an_existing_connector_failure_is_reported_without_raw_module_text() {
    let scratch = tempfile::tempdir().expect("create isolated fixture");
    let config = config_in(scratch.path());
    let calls = Arc::new(AtomicUsize::new(0));
    let check = ComposioCheck::with_probe(Arc::new(ExistingList {
        expected_workspace: config.workspace_dir.clone(),
        result: Err(()),
        calls: Arc::clone(&calls),
    }));
    let finding = check
        .detect(&DoctorCtx::from_config(config))
        .await
        .remove(0);
    assert_eq!(finding.severity, FindingSeverity::Error);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[cfg(feature = "modules")]
#[tokio::test]
async fn the_default_probe_skips_a_disabled_connector_without_starting_the_runtime() {
    let scratch = tempfile::tempdir().expect("create isolated fixture");
    let mut config = config_in(scratch.path());
    config.modules.enabled = false;
    let started_before = crate::modules::runtime::is_started();

    let result = ExistingConnectorProbe.list_existing(&config).await;

    assert!(matches!(result, Ok(None)));
    assert_eq!(crate::modules::runtime::is_started(), started_before);
}
