use super::super::inspection_entry;
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use openhuman_embed::__host::config::Config;
use openhuman_embed::__host::credentials::session_state::{
    SESSION_EXPIRES_AT_META, SESSION_ISSUING_BACKEND_META,
};
use openhuman_embed::__host::credentials::{APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME};
use openhuman_embed::__host::inference_registry::{
    InspectionCatalogError, InspectionCatalogRequest, PreparedInspectionCatalog,
};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

fn with_backend_env_test_lock(future: impl std::future::Future<Output = ()>) {
    let _env_lock = crate::backend::url::backend_env_test_lock();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("create current-thread test runtime")
        .block_on(future);
}

#[derive(Clone)]
struct MockManagedCatalog {
    status: StatusCode,
    requests: Arc<AtomicUsize>,
    authorization: Arc<std::sync::Mutex<Vec<Option<String>>>>,
}

async fn catalog_handler(State(state): State<MockManagedCatalog>, headers: HeaderMap) -> Response {
    state.requests.fetch_add(1, Ordering::SeqCst);
    state
        .authorization
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .push(
            headers
                .get("authorization")
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned),
        );
    if state.status.is_success() {
        Json(serde_json::json!({
            "data": [{ "id": "managed-fixture-model", "owned_by": "openrouter" }]
        }))
        .into_response()
    } else {
        (
            state.status,
            Json(serde_json::json!({ "error": "managed fixture rejected the request" })),
        )
            .into_response()
    }
}

async fn spawn_managed_backend(status: StatusCode) -> (String, MockManagedCatalog) {
    let state = MockManagedCatalog {
        status,
        requests: Arc::new(AtomicUsize::new(0)),
        authorization: Arc::new(std::sync::Mutex::new(Vec::new())),
    };
    let app = Router::new()
        .route("/openai/v1/models", get(catalog_handler))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind local managed-backend fixture");
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve local managed-backend fixture");
    });
    (base, state)
}

struct BackendEnvSnapshot {
    backend_url: Option<std::ffi::OsString>,
    vite_backend_url: Option<std::ffi::OsString>,
}

impl BackendEnvSnapshot {
    fn set(base: &str) -> Self {
        let snapshot = Self {
            backend_url: std::env::var_os("BACKEND_URL"),
            vite_backend_url: std::env::var_os("VITE_BACKEND_URL"),
        };
        std::env::set_var("BACKEND_URL", base);
        std::env::remove_var("VITE_BACKEND_URL");
        snapshot
    }
}

impl Drop for BackendEnvSnapshot {
    fn drop(&mut self) {
        match self.backend_url.take() {
            Some(value) => std::env::set_var("BACKEND_URL", value),
            None => std::env::remove_var("BACKEND_URL"),
        }
        match self.vite_backend_url.take() {
            Some(value) => std::env::set_var("VITE_BACKEND_URL", value),
            None => std::env::remove_var("VITE_BACKEND_URL"),
        }
    }
}

fn temporary_config(base: &str) -> (tempfile::TempDir, Config) {
    let root = tempfile::tempdir().expect("temporary managed-catalog fixture");
    let mut config = Config {
        config_path: root.path().join("config.toml"),
        workspace_dir: root.path().join("workspace"),
        action_dir: root.path().join("workspace"),
        ..Config::default()
    };
    config.api_url = None;
    config.secrets.encrypt = false;
    config.resolved_endpoints.control_plane_url = Some(base.to_owned());
    (root, config)
}

