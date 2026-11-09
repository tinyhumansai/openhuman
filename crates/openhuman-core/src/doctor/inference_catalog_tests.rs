use super::{
    ops::{doctor_models, doctor_report_with_options},
    DoctorOptions, FindingSeverity, ModelProbeOutcome,
};
use crate::config::schema::cloud_providers::{AuthStyle, CloudProviderCreds};
use crate::config::Config;
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::Duration;
use tinyinference_llm::providers::overrides::{CatalogEntry, CatalogListing};

#[derive(Clone)]
struct CatalogServerState {
    status: StatusCode,
    body: serde_json::Value,
    delay: Duration,
    requests: Arc<AtomicUsize>,
    authorizations: Arc<std::sync::Mutex<Vec<Option<String>>>>,
}

async fn catalog_handler(State(state): State<CatalogServerState>, headers: HeaderMap) -> Response {
    state.requests.fetch_add(1, Ordering::SeqCst);
    if !state.delay.is_zero() {
        tokio::time::sleep(state.delay).await;
    }
    state
        .authorizations
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .push(
            headers
                .get("authorization")
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned),
        );
    (state.status, Json(state.body)).into_response()
}

async fn missing_catalog_handler(State(state): State<CatalogServerState>) -> Response {
    state.requests.fetch_add(1, Ordering::SeqCst);
    StatusCode::NOT_FOUND.into_response()
}

async fn spawn_catalog_server(
    status: StatusCode,
    body: serde_json::Value,
    has_models_route: bool,
) -> (String, CatalogServerState) {
    spawn_catalog_server_with_delay(status, body, has_models_route, Duration::ZERO).await
}

async fn spawn_catalog_server_with_delay(
    status: StatusCode,
    body: serde_json::Value,
    has_models_route: bool,
    delay: Duration,
) -> (String, CatalogServerState) {
    let state = CatalogServerState {
        status,
        body,
        delay,
        requests: Arc::new(AtomicUsize::new(0)),
        authorizations: Arc::new(std::sync::Mutex::new(Vec::new())),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind local catalog server");
    let address = listener.local_addr().expect("local server address");
    let app = if has_models_route {
        Router::new().route("/v1/models", get(catalog_handler))
    } else {
        Router::new()
            .route("/v1/health", get(catalog_handler))
            .fallback(get(missing_catalog_handler))
    };
    let app = app.with_state(state.clone());
    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve local catalog");
    });
    (format!("http://{address}/v1"), state)
}

fn temporary_config() -> (tempfile::TempDir, Config) {
    let root = tempfile::tempdir().expect("temporary doctor fixture");
    let mut config = Config {
        config_path: root.path().join("config.toml"),
        workspace_dir: root.path().join("workspace"),
        action_dir: root.path().join("workspace"),
        ..Config::default()
    };
    config.cloud_providers = Vec::new();
    config.default_model = None;
    config.model_routes.clear();
    (root, config)
}

fn config_for_catalog(
    endpoint: String,
    provider_id: &str,
    slug: &str,
) -> (tempfile::TempDir, Config) {
    let (root, mut config) = temporary_config();
    config.secrets.encrypt = false;
    config.cloud_providers = vec![CloudProviderCreds {
        id: provider_id.to_owned(),
        slug: slug.to_owned(),
        label: "Doctor catalog fixture".to_owned(),
        endpoint,
        auth_style: AuthStyle::None,
        default_model: None,
    }];
    (root, config)
}

fn catalog_body(ids: &[&str]) -> serde_json::Value {
    serde_json::json!({
        "data": ids.iter().map(|id| serde_json::json!({ "id": id })).collect::<Vec<_>>()
    })
}

