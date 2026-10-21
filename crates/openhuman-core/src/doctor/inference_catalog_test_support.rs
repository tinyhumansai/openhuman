use super::{ops::doctor_report_with_options, DoctorOptions};
use crate::config::schema::cloud_providers::{AuthStyle, CloudProviderCreds};
use crate::config::Config;
use crate::security::credentials::{AuthService, DEFAULT_AUTH_PROFILE_NAME};
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
pub(super) struct CatalogServerState {
    pub(super) status: StatusCode,
    pub(super) body: serde_json::Value,
    pub(super) delay: Duration,
    pub(super) requests: Arc<AtomicUsize>,
    pub(super) authorizations: Arc<std::sync::Mutex<Vec<Option<String>>>>,
}

pub(super) async fn catalog_handler(
    State(state): State<CatalogServerState>,
    headers: HeaderMap,
) -> Response {
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

pub(super) async fn missing_catalog_handler(State(state): State<CatalogServerState>) -> Response {
    state.requests.fetch_add(1, Ordering::SeqCst);
    StatusCode::NOT_FOUND.into_response()
}

pub(super) async fn spawn_catalog_server(
    status: StatusCode,
    body: serde_json::Value,
    has_models_route: bool,
) -> (String, CatalogServerState) {
    spawn_catalog_server_with_delay(status, body, has_models_route, Duration::ZERO).await
}

pub(super) async fn spawn_catalog_server_with_delay(
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

pub(super) fn temporary_config() -> (tempfile::TempDir, Config) {
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

pub(super) fn config_for_catalog(
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

pub(super) fn catalog_body(ids: &[&str]) -> serde_json::Value {
    serde_json::json!({
        "data": ids.iter().map(|id| serde_json::json!({ "id": id })).collect::<Vec<_>>()
    })
}

pub(super) fn store_provider_key(config: &Config, slug: &str, token: &str) {
    AuthService::from_config(config)
        .store_provider_token(
            &crate::inference::provider::factory::auth_key_for_slug(slug),
            DEFAULT_AUTH_PROFILE_NAME,
            token,
            std::collections::HashMap::new(),
            true,
        )
        .expect("seed scoped provider credential");
}

pub(super) fn auth_profiles_path(config: &Config) -> std::path::PathBuf {
    config
        .config_path
        .parent()
        .expect("fixture config parent")
        .join("auth-profiles.json")
}

pub(super) fn profile_context(
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

pub(super) fn workspace_context(
    config: Config,
) -> std::sync::Arc<crate::core::runtime::CoreContext> {
    use crate::core::runtime::{ContextOverlay, CoreContext, DomainSet};

    CoreContext::for_test(DomainSet::full(), None).derive_with(ContextOverlay::new(
        config,
        DomainSet::full(),
        crate::tools::toolpacks::ToolGroups::default(),
    ))
}

pub(super) fn profile_context_with_extensions(
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

pub(super) fn register_test_inspection_catalog(
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

pub(super) async fn report_for_profile(
    config: &Config,
    profile_id: &str,
    options: DoctorOptions,
) -> crate::core::Outcome<super::DoctorReport> {
    report_for_context(config, profile_context(config.clone(), profile_id), options).await
}

pub(super) async fn report_for_context(
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