fn store_session(
    config: &Config,
    token: &str,
    issuer: &str,
    expires_at: chrono::DateTime<chrono::Utc>,
) {
    let mut metadata = std::collections::HashMap::new();
    metadata.insert(SESSION_ISSUING_BACKEND_META.to_owned(), issuer.to_owned());
    metadata.insert(SESSION_EXPIRES_AT_META.to_owned(), expires_at.to_rfc3339());
    let profile_id = format!("{APP_SESSION_PROVIDER}:{DEFAULT_AUTH_PROFILE_NAME}");
    let now = chrono::Utc::now().to_rfc3339();
    let mut active_profiles = std::collections::HashMap::new();
    active_profiles.insert(APP_SESSION_PROVIDER, profile_id.clone());
    let mut profiles = std::collections::HashMap::new();
    profiles.insert(
        profile_id,
        serde_json::json!({
            "provider": APP_SESSION_PROVIDER,
            "profile_name": DEFAULT_AUTH_PROFILE_NAME,
            "kind": "token",
            "token": token,
            "metadata": metadata,
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
    .expect("seed read-only session profile fixture");
}

fn auth_profiles_path(config: &Config) -> std::path::PathBuf {
    config
        .config_path
        .parent()
        .expect("fixture config parent")
        .join("auth-profiles.json")
}

async fn prepare_managed_catalog(
    config: Config,
) -> Result<PreparedInspectionCatalog, InspectionCatalogError> {
    let inspect = inspection_entry();
    inspect(
        config,
        InspectionCatalogRequest {
            provider_key: String::new(),
            client: reqwest::Client::new(),
        },
    )
    .await
}

async fn inspect_managed_catalog(
    config: Config,
) -> Result<openhuman_embed::__host::core::Outcome<serde_json::Value>, InspectionCatalogError> {
    prepare_managed_catalog(config).await?.fetch().await
}

#[test]
fn managed_inspection_catalog_lists_models_with_a_read_only_session() {
    with_backend_env_test_lock(async {
        let _registry = crate::registry_test_lock_async().await;
        let (base, server) = spawn_managed_backend(StatusCode::OK).await;
        let _env = BackendEnvSnapshot::set(&base);
        let (_temp, config) = temporary_config(&base);
        store_session(
            &config,
            "fixture-managed-session",
            &base,
            chrono::Utc::now() + chrono::Duration::hours(1),
        );
        let auth_file_before = std::fs::read(auth_profiles_path(&config)).expect("session fixture");

        let result = inspect_managed_catalog(config.clone())
            .await
            .expect("managed inspection should list the catalog");

        assert_eq!(result.value["models"][0]["id"], "managed-fixture-model");
        assert_eq!(server.requests.load(Ordering::SeqCst), 1);
        assert_eq!(
            *server
                .authorization
                .lock()
                .unwrap_or_else(|error| error.into_inner()),
            vec![Some("Bearer fixture-managed-session".to_owned())]
        );
        assert_eq!(
            std::fs::read(auth_profiles_path(&config)).unwrap(),
            auth_file_before
        );
    });
}

#[test]
fn managed_inspection_catalog_classifies_a_401_without_mutating_credentials() {
    with_backend_env_test_lock(async {
        let _registry = crate::registry_test_lock_async().await;
        let (base, server) = spawn_managed_backend(StatusCode::UNAUTHORIZED).await;
        let _env = BackendEnvSnapshot::set(&base);
        let (_temp, config) = temporary_config(&base);
        store_session(
            &config,
            "fixture-managed-session",
            &base,
            chrono::Utc::now() + chrono::Duration::hours(1),
        );
        let auth_file_before = std::fs::read(auth_profiles_path(&config)).expect("session fixture");

        let error = inspect_managed_catalog(config.clone())
            .await
            .expect_err("a managed-catalog 401 is an auth/access result");

        assert!(matches!(error, InspectionCatalogError::AuthOrAccess));
        assert_eq!(server.requests.load(Ordering::SeqCst), 1);
        assert_eq!(
            std::fs::read(auth_profiles_path(&config)).unwrap(),
            auth_file_before
        );
    });
}

#[test]
fn a_prepared_managed_fetch_uses_the_frozen_session_after_rotation() {
    with_backend_env_test_lock(async {
        let _registry = crate::registry_test_lock_async().await;
        let (base, server) = spawn_managed_backend(StatusCode::OK).await;
        let _env = BackendEnvSnapshot::set(&base);
        let (_temp, config) = temporary_config(&base);
        store_session(
            &config,
            "fixture-session-before-rotation",
            &base,
            chrono::Utc::now() + chrono::Duration::hours(1),
        );
        let auth_file_a_before =
            std::fs::read(auth_profiles_path(&config)).expect("session A fixture");

        let prepared_a = prepare_managed_catalog(config.clone())
            .await
            .expect("read-only prepare should capture session A");
        assert_eq!(
            std::fs::read(auth_profiles_path(&config)).unwrap(),
            auth_file_a_before,
            "preparing A must not rewrite auth state"
        );
        store_session(
            &config,
            "fixture-session-after-rotation",
            &base,
            chrono::Utc::now() + chrono::Duration::hours(1),
        );
        let auth_file_after_rotation =
            std::fs::read(auth_profiles_path(&config)).expect("rotated session fixture");

        prepared_a
            .fetch()
            .await
            .expect("the prepared request must use its captured session");
        assert_eq!(
            std::fs::read(auth_profiles_path(&config)).unwrap(),
            auth_file_after_rotation,
            "fetching frozen A must not change the newly stored B credential"
        );
        let prepared_b = prepare_managed_catalog(config.clone())
            .await
            .expect("a new prepare should see session B");
        assert_eq!(
            std::fs::read(auth_profiles_path(&config)).unwrap(),
            auth_file_after_rotation,
            "preparing B must not rewrite auth state"
        );
        prepared_b
            .fetch()
            .await
            .expect("the second prepared request should use session B");
        assert_eq!(
            std::fs::read(auth_profiles_path(&config)).unwrap(),
            auth_file_after_rotation,
            "fetching frozen B must not rewrite auth state"
        );

        assert_eq!(server.requests.load(Ordering::SeqCst), 2);
        assert_eq!(
            *server
                .authorization
                .lock()
                .unwrap_or_else(|error| error.into_inner()),
            vec![
                Some("Bearer fixture-session-before-rotation".to_owned()),
                Some("Bearer fixture-session-after-rotation".to_owned())
            ]
        );
        assert_eq!(
            std::fs::read(auth_profiles_path(&config)).unwrap(),
            auth_file_after_rotation
        );
    });
}

#[test]
fn a_session_from_another_backend_is_never_sent_to_the_managed_catalog() {
    with_backend_env_test_lock(async {
        let _registry = crate::registry_test_lock_async().await;
        let (base, server) = spawn_managed_backend(StatusCode::OK).await;
        let _env = BackendEnvSnapshot::set(&base);
        let (_temp, config) = temporary_config(&base);
        store_session(
            &config,
            "fixture-session-issued-elsewhere",
            "http://issuer-a.example",
            chrono::Utc::now() + chrono::Duration::hours(1),
        );
        let auth_file_before = std::fs::read(auth_profiles_path(&config)).expect("session fixture");

        let result = inspect_managed_catalog(config.clone()).await;

        assert!(
            result.is_err(),
            "a foreign session must not pass inspection"
        );
        assert_eq!(server.requests.load(Ordering::SeqCst), 0);
        assert_eq!(
            std::fs::read(auth_profiles_path(&config)).unwrap(),
            auth_file_before
        );
    });
}

#[test]
fn an_expired_session_is_skipped_without_broadcast_or_store_mutation() {
    use openhuman_embed::__host::core::events::DomainEvent;

    with_backend_env_test_lock(async {
        let _registry = crate::registry_test_lock_async().await;
        openhuman_embed::__host::core::bus::init()
            .await
            .expect("event bus init");
        let mut events = openhuman_embed::__host::core::bus::BUS
            .get()
            .expect("event bus initialized")
            .receiver();
        while events.try_recv().is_ok() {}

        let (base, server) = spawn_managed_backend(StatusCode::OK).await;
        let _env = BackendEnvSnapshot::set(&base);
        let (_temp, config) = temporary_config(&base);
        store_session(
            &config,
            "fixture-expired-session",
            &base,
            chrono::Utc::now() - chrono::Duration::hours(1),
        );
        let auth_file_before = std::fs::read(auth_profiles_path(&config)).expect("session fixture");

        let result = inspect_managed_catalog(config.clone()).await;

        assert!(matches!(result, Err(InspectionCatalogError::Unavailable)));
        assert_eq!(server.requests.load(Ordering::SeqCst), 0);
        assert_eq!(
            std::fs::read(auth_profiles_path(&config)).unwrap(),
            auth_file_before
        );
        let published_expiry = std::iter::from_fn(|| events.try_recv().ok()).any(|event| {
            matches!(
                event,
                DomainEvent::SessionExpired { source, .. }
                    if source == "credentials.local_expiry_precheck"
            )
        });
        assert!(
            !published_expiry,
            "inspection must not broadcast local expiry"
        );
    });
}

#[test]
fn managed_inspection_catalog_maps_each_failure_status_to_its_variant() {
    with_backend_env_test_lock(async {
        let _registry = crate::registry_test_lock_async().await;
        for (status, expected) in [
            (StatusCode::FORBIDDEN, "AuthOrAccess"),
            (StatusCode::NOT_FOUND, "NoCatalog"),
            (StatusCode::INTERNAL_SERVER_ERROR, "Failure"),
            (StatusCode::BAD_GATEWAY, "Failure"),
        ] {
            let (base, server) = spawn_managed_backend(status).await;
            let _env = BackendEnvSnapshot::set(&base);
            let (_temp, config) = temporary_config(&base);
            store_session(
                &config,
                "fixture-managed-session",
                &base,
                chrono::Utc::now() + chrono::Duration::hours(1),
            );
            let auth_file_before =
                std::fs::read(auth_profiles_path(&config)).expect("session fixture");

            let error = inspect_managed_catalog(config.clone())
                .await
                .expect_err("a non-2xx catalog answer is an error");

            let variant = match error {
                InspectionCatalogError::AuthOrAccess => "AuthOrAccess",
                InspectionCatalogError::NoCatalog => "NoCatalog",
                InspectionCatalogError::Failure => "Failure",
                _ => "other",
            };
            assert_eq!(variant, expected, "status {status}");
            assert_eq!(server.requests.load(Ordering::SeqCst), 1, "status {status}");
            assert_eq!(
                std::fs::read(auth_profiles_path(&config)).unwrap(),
                auth_file_before,
                "status {status}"
            );
        }
    });
}