fn store_provider_key(config: &Config, slug: &str, token: &str) {
    let provider = crate::inference::provider::factory::auth_key_for_slug(slug);
    let profile_id = format!("{provider}:default");
    let now = chrono::Utc::now().to_rfc3339();
    let mut active_profiles = std::collections::HashMap::new();
    active_profiles.insert(provider.clone(), profile_id.clone());
    let mut profiles = std::collections::HashMap::new();
    profiles.insert(
        profile_id,
        serde_json::json!({
            "provider": provider,
            "profile_name": "default",
            "kind": "token",
            "token": token,
            "metadata": {},
            "created_at": now,
            "updated_at": now,
        }),
    );
    let document = serde_json::json!({
        "schema_version": 1,
        "updated_at": now,
        "active_profiles": active_profiles,
        "profiles": profiles,
    });
    std::fs::write(
        auth_profiles_path(config),
        serde_json::to_vec(&document).unwrap(),
    )
    .expect("seed read-only provider credential fixture");
}

fn auth_profiles_path(config: &Config) -> std::path::PathBuf {
    config
        .config_path
        .parent()
        .expect("fixture config parent")
        .join("auth-profiles.json")
}

fn profile_context(
    config: Config,
    profile_id: &str,
) -> std::sync::Arc<crate::core::runtime::CoreContext> {
    use crate::core::runtime::{ContextOverlay, CoreContext, DomainSet};

    CoreContext::for_test(DomainSet::full(), None).derive_with(
        ContextOverlay::new(
            config,
            DomainSet::full(),
            crate::tools::toolpacks::ToolGroups::default(),
        )
        .profile(profile_id),
    )
}

fn workspace_context(config: Config) -> std::sync::Arc<crate::core::runtime::CoreContext> {
    use crate::core::runtime::{ContextOverlay, CoreContext, DomainSet};

    CoreContext::for_test(DomainSet::full(), None).derive_with(ContextOverlay::new(
        config,
        DomainSet::full(),
        crate::tools::toolpacks::ToolGroups::default(),
    ))
}

fn profile_context_with_extensions(
    config: Config,
    profile_id: &str,
    extensions: crate::core::slots::Extensions,
) -> std::sync::Arc<crate::core::runtime::CoreContext> {
    use crate::core::runtime::{ContextOverlay, CoreContext, DomainSet};

    CoreContext::for_test_with_extensions(DomainSet::full(), None, extensions).derive_with(
        ContextOverlay::new(
            config,
            DomainSet::full(),
            crate::tools::toolpacks::ToolGroups::default(),
        )
        .profile(profile_id),
    )
}

fn register_test_inspection_catalog(
    extensions: &crate::core::slots::Extensions,
    provider: &str,
    endpoint: String,
) -> Arc<AtomicUsize> {
    use crate::inference::registry::{
        register_inspection_catalog_in, InspectionCatalogError, InspectionCatalogListing,
        PreparedInspectionCatalog,
    };

    let requests = Arc::new(AtomicUsize::new(0));
    let request_count = Arc::clone(&requests);
    let provider_id = provider.to_owned();
    let listing: InspectionCatalogListing = Arc::new(move |config, request| {
        let endpoint = endpoint.clone();
        let provider_id = provider_id.clone();
        let request_count = Arc::clone(&request_count);
        Box::pin(async move {
            assert!(
                request.provider_key.is_empty(),
                "fixture uses the separate config credential identity"
            );
            let credential = config.api_key.clone();
            let cache_endpoint = endpoint.clone();
            let client = request.client.clone();
            let fetch_endpoint = endpoint.clone();
            let fetch_credential = credential.clone();
            let fetch_count = Arc::clone(&request_count);
            let fetch = Box::pin(async move {
                fetch_count.fetch_add(1, Ordering::SeqCst);
                let mut get = client.get(fetch_endpoint);
                if let Some(credential) = fetch_credential.as_deref() {
                    get = get.bearer_auth(credential);
                }
                let response = get
                    .send()
                    .await
                    .map_err(|_| InspectionCatalogError::Unavailable)?;
                let status = response.status();
                if status == StatusCode::NOT_FOUND {
                    return Err(InspectionCatalogError::NoCatalog);
                }
                if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
                    return Err(InspectionCatalogError::AuthOrAccess);
                }
                if !status.is_success() {
                    return Err(InspectionCatalogError::Failure);
                }
                let body = response
                    .json::<serde_json::Value>()
                    .await
                    .map_err(|_| InspectionCatalogError::Failure)?;
                let models = body
                    .get("data")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!([]));
                Ok(crate::core::Outcome::new(
                    serde_json::json!({ "models": models }),
                    Vec::new(),
                ))
            });
            Ok(PreparedInspectionCatalog::new(
                &provider_id,
                &cache_endpoint,
                credential.as_deref(),
                fetch,
            ))
        })
    });
    register_inspection_catalog_in(extensions, provider, listing);
    requests
}

