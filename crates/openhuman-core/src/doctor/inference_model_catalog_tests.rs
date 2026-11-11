use super::{
    ops::{doctor_models, doctor_report_with_options},
    DoctorOptions, FindingSeverity, ModelProbeOutcome,
};
use crate::config::schema::cloud_providers::{AuthStyle, CloudProviderCreds};
use crate::config::schema::ModelRouteConfig;
use crate::config::Config;
use crate::credentials::{AuthService, DEFAULT_AUTH_PROFILE_NAME};
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

async fn store_provider_key(config: &Config, slug: &str, token: &str) {
    AuthService::from_config(config)
        .store_provider_token(
            &crate::inference::provider::factory::auth_key_for_slug(slug),
            DEFAULT_AUTH_PROFILE_NAME,
            token,
            std::collections::HashMap::new(),
            true,
        )
        .await
        .expect("seed scoped provider credential");
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

#[tokio::test]
async fn doctor_models_skips_a_provider_without_a_models_route() {
    let (endpoint, server) = spawn_catalog_server(StatusCode::OK, catalog_body(&[]), false).await;
    let (_temp, config) =
        config_for_catalog(endpoint, "p_doctor_497_no_models", "doctor-497-no-models");

    let endpoint_status = reqwest::get(format!("{}/models", config.cloud_providers[0].endpoint))
        .await
        .expect("mock provider should answer the missing catalog route");
    assert_eq!(endpoint_status.status(), StatusCode::NOT_FOUND);
    server.requests.store(0, Ordering::SeqCst);

    let report = doctor_models(&config, false)
        .await
        .expect("a provider without a model-listing route is an ordinary skipped check");

    assert_eq!(report.value.summary.skipped, 1);
    assert_eq!(report.value.summary.errors, 0);
    assert_eq!(report.value.entries[0].outcome, ModelProbeOutcome::Skipped);
    assert!(report.value.entries[0]
        .message
        .as_deref()
        .is_some_and(|message| message.to_ascii_lowercase().contains("models")));
    assert_eq!(server.requests.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn model_in_catalog_check_skips_a_provider_without_a_models_route() {
    let (endpoint, server) = spawn_catalog_server(StatusCode::OK, catalog_body(&[]), false).await;
    let (_temp, config) = config_for_catalog(
        endpoint.clone(),
        "p_doctor_497_route_missing",
        "doctor-497-route-missing",
    );
    let extensions = crate::core::slots::Extensions::new();
    register_test_inspection_catalog(
        &extensions,
        "doctor-497-route-missing",
        format!("{endpoint}/models"),
    );
    let context =
        profile_context_with_extensions(config.clone(), "doctor-497-route-profile", extensions);

    let report = crate::core::runtime::CoreContext::scope(context, async {
        doctor_report_with_options(
            &config,
            DoctorOptions {
                only: vec!["inference/model-in-catalog".into()],
                ..DoctorOptions::default()
            },
        )
        .await
    })
    .await
    .expect("scoped doctor report");

    assert!(report
        .value
        .checks_run
        .iter()
        .any(|id| id == "inference/model-in-catalog"));
    assert!(report.value.findings.iter().any(|finding| {
        finding.check_id == "inference/model-in-catalog"
            && finding.severity == FindingSeverity::Skipped
            && (finding.message.to_ascii_lowercase().contains("models")
                || finding.message.to_ascii_lowercase().contains("catalog"))
    }));
    assert_eq!(report.value.summary.error, 0);
    assert_eq!(server.requests.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn doctor_models_classifies_provider_401_as_auth_or_access() {
    let (endpoint, server) = spawn_catalog_server(
        StatusCode::UNAUTHORIZED,
        serde_json::json!({ "error": "credentials rejected" }),
        true,
    )
    .await;
    let (_temp, config) = config_for_catalog(
        endpoint,
        "p_doctor_497_unauthorized",
        "doctor-497-unauthorized",
    );

    let report = doctor_models(&config, false)
        .await
        .expect("provider authorization failures should be represented in the report");

    assert_eq!(report.value.summary.auth_or_access, 1);
    assert_eq!(report.value.summary.errors, 0);
    assert_eq!(
        report.value.entries[0].outcome,
        ModelProbeOutcome::AuthOrAccess
    );
    assert_eq!(server.requests.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn missing_default_and_route_models_are_warned_by_the_inference_check() {
    let (endpoint, server) =
        spawn_catalog_server(StatusCode::OK, catalog_body(&["present-model"]), true).await;
    let (_temp, mut config) =
        config_for_catalog(endpoint, "p_doctor_497_missing", "doctor-497-missing");
    config.default_model = Some("doctor-497-missing:missing-default".into());
    config.model_routes.push(ModelRouteConfig {
        hint: "coding".into(),
        model: "doctor-497-missing:missing-route-model".into(),
    });
    let extensions = crate::core::slots::Extensions::new();
    let catalog_calls = register_test_inspection_catalog(
        &extensions,
        "doctor-497-missing",
        format!("{}/models", config.cloud_providers[0].endpoint),
    );
    let context = profile_context_with_extensions(config.clone(), "doctor-profile-b", extensions);
    let (catalog, report) = crate::core::runtime::CoreContext::scope(context, async {
        let catalog = doctor_models(&config, true).await?;
        let report = doctor_report_with_options(
            &config,
            DoctorOptions {
                only: vec!["inference/model-in-catalog".into()],
                ..DoctorOptions::default()
            },
        )
        .await?;
        Ok::<_, String>((catalog, report))
    })
    .await
    .expect("selected profile scope");

    assert_eq!(catalog.value.entries[0].outcome, ModelProbeOutcome::Ok);
    assert_eq!(report.value.profile.as_deref(), Some("doctor-profile-b"));
    assert!(report
        .value
        .checks_run
        .iter()
        .any(|id| id == "inference/model-in-catalog"));
    let missing_default = report.value.findings.iter().find(|finding| {
        finding.check_id == "inference/model-in-catalog"
            && finding.severity == FindingSeverity::Warn
            && finding.config_path.as_deref() == Some("default_model")
            && finding.message.contains("default_model")
    });
    assert!(missing_default.is_some(), "{:#?}", report.value.findings);
    let missing_route = report.value.findings.iter().find(|finding| {
        finding.check_id == "inference/model-in-catalog"
            && finding.severity == FindingSeverity::Warn
            && finding.config_path.as_deref() == Some("model_routes[0].model")
    });
    assert!(missing_route.is_some(), "{:#?}", report.value.findings);
    assert!(report.value.findings.iter().all(|finding| {
        !finding.message.contains("missing-default")
            && !finding.message.contains("missing-route-model")
    }));
    assert_eq!(catalog_calls.load(Ordering::SeqCst), 1);
    assert_eq!(server.requests.load(Ordering::SeqCst), 1);
}
