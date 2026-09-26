use super::*;

fn config_in(dir: &std::path::Path) -> Config {
    let mut config = Config::default();
    config.config_path = dir.join("config.toml");
    config.workspace_dir = dir.join("workspace");
    config.secrets.encrypt = false;
    config
}

#[test]
fn direct_provider_config_keeps_limits_and_private_key() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = config_in(dir.path());
    config.search.enabled_providers = Some(["parallel".to_owned()].into());
    config.search.parallel.api_key = Some("test-secret".into());
    config.search.max_results = 12;
    config.search.timeout_secs = 22;
    let payload = module_config(&config);
    let parallel = &payload.providers["parallel"];
    assert!(parallel.enabled);
    assert_eq!(parallel.route, ProviderRoute::Direct);
    assert_eq!(parallel.credential.as_deref(), Some("test-secret"));
    assert_eq!(parallel.max_results, Some(12));
    assert_eq!(parallel.timeout_secs, Some(22));
    assert!(!format!("{payload:?}").contains("test-secret"));
}

#[test]
fn synchronous_tool_specs_follow_provider_selection() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = config_in(dir.path());
    config.search.enabled_providers = Some(["parallel".to_owned()].into());
    config.search.parallel.api_key = Some("test-secret".into());
    let specs = configured_tool_specs(&config);
    assert!(specs.iter().any(|spec| spec.name == "parallel_search"));
    config.search.enabled_providers = Some(Default::default());
    assert!(configured_tool_specs(&config).is_empty());
}

#[test]
fn managed_presentation_targets_backend_parallel() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = config_in(dir.path());
    config.search.presentation = "one_provider".into();
    config.search.presentation_provider = Some("managed".into());
    let payload = module_config(&config);
    assert_eq!(payload.presentation.provider.as_deref(), Some("parallel"));
}

#[test]
fn managed_route_survives_parallel_selection_without_direct_key() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = config_in(dir.path());
    crate::security::credentials::api_key::store_api_key(&config, "backend-sentinel").unwrap();
    config.search.enabled_providers = Some(["managed".into(), "parallel".into()].into());
    let payload = module_config(&config);
    assert!(payload.providers["parallel"].enabled);
    assert_eq!(payload.providers["parallel"].route, ProviderRoute::Backend);
    assert!(payload.providers["parallel"].credential.is_none());
}

#[test]
fn explicit_tinyfish_selection_overrides_legacy_toggle() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = config_in(dir.path());
    config.integrations.tinyfish.enabled = false;
    config.search.enabled_providers = Some(["tinyfish".into()].into());
    assert!(module_config(&config).providers["tinyfish"].enabled);
    config.search.enabled_providers = None;
    assert!(!module_config(&config).providers["tinyfish"].enabled);
}

#[tokio::test]
async fn configured_calls_keep_the_lock_through_invocation() {
    use std::sync::{Arc, Mutex};
    let active = Arc::new(Mutex::new(String::new()));
    let (a_entered_tx, a_entered_rx) = tokio::sync::oneshot::channel();
    let (release_a_tx, release_a_rx) = tokio::sync::oneshot::channel();
    let a_active = active.clone();
    let a = tokio::spawn(async move {
        with_module_lock(|| async move {
            *a_active.lock().unwrap() = "A".into();
            a_entered_tx.send(()).unwrap();
            release_a_rx.await.unwrap();
            assert_eq!(*a_active.lock().unwrap(), "A");
            Ok::<_, String>(())
        })
        .await
        .unwrap();
    });
    a_entered_rx.await.unwrap();
    let b_active = active.clone();
    let (b_started_tx, b_started_rx) = tokio::sync::oneshot::channel();
    let b = tokio::spawn(async move {
        b_started_tx.send(()).unwrap();
        with_module_lock(|| async move {
            *b_active.lock().unwrap() = "B".into();
            Ok::<_, String>(())
        })
        .await
        .unwrap();
    });
    b_started_rx.await.unwrap();
    tokio::task::yield_now().await;
    assert_eq!(*active.lock().unwrap(), "A");
    release_a_tx.send(()).unwrap();
    a.await.unwrap();
    b.await.unwrap();
    assert_eq!(*active.lock().unwrap(), "B");
}

#[test]
fn gemini_deep_research_keeps_direct_key_on_backend_grounded_route() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = config_in(dir.path());
    config.search.enabled_providers = Some(["gemini".to_owned()].into());
    config.search.gemini_route = "backend".into();
    config.search.gemini.api_key = Some("test-secret".into());
    let payload = module_config(&config);
    let deep = &payload.providers["gemini_deep_research"];
    assert!(deep.enabled);
    assert_eq!(deep.route, ProviderRoute::Direct);
    assert_eq!(deep.credential.as_deref(), Some("test-secret"));
}

#[test]
fn backend_api_key_uses_x_api_key_mode() {
    let dir = tempfile::tempdir().unwrap();
    let config = config_in(dir.path());
    crate::security::credentials::api_key::store_api_key(&config, "test-backend-key").unwrap();
    let payload = module_config(&config);
    assert_eq!(payload.backend.auth_mode, BackendAuthMode::ApiKey);
    assert_eq!(
        payload.backend.credential.as_deref(),
        Some("test-backend-key")
    );
    assert!(!format!("{payload:?}").contains("test-backend-key"));
}

#[test]
fn explicit_provider_set_controls_separate_search_engines() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = config_in(dir.path());
    config.seltz.enabled = true;
    config.seltz.api_key = Some("test-secret".into());
    config.searxng.enabled = true;
    config.search.enabled_providers = Some(Default::default());
    let payload = module_config(&config);
    assert!(!payload.providers["seltz"].enabled);
    assert!(!payload.providers["searxng"].enabled);
    config.search.enabled_providers = Some(["seltz".to_owned()].into());
    let payload = module_config(&config);
    assert!(payload.providers["seltz"].enabled);
    assert!(!payload.providers["searxng"].enabled);
}

#[test]
fn legacy_disabled_removes_all_module_tools() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = config_in(dir.path());
    config.search.engine = "disabled".into();
    config.search.parallel.api_key = Some("test-secret".into());
    let payload = module_config(&config);
    assert!(!payload.enabled);
    assert!(payload.providers.values().all(|provider| !provider.enabled));
}