async fn report_for_profile(
    config: &Config,
    profile_id: &str,
    options: DoctorOptions,
) -> crate::core::Outcome<super::DoctorReport> {
    report_for_context(config, profile_context(config.clone(), profile_id), options).await
}

async fn report_for_context(
    config: &Config,
    context: std::sync::Arc<crate::core::runtime::CoreContext>,
    options: DoctorOptions,
) -> crate::core::Outcome<super::DoctorReport> {
    crate::core::runtime::CoreContext::scope(context, async {
        doctor_report_with_options(config, options).await
    })
    .await
    .expect("doctor report for selected profile")
}

#[tokio::test]
async fn doctor_models_lists_configured_catalogs_and_honors_use_cache() {
    let (endpoint, server) =
        spawn_catalog_server(StatusCode::OK, catalog_body(&["present-model"]), true).await;
    let (_temp, config) = config_for_catalog(endpoint, "p_doctor_497_cache", "doctor-497-cache");

    let first = doctor_models(&config, true)
        .await
        .expect("a configured provider catalog should be inspectable");
    let second = doctor_models(&config, true)
        .await
        .expect("a cached catalog should remain available");
    assert_eq!(first.value.entries.len(), 1);
    assert_eq!(first.value.entries[0].provider, "doctor-497-cache");
    assert_eq!(first.value.entries[0].outcome, ModelProbeOutcome::Ok);
    assert_eq!(second.value.entries[0].outcome, ModelProbeOutcome::Ok);
    assert_eq!(server.requests.load(Ordering::SeqCst), 1);

    doctor_models(&config, false)
        .await
        .expect("use_cache=false should refresh the provider catalog");
    assert_eq!(server.requests.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn doctor_models_does_not_reuse_a_catalog_for_another_profiles_credential() {
    let (endpoint, server) =
        spawn_catalog_server(StatusCode::OK, catalog_body(&["configured-model"]), true).await;
    let (_temp_a, mut config_a) = config_for_catalog(
        endpoint.clone(),
        "p_doctor_497_selected",
        "doctor-497-selected",
    );
    let (_temp_b, mut config_b) =
        config_for_catalog(endpoint, "p_doctor_497_selected", "doctor-497-selected");
    config_a.cloud_providers[0].auth_style = AuthStyle::Bearer;
    config_b.cloud_providers[0].auth_style = AuthStyle::Bearer;
    store_provider_key(&config_a, "doctor-497-selected", "fixture-token-profile-a");
    store_provider_key(&config_b, "doctor-497-selected", "fixture-token-profile-b");
    let auth_file_a_before = std::fs::read(auth_profiles_path(&config_a)).expect("profile A file");
    let auth_file_b_before = std::fs::read(auth_profiles_path(&config_b)).expect("profile B file");

    doctor_models(&config_a, true)
        .await
        .expect("first profile catalog should be available");
    assert_eq!(
        std::fs::read(auth_profiles_path(&config_a)).unwrap(),
        auth_file_a_before,
        "profile A credential bytes are unchanged after its probe"
    );
    assert_eq!(
        std::fs::read(auth_profiles_path(&config_b)).unwrap(),
        auth_file_b_before,
        "profile B credential bytes are unchanged while A is probed"
    );
    doctor_models(&config_b, true)
        .await
        .expect("selected profile catalog should use its own credential");
    assert_eq!(
        std::fs::read(auth_profiles_path(&config_a)).unwrap(),
        auth_file_a_before,
        "profile A credential bytes are unchanged while B is probed"
    );
    assert_eq!(
        std::fs::read(auth_profiles_path(&config_b)).unwrap(),
        auth_file_b_before,
        "profile B credential bytes are unchanged after its probe"
    );
    doctor_models(&config_b, true)
        .await
        .expect("the selected profile may reuse its own cached catalog");
    assert_eq!(
        std::fs::read(auth_profiles_path(&config_b)).unwrap(),
        auth_file_b_before,
        "profile B credential bytes are unchanged after its cache hit"
    );

    assert_eq!(server.requests.load(Ordering::SeqCst), 2);
    assert_eq!(
        *server
            .authorizations
            .lock()
            .unwrap_or_else(|error| error.into_inner()),
        vec![
            Some("Bearer fixture-token-profile-a".to_owned()),
            Some("Bearer fixture-token-profile-b".to_owned())
        ]
    );
    assert_eq!(
        std::fs::read(auth_profiles_path(&config_a)).unwrap(),
        auth_file_a_before
    );
    assert_eq!(
        std::fs::read(auth_profiles_path(&config_b)).unwrap(),
        auth_file_b_before
    );
}

#[tokio::test]
async fn prepared_catalog_identity_changes_when_managed_credential_rotates_in_place() {
    let (endpoint, server) =
        spawn_catalog_server(StatusCode::OK, catalog_body(&["managed-model"]), true).await;
    let (_temp, mut config_a) = temporary_config();
    config_a.api_key = Some("fixture-managed-credential-a".into());
    config_a.cloud_providers = vec![CloudProviderCreds {
        id: "openhuman".into(),
        slug: "openhuman".into(),
        label: "Managed catalog fixture".into(),
        endpoint: endpoint.clone(),
        auth_style: AuthStyle::OpenhumanJwt,
        default_model: None,
    }];
    let mut config_b = config_a.clone();
    config_b.api_key = Some("fixture-managed-credential-b".into());
    let extensions = crate::core::slots::Extensions::new();
    let prepared_fetches =
        register_test_inspection_catalog(&extensions, "openhuman", format!("{endpoint}/models"));
    let context = profile_context_with_extensions(config_b.clone(), "tenant-b", extensions);

    let (first, second, third) = crate::core::runtime::CoreContext::scope(context, async {
        let first = doctor_models(&config_a, true).await?;
        let second = doctor_models(&config_b, true).await?;
        let third = doctor_models(&config_b, true).await?;
        Ok::<_, String>((first, second, third))
    })
    .await
    .expect("selected profile scope");

    assert_eq!(first.value.entries[0].outcome, ModelProbeOutcome::Ok);
    assert_eq!(second.value.entries[0].outcome, ModelProbeOutcome::Ok);
    assert_eq!(third.value.entries[0].outcome, ModelProbeOutcome::Ok);
    for result in [&first, &second, &third] {
        assert!(result.value.entries.iter().all(|entry| {
            entry
                .message
                .as_deref()
                .is_none_or(|message| !message.contains("fixture-managed-credential-"))
        }));
    }
    assert_eq!(server.requests.load(Ordering::SeqCst), 2);
    assert_eq!(prepared_fetches.load(Ordering::SeqCst), 2);
    assert_eq!(
        *server
            .authorizations
            .lock()
            .unwrap_or_else(|error| error.into_inner()),
        vec![
            Some("Bearer fixture-managed-credential-a".to_owned()),
            Some("Bearer fixture-managed-credential-b".to_owned())
        ]
    );
}

#[tokio::test]
async fn a_slow_provider_times_out_without_blocking_the_other_catalog() {
    let per_provider_budget = Duration::from_secs(10);
    let (slow_endpoint, slow_server) = spawn_catalog_server_with_delay(
        StatusCode::OK,
        catalog_body(&["slow-model"]),
        true,
        per_provider_budget + Duration::from_secs(2),
    )
    .await;
    let (fast_endpoint, fast_server) =
        spawn_catalog_server(StatusCode::OK, catalog_body(&["fast-model"]), true).await;
    let (_temp, mut config) = temporary_config();
    config.cloud_providers = vec![
        CloudProviderCreds {
            id: "p_doctor_497_slow".into(),
            slug: "doctor-497-slow".into(),
            label: "Slow catalog fixture".into(),
            endpoint: slow_endpoint,
            auth_style: AuthStyle::None,
            default_model: None,
        },
        CloudProviderCreds {
            id: "p_doctor_497_fast".into(),
            slug: "doctor-497-fast".into(),
            label: "Fast catalog fixture".into(),
            endpoint: fast_endpoint,
            auth_style: AuthStyle::None,
            default_model: None,
        },
    ];

    let report = tokio::time::timeout(
        per_provider_budget + Duration::from_secs(2),
        doctor_models(&config, false),
    )
    .await
    .expect("the per-provider timeout must bound the doctor request")
    .expect("a timed-out provider should be represented in the report");

    let slow = report
        .value
        .entries
        .iter()
        .find(|entry| entry.provider == "doctor-497-slow")
        .expect("slow provider result");
    let fast = report
        .value
        .entries
        .iter()
        .find(|entry| entry.provider == "doctor-497-fast")
        .expect("fast provider result");
    assert_eq!(slow.outcome, ModelProbeOutcome::Error);
    assert_eq!(fast.outcome, ModelProbeOutcome::Ok);
    assert_eq!(slow_server.requests.load(Ordering::SeqCst), 1);
    assert_eq!(fast_server.requests.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn doctor_models_uses_read_only_http_without_calling_the_mutable_catalog() {
    use crate::core::runtime::{CoreContext, DomainSet};
    use crate::core::slots::Extensions;
    use crate::inference::registry::register_catalog_in;

    let (endpoint, server) =
        spawn_catalog_server(StatusCode::OK, catalog_body(&["must-not-be-used"]), true).await;
    let (_temp, config) = config_for_catalog(
        endpoint,
        "p_doctor_497_no_safe_hook",
        "doctor-497-no-safe-hook",
    );
    let normal_calls = Arc::new(AtomicUsize::new(0));
    let call_counter = Arc::clone(&normal_calls);
    let ordinary_listing: CatalogListing<crate::inference::registry::ProviderContext> =
        Arc::new(move |_| {
            let call_counter = Arc::clone(&call_counter);
            Box::pin(async move {
                call_counter.fetch_add(1, Ordering::SeqCst);
                Ok(serde_json::json!({ "models": [{ "id": "must-not-be-used" }] }))
            })
        });
    let extensions = Extensions::new();
    register_catalog_in(
        &extensions,
        "doctor-497-no-safe-hook",
        CatalogEntry {
            list: ordinary_listing,
            limits: None,
        },
    );
    let context = CoreContext::for_test_with_extensions(DomainSet::full(), None, extensions);

    let result = CoreContext::scope(context, async { doctor_models(&config, false).await }).await;

    assert_eq!(normal_calls.load(Ordering::SeqCst), 0);
    match result {
        Ok(report) => assert_eq!(report.value.entries[0].outcome, ModelProbeOutcome::Ok),
        Err(error) => panic!("read-only HTTP catalog should be available: {error}"),
    }
    assert_eq!(server.requests.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn inference_connect_checks_are_registered_but_skipped_in_static_tier() {
    let (endpoint, server) =
        spawn_catalog_server(StatusCode::OK, catalog_body(&["present-model"]), true).await;
    let (_temp, config) = config_for_catalog(endpoint, "p_doctor_497_static", "doctor-497-static");
    let provider = "doctor-497-static";
    crate::inference::auth_error_registry::clear(provider);
    crate::inference::auth_error_registry::record(provider, 401);

    let report = report_for_profile(
        &config,
        "doctor-497-static-profile",
        DoctorOptions {
            tier: super::DoctorTier::Static,
            only: vec![
                "inference/model-in-catalog".into(),
                "inference/auth-errors".into(),
            ],
            ..DoctorOptions::default()
        },
    )
    .await;
    crate::inference::auth_error_registry::clear(provider);

    for check_id in ["inference/model-in-catalog", "inference/auth-errors"] {
        assert!(report.value.checks_skipped.iter().any(|id| id == check_id));
        assert!(report.value.findings.iter().any(|finding| {
            finding.check_id == check_id && finding.severity == FindingSeverity::Skipped
        }));
    }
    assert!(report.value.ok);
    assert_eq!(server.requests.load(Ordering::SeqCst), 0);
}
