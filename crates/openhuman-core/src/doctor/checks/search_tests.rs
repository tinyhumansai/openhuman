use super::*;
use crate::config::{
    PresentationMode, ProviderRoute, SearchEngineCredentials, SearchProviderSettings,
};
use crate::doctor::contract::{DoctorCtx, FindingSeverity};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

struct FixedManagedResolver {
    managed_available: bool,
    calls: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl SearchProviderResolver for FixedManagedResolver {
    async fn resolve(
        &self,
        config: &crate::config::Config,
    ) -> Vec<crate::search::providers::ResolvedProvider> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        crate::search::providers::resolve_with(config, self.managed_available)
    }
}

fn config_in(root: &std::path::Path) -> crate::config::Config {
    let mut config = crate::config::Config::default();
    config.workspace_dir = root.join("workspace");
    config.action_dir = root.join("actions");
    config.config_path = root.join("config.toml");
    config.search.providers.insert(
        "brave".into(),
        SearchProviderSettings {
            enabled: true,
            route: ProviderRoute::Direct,
        },
    );
    config
}

fn backend_config_in(root: &std::path::Path) -> crate::config::Config {
    let mut config = config_in(root);
    config.resolved_endpoints.control_plane_url = Some("https://backend-b.example".into());
    config.secrets.encrypt = false;
    config
}

fn write_session_profile(root: &std::path::Path, expires_at: &str, issuer: &str) {
    let now = chrono::Utc::now().to_rfc3339();
    std::fs::write(
        root.join("auth-profiles.json"),
        serde_json::json!({
            "schema_version": 1,
            "updated_at": now,
            "active_profiles": {"app-session": "app-session:default"},
            "profiles": {
                "app-session:default": {
                    "provider": "app-session",
                    "profile_name": "default",
                    "kind": "token",
                    "token": "doctor-fake-session-token",
                    "metadata": {
                        "session_issuing_backend": issuer,
                        "session_expires_at": expires_at
                    }
                }
            }
        })
        .to_string(),
    )
    .expect("write fake session profile");
}

fn write_api_key_profile(root: &std::path::Path) {
    let now = chrono::Utc::now().to_rfc3339();
    std::fs::write(
        root.join("auth-profiles.json"),
        serde_json::json!({
            "schema_version": 1,
            "updated_at": now,
            "active_profiles": {"api-key": "api-key:default"},
            "profiles": {
                "api-key:default": {
                    "provider": "api-key",
                    "profile_name": "default",
                    "kind": "token",
                    "token": "doctor-fake-managed-api-key",
                    "metadata": {"kind": "tinyhumans-api-key"}
                }
            }
        })
        .to_string(),
    )
    .expect("write fake API key profile");
}

fn drain_session_expiry(
    receiver: &mut tinybus::EventReceiver<crate::core::events::DomainEvent>,
) -> bool {
    loop {
        match receiver.try_recv() {
            Ok(crate::core::events::DomainEvent::SessionExpired { .. }) => return true,
            Ok(_) | Err(tinybus::TryRecvError::Lagged(_)) => continue,
            Err(_) => return false,
        }
    }
}

#[tokio::test]
async fn needs_key_is_a_warning_with_the_provider_config_key_in_its_hint() {
    let ambient = tempfile::tempdir().expect("create ambient config fixture");
    let selected = tempfile::tempdir().expect("create selected config fixture");
    let mut config_a = config_in(ambient.path());
    config_a.search.brave.api_key = Some("doctor-fake-key-a".into());
    let config_b = config_in(selected.path());
    let resolver_calls = Arc::new(AtomicUsize::new(0));
    let check = SearchProviderCheck::with_resolver(
        "brave",
        Arc::new(FixedManagedResolver {
            managed_available: false,
            calls: Arc::clone(&resolver_calls),
        }),
    );
    let ambient_context = crate::core::runtime::CoreContext::for_test_with_config(
        crate::core::runtime::DomainSet::full(),
        config_a,
    );
    let finding = crate::core::runtime::CoreContext::scope(ambient_context, async {
        check
            .detect(&DoctorCtx::from_config(config_b))
            .await
            .into_iter()
            .next()
            .expect("NeedsKey produces a finding")
    })
    .await;

    assert_eq!(finding.check_id, "search/brave");
    assert_eq!(finding.severity, FindingSeverity::Warn);
    assert!(finding.fix_hint.contains("search.brave.api_key"));
    assert!(!finding.message.contains("doctor-fake-key-a"));
    assert_eq!(resolver_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_selected_direct_key_makes_the_provider_ready_without_any_network_call() {
    let scratch = tempfile::tempdir().expect("create isolated config fixture");
    let mut config = config_in(scratch.path());
    config.search.brave.api_key = Some("doctor-fake-key-b".into());
    let resolver_calls = Arc::new(AtomicUsize::new(0));
    let check = SearchProviderCheck::with_resolver(
        "brave",
        Arc::new(FixedManagedResolver {
            managed_available: false,
            calls: Arc::clone(&resolver_calls),
        }),
    );

    let finding = check
        .detect(&DoctorCtx::from_config(config))
        .await
        .into_iter()
        .next()
        .expect("ready provider produces a finding");

    assert_eq!(finding.check_id, "search/brave");
    assert_eq!(finding.severity, FindingSeverity::Pass);
    assert!(!finding.message.contains("doctor-fake-key-b"));
    assert_eq!(resolver_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_live_selected_b_session_enables_managed_search_without_refreshing_it() {
    let ambient = tempfile::tempdir().expect("create ambient credential fixture");
    let selected = tempfile::tempdir().expect("create selected credential fixture");
    let now = chrono::Utc::now();
    write_session_profile(
        ambient.path(),
        &(now - chrono::Duration::hours(1)).to_rfc3339(),
        "https://backend-a.example",
    );
    write_session_profile(
        selected.path(),
        &(now + chrono::Duration::hours(1)).to_rfc3339(),
        "https://backend-b.example",
    );
    let config_a = backend_config_in(ambient.path());
    let mut config_b = backend_config_in(selected.path());
    config_b
        .search
        .providers
        .insert("exa".into(), SearchProviderSettings::managed());
    let selected_profile_path = selected.path().join("auth-profiles.json");
    let selected_profile_before = std::fs::read(&selected_profile_path).unwrap();
    let ambient_profile_path = ambient.path().join("auth-profiles.json");
    let ambient_profile_before = std::fs::read(&ambient_profile_path).unwrap();
    let check = SearchProviderCheck::new("exa");
    let ambient_context = crate::core::runtime::CoreContext::for_test_with_config(
        crate::core::runtime::DomainSet::full(),
        config_a,
    );

    let finding = crate::core::runtime::CoreContext::scope(ambient_context, async {
        check
            .detect(&DoctorCtx::from_config(config_b.clone()))
            .await
            .into_iter()
            .next()
            .expect("live selected-B managed provider produces a finding")
    })
    .await;

    assert_eq!(finding.check_id, "search/exa");
    assert_eq!(finding.severity, FindingSeverity::Pass);
    assert!(!finding.message.contains("doctor-fake-session-token"));
    assert_eq!(
        std::fs::read(&selected_profile_path).unwrap(),
        selected_profile_before
    );
    assert_eq!(
        std::fs::read(&ambient_profile_path).unwrap(),
        ambient_profile_before
    );
    assert!(!selected.path().join("auth-profiles.lock").exists());
    assert!(!ambient.path().join("auth-profiles.lock").exists());
    assert!(!config_b.workspace_dir.exists());
}

#[tokio::test]
async fn an_expired_or_wrong_backend_session_does_not_make_managed_search_ready() {
    let scratch = tempfile::tempdir().expect("create isolated credential fixture");
    let now = chrono::Utc::now();
    let mut config = backend_config_in(scratch.path());
    config
        .search
        .providers
        .insert("exa".into(), SearchProviderSettings::managed());
    let profile_path = scratch.path().join("auth-profiles.json");
    let check = SearchProviderCheck::new("exa");
    crate::core::bus::init()
        .await
        .expect("initialize event bus to observe forbidden expiry publication");
    let mut events = crate::core::bus::BUS
        .get()
        .expect("event bus initialized")
        .receiver();
    let missing = check
        .detect(&DoctorCtx::from_config(config.clone()))
        .await
        .into_iter()
        .next()
        .expect("missing managed credential produces an advisory finding");
    assert_ne!(missing.severity, FindingSeverity::Pass);
    assert!(!missing.message.contains("doctor-fake"));

    write_session_profile(
        scratch.path(),
        &(now - chrono::Duration::hours(1)).to_rfc3339(),
        "https://backend-b.example",
    );
    let expired_before = std::fs::read(&profile_path).unwrap();
    let expired = check
        .detect(&DoctorCtx::from_config(config.clone()))
        .await
        .into_iter()
        .next()
        .expect("expired managed credential produces an advisory finding");
    assert_ne!(expired.severity, FindingSeverity::Pass);
    assert!(!expired.message.contains("doctor-fake-session-token"));
    assert!(!expired.fix_hint.trim().is_empty());
    assert_eq!(std::fs::read(&profile_path).unwrap(), expired_before);
    assert!(!scratch.path().join("auth-profiles.lock").exists());
    assert!(!config.workspace_dir.exists());
    assert!(
        !drain_session_expiry(&mut events),
        "doctor checks expiry without publishing a session-expired event"
    );

    write_session_profile(
        scratch.path(),
        &(now + chrono::Duration::hours(1)).to_rfc3339(),
        "https://backend-a.example",
    );
    let mismatch_before = std::fs::read(&profile_path).unwrap();
    let mismatched = check
        .detect(&DoctorCtx::from_config(config))
        .await
        .into_iter()
        .next()
        .expect("wrong-backend session produces an advisory finding");
    assert_ne!(mismatched.severity, FindingSeverity::Pass);
    assert!(!mismatched.message.contains("doctor-fake-session-token"));
    assert_eq!(std::fs::read(&profile_path).unwrap(), mismatch_before);
}

#[tokio::test]
async fn a_selected_api_key_is_a_valid_managed_credential() {
    let scratch = tempfile::tempdir().expect("create isolated credential fixture");
    write_api_key_profile(scratch.path());
    let mut config = backend_config_in(scratch.path());
    config
        .search
        .providers
        .insert("exa".into(), SearchProviderSettings::managed());
    let profile_path = scratch.path().join("auth-profiles.json");
    let bytes_before = std::fs::read(&profile_path).unwrap();
    let finding = SearchProviderCheck::new("exa")
        .detect(&DoctorCtx::from_config(config.clone()))
        .await
        .into_iter()
        .next()
        .expect("selected API key enables managed search");
    assert_eq!(finding.severity, FindingSeverity::Pass);
    assert!(!finding.message.contains("doctor-fake-managed-api-key"));
    assert_eq!(std::fs::read(&profile_path).unwrap(), bytes_before);
    assert!(!scratch.path().join("auth-profiles.lock").exists());
    assert!(!config.workspace_dir.exists());
}

#[tokio::test]
async fn profile_search_binding_is_resolved_without_operator_session_credentials() {
    let scratch = tempfile::tempdir().expect("create isolated profile fixture");
    let mut config = backend_config_in(scratch.path());
    config
        .search
        .providers
        .insert("exa".into(), SearchProviderSettings::managed());
    let mut saas = crate::core::runtime::SaasConfig::new(scratch.path());
    let mut search = crate::config::SearchConfig::default();
    search.presentation = PresentationMode::Roles;
    search.providers = std::collections::BTreeMap::from([
        ("exa".into(), SearchProviderSettings::direct()),
        ("gemini".into(), SearchProviderSettings::direct()),
    ]);
    search.roles = std::collections::BTreeMap::from([
        ("search".into(), vec!["exa".into(), "gemini".into()]),
        ("answer".into(), vec!["gemini".into()]),
        ("contents".into(), vec!["exa".into()]),
    ]);
    search.exa = SearchEngineCredentials {
        api_key: Some("doctor-profile-exa-key".into()),
        base_url: Some("http://127.0.0.1:1234".into()),
    };
    search.gemini = SearchEngineCredentials {
        api_key: Some("doctor-profile-gemini-key".into()),
        base_url: Some("http://127.0.0.1:2345".into()),
    };
    saas.search = Some(search);
    crate::profiles::provider_settings::ProfileProviderSettings::from_saas(&saas)
        .expect("valid direct profile binding")
        .bind(&mut config);
    let finding = SearchProviderCheck::new("exa")
        .detect(&DoctorCtx::from_config(config.clone()))
        .await
        .into_iter()
        .next()
        .expect("selected profile binding supplies the Exa direct key");
    assert_eq!(finding.severity, FindingSeverity::Pass);
    assert!(!finding.message.contains("doctor-profile-exa-key"));
    assert!(!finding.message.contains("doctor-profile-gemini-key"));
    assert!(!config.workspace_dir.exists());
}
