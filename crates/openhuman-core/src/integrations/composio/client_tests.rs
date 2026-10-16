use super::*;
use crate::config::Config;
use crate::integrations::composio::module_client::module_guard;

use axum::{extract::State, http::StatusCode, routing::get, Json, Router};
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};

async fn start_mock_backend(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://127.0.0.1:{}", addr.port())
}

// ── Route resolution tests (`resolve_composio_route`) ─────────────────
//
// Mirror the branches the spec demands:
//   1. backend mode with a session JWT — Backend variant
//   2. direct mode + stored api key — Direct variant
//   3. direct mode without api key — explicit error
//   4. unknown mode string — explicit error

fn config_with_session_token(tmp: &tempfile::TempDir) -> Config {
    let mut config = Config::default();
    config.config_path = tmp.path().join("config.toml");
    crate::security::credentials::AuthService::from_config(&config)
        .store_provider_token(
            crate::security::credentials::APP_SESSION_PROVIDER,
            crate::security::credentials::DEFAULT_AUTH_PROFILE_NAME,
            "test-token",
            std::collections::HashMap::new(),
            true,
        )
        .expect("store test session token");
    config
}

/// Direct-mode reads are exercised over HTTP through the connector module:
/// `DirectCredential::new_with_v3_base` points the module's `/tools` and
/// `/connected_accounts` GETs at a local axum mock, so we can assert the
/// outbound `tags` filter (repeated query params) and the v3 ->
/// canonical-envelope reshape without touching `backend.composio.dev`. These
/// tests reach the process-global module, so they hold `module_guard`.
fn direct_tool_for_mock(base_v3: String) -> std::sync::Arc<DirectCredential> {
    direct_tool_for_mock_with_key(base_v3, "ck_test_direct")
}

fn direct_tool_for_mock_with_key(
    base_v3: String,
    api_key: &str,
) -> std::sync::Arc<DirectCredential> {
    std::sync::Arc::new(DirectCredential::new_with_v3_base(api_key, base_v3))
}

/// A config that can load the connector module and names no route.
fn module_test_config(tmp: &tempfile::TempDir) -> Config {
    let mut config = Config::default();
    config.config_path = tmp.path().join("config.toml");
    config.workspace_dir = tmp.path().join("workspace");
    config
}

struct DirectAuthFailureGuard {
    key_id: u64,
}

impl DirectAuthFailureGuard {
    fn for_tool(tool: &std::sync::Arc<DirectCredential>) -> Self {
        let key_id = tool.auth_key_fingerprint();
        crate::integrations::composio::direct_auth::reset_direct_auth_failure(key_id);
        Self { key_id }
    }
}

impl Drop for DirectAuthFailureGuard {
    fn drop(&mut self) {
        crate::integrations::composio::direct_auth::reset_direct_auth_failure(self.key_id);
    }
}

/// A host-registered backend route (the core ships none of its own).
fn register_fake_backend_route() {
    crate::integrations::composio::test_backend_route::register();
}

#[test]
fn resolve_composio_route_backend_is_unavailable_without_a_registered_route() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config = config_with_session_token(&tmp);
    let err = resolve_composio_route(&config)
        .err()
        .expect("a core with no backend route builder has no backend route");
    assert!(
        err.to_string()
            .starts_with(crate::core::observability::BACKEND_UNAVAILABLE_PREFIX),
        "got: {err}"
    );
}

#[test]
fn resolve_composio_route_backend_variant_when_mode_default() {
    register_fake_backend_route();
    let tmp = tempfile::tempdir().expect("tempdir");
    let config = config_with_session_token(&tmp);
    let route = resolve_composio_route(&config).expect("backend mode should build");
    assert_eq!(route.mode(), "backend");
    assert!(matches!(route, ComposioRoute::Backend));
}

#[test]
fn resolve_composio_route_backend_empty_mode_falls_back_to_backend() {
    // A literal empty string in TOML should be treated as the default
    // (`"backend"`) rather than an unknown mode error.
    register_fake_backend_route();
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut config = config_with_session_token(&tmp);
    config.composio.mode = String::new();
    let route = resolve_composio_route(&config).expect("empty mode should fall back to backend");
    assert_eq!(route.mode(), "backend");
}

#[test]
fn resolve_composio_route_disabled_errors_even_with_a_session() {
    // `disabled` wins over a signed-in session: no route, so no tools register.
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut config = config_with_session_token(&tmp);
    config.composio.mode = "disabled".into();
    let err = resolve_composio_route(&config)
        .err()
        .expect("disabled must not resolve a route");
    assert!(err.to_string().contains("disabled"), "got: {err}");
}

#[tokio::test]
async fn disabled_mode_reports_no_integrations_without_a_backend_call() {
    use crate::integrations::composio::{
        fetch_connected_integrations_status, FetchConnectedIntegrationsStatus,
    };
    // Default config would try the hosted backend; disabled must answer locally.
    let mut config = Config::default();
    config.composio.mode = "disabled".into();
    match fetch_connected_integrations_status(&config).await {
        FetchConnectedIntegrationsStatus::Authoritative(v) => assert!(v.is_empty()),
        FetchConnectedIntegrationsStatus::Unavailable => panic!("expected authoritative empty"),
    }
}

#[test]
fn resolve_composio_route_backend_errors_without_session() {
    // Backend mode requires the app-session JWT — without it the
    // factory must return an explicit error (not silently downgrade).
    register_fake_backend_route();
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut config = crate::config::Config::default();
    config.config_path = tmp.path().join("config.toml");
    let err = resolve_composio_route(&config)
        .err()
        .expect("must error without auth token");
    assert!(
        err.to_string().contains("no backend session token"),
        "unexpected error: {err}"
    );
}

#[test]
fn resolve_composio_route_direct_variant_with_stored_key() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut config = crate::config::Config::default();
    config.config_path = tmp.path().join("config.toml");
    config.composio.mode = "direct".into();
    // Persist the key the way the RPC layer would.
    crate::security::credentials::AuthService::from_config(&config)
        .store_provider_token(
            crate::security::credentials::COMPOSIO_DIRECT_PROVIDER,
            crate::security::credentials::DEFAULT_AUTH_PROFILE_NAME,
            "ck_test_key_redacted",
            std::collections::HashMap::new(),
            true,
        )
        .expect("store direct api key");
    let route = resolve_composio_route(&config).expect("direct mode with stored key should build");
    assert_eq!(route.mode(), "direct");
    assert!(matches!(route, ComposioRoute::Direct(_)));
}

#[test]
fn resolve_composio_route_direct_falls_back_to_config_api_key() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut config = crate::config::Config::default();
    config.config_path = tmp.path().join("config.toml");
    config.composio.mode = "direct".into();
    // No keychain entry — fall back to the inline config field.
    config.composio.api_key = Some("ck_inline_redacted".into());
    let route = resolve_composio_route(&config)
        .expect("direct mode should accept inline config.api_key when keychain is empty");
    assert_eq!(route.mode(), "direct");
}

#[test]
fn resolve_composio_route_direct_errors_without_key() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut config = crate::config::Config::default();
    config.config_path = tmp.path().join("config.toml");
    config.composio.mode = "direct".into();
    let err = resolve_composio_route(&config)
        .err()
        .expect("direct without key must error");
    let msg = err.to_string();
    assert!(
        msg.contains("no api key is configured"),
        "unexpected error: {msg}"
    );
}

#[test]
fn resolve_composio_route_unknown_mode_errors() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut config = crate::config::Config::default();
    config.config_path = tmp.path().join("config.toml");
    config.composio.mode = "voyage".into();
    let err = resolve_composio_route(&config)
        .err()
        .expect("unknown mode must error");
    let msg = err.to_string();
    assert!(msg.contains("unknown composio mode"), "got: {msg}");
    assert!(
        msg.contains("voyage"),
        "should echo the invalid value, got: {msg}"
    );
}

// ── Direct-mode credentials helpers ─────────────────────────────────

#[test]
fn store_get_clear_composio_api_key_roundtrip() {
    use crate::security::credentials::{get_composio_api_key, COMPOSIO_DIRECT_PROVIDER};

    let tmp = tempfile::tempdir().expect("tempdir");
    let mut config = crate::config::Config::default();
    config.config_path = tmp.path().join("config.toml");

    // Initially: nothing stored.
    assert_eq!(
        get_composio_api_key(&config).expect("read empty store"),
        None
    );

    // Store under the direct-mode provider slot.
    crate::security::credentials::AuthService::from_config(&config)
        .store_provider_token(
            COMPOSIO_DIRECT_PROVIDER,
            crate::security::credentials::DEFAULT_AUTH_PROFILE_NAME,
            "ck_secret_value_redacted",
            std::collections::HashMap::new(),
            true,
        )
        .expect("store");

    assert_eq!(
        get_composio_api_key(&config).expect("read stored"),
        Some("ck_secret_value_redacted".into())
    );

    // Clearing the profile must remove it again.
    crate::security::credentials::AuthService::from_config(&config)
        .remove_profile(
            COMPOSIO_DIRECT_PROVIDER,
            crate::security::credentials::DEFAULT_AUTH_PROFILE_NAME,
        )
        .expect("remove");
    assert_eq!(
        get_composio_api_key(&config).expect("read post-clear"),
        None
    );
}

#[tokio::test]
async fn direct_list_connections_stops_hitting_composio_after_repeated_invalid_api_key() {
    let _module = module_guard().await;
    let tmp = tempfile::tempdir().unwrap();
    let config = module_test_config(&tmp);
    let hits = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route(
            "/connected_accounts",
            get(|State(hits): State<Arc<AtomicUsize>>| async move {
                hits.fetch_add(1, Ordering::SeqCst);
                (
                    StatusCode::UNAUTHORIZED,
                    Json(json!({ "error": { "message": "Invalid API key" } })),
                )
            }),
        )
        .with_state(hits.clone());
    let base = start_mock_backend(app).await;
    let tool = direct_tool_for_mock_with_key(base, "ck_test_direct_invalid_backoff");
    let _auth_guard = DirectAuthFailureGuard::for_tool(&tool);

    for _ in 0..2 {
        let err = direct_list_connections(&config, &tool)
            .await
            .expect_err("invalid key should reject");
        assert!(
            err.to_string().contains("Invalid API key"),
            "unexpected error: {err:#}"
        );
    }

    let opened = direct_list_connections(&config, &tool)
        .await
        .expect_err("third invalid-key failure should open the backoff gate");
    assert!(
        opened.to_string().contains("re-enter"),
        "backoff error should be actionable, got: {opened:#}"
    );

    let short_circuit = direct_list_connections(&config, &tool)
        .await
        .expect_err("open backoff gate should short-circuit before HTTP");
    assert!(
        short_circuit.to_string().contains("re-enter"),
        "short-circuit error should stay actionable, got: {short_circuit:#}"
    );
    assert_eq!(
        hits.load(Ordering::SeqCst),
        3,
        "after three invalid-key failures, later polls must not hit Composio"
    );
}

#[tokio::test]
async fn direct_list_tools_forwards_tags_and_reshapes_v3_envelope() {
    let _module = module_guard().await;
    let tmp = tempfile::tempdir().unwrap();
    let config = module_test_config(&tmp);
    use axum::extract::RawQuery;
    use std::sync::Mutex;

    let captured: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let sink = captured.clone();
    let app = Router::new().route(
        "/tools",
        get(move |RawQuery(q): RawQuery| {
            let sink = sink.clone();
            async move {
                *sink.lock().unwrap() = q;
                Json(json!({
                    "items": [
                        {
                            "slug": "GITHUB_STAR_A_REPOSITORY",
                            "description": "Star a repository",
                            "input_parameters": { "type": "object" },
                            "toolkit": { "slug": "github" }
                        },
                        // Empty-slug rows must be dropped by the reshaper.
                        { "slug": "", "description": "junk" }
                    ]
                }))
            }
        }),
    );
    let base = start_mock_backend(app).await;
    let tool = direct_tool_for_mock(base);

    let resp = super::direct_list_tools(
        &config,
        &tool,
        &["github".to_string()],
        Some(&["stars".to_string(), "repos".to_string()]),
    )
    .await
    .expect("direct_list_tools should succeed against the mock");

    // Outbound: tags forwarded as repeated params, toolkits CSV.
    let query = captured.lock().unwrap().clone().expect("server saw query");
    assert!(query.contains("tags=stars"), "query was: {query}");
    assert!(query.contains("tags=repos"), "query was: {query}");
    assert!(query.contains("toolkits=github"), "query was: {query}");

    // Inbound: reshaped into the backend envelope, empty-slug row dropped.
    assert_eq!(resp.tools.len(), 1);
    assert_eq!(resp.tools[0].function.name, "GITHUB_STAR_A_REPOSITORY");
    assert_eq!(resp.tools[0].kind, "function");
}

#[tokio::test]
async fn direct_connected_integrations_fetches_schemas_without_backend_composio_route() {
    let _module = module_guard().await;
    let tmp = tempfile::tempdir().unwrap();
    let backend_tool_requests = Arc::new(AtomicUsize::new(0));
    let backend_hits = backend_tool_requests.clone();
    let backend = Router::new().route(
        "/agent-integrations/composio/tools",
        get(move || {
            let hits = backend_hits.clone();
            async move {
                hits.fetch_add(1, Ordering::SeqCst);
                (
                    StatusCode::UNAUTHORIZED,
                    Json(json!({"error":"SESSION_EXPIRED"})),
                )
            }
        }),
    );
    let backend_url = start_mock_backend(backend).await;

    let direct_api = Router::new()
        .route(
            "/connected_accounts",
            get(|| async {
                Json(json!({"items":[{
                    "id":"connection-1",
                    "status":"ACTIVE",
                    "toolkit":{"slug":"gmail"}
                }]}))
            }),
        )
        .route(
            "/tools",
            get(|| async {
                Json(json!({"items":[{
                    "slug":"GMAIL_SEND_EMAIL",
                    "description":"Send email",
                    "input_parameters":{"type":"object"},
                    "toolkit":{"slug":"gmail"}
                }]}))
            }),
        );
    let direct_url = start_mock_backend(direct_api).await;
    let mut config = config_with_session_token(&tmp);
    config.workspace_dir = tmp.path().join("workspace");
    config.api_url = Some(backend_url);
    crate::security::credentials::AuthService::from_config(&config)
        .store_provider_token(
            crate::security::credentials::APP_SESSION_PROVIDER,
            crate::security::credentials::DEFAULT_AUTH_PROFILE_NAME,
            "desktop.test.local",
            std::collections::HashMap::new(),
            true,
        )
        .expect("store offline local session token");
    crate::security::credentials::api_key::store_api_key(&config, "th_test_backend_key")
        .expect("store backend API key");
    config.composio.mode = "direct".into();
    config.composio.pin_host_credential(
        crate::config::ComposioHostCredential::direct("ck_test_spawn_direct")
            .base_urls(direct_url.clone(), direct_url),
    );

    // The operator scope's cache epoch is process-wide: any other test that
    // invalidates it while this (slow, module-backed) fetch is in flight fences
    // the result to `Unavailable`. Retry a fenced fetch; the assertion that the
    // backend route is never asked still covers every attempt.
    let mut integrations = Vec::new();
    for attempt in 1..=5 {
        match crate::integrations::composio::fetch_connected_integrations_status(&config).await {
            crate::integrations::composio::FetchConnectedIntegrationsStatus::Authoritative(
                found,
            ) => {
                integrations = found;
                break;
            }
            crate::integrations::composio::FetchConnectedIntegrationsStatus::Unavailable => {
                tracing::debug!(
                    attempt,
                    "[composio][test] fetch fenced or unavailable; retrying"
                );
            }
        }
    }

    let gmail = integrations
        .iter()
        .find(|integration| integration.toolkit == "gmail")
        .unwrap_or_else(|| panic!("direct connection should be represented: {integrations:?}"));
    assert_eq!(gmail.tools.len(), 1);
    assert_eq!(gmail.tools[0].name, "GMAIL_SEND_EMAIL");
    assert_eq!(backend_tool_requests.load(Ordering::SeqCst), 0);
}

// ── failure messages stay byte-identical to the pre-module client ─────────

#[tokio::test]
async fn http_failures_keep_their_user_facing_messages() {
    let _module = module_guard().await;
    let tmp = tempfile::tempdir().unwrap();
    let config = module_test_config(&tmp);
    let app = Router::new()
        .route(
            "/connected_accounts",
            get(|| async {
                (
                    StatusCode::UNAUTHORIZED,
                    Json(json!({"error": {"message": "Invalid API key"}})),
                )
            }),
        )
        .route(
            "/tools",
            get(|| async { (StatusCode::INTERNAL_SERVER_ERROR, "") }),
        );
    let tool = direct_tool_for_mock_with_key(
        start_mock_backend(app).await,
        "ck_test_direct_message_fixture",
    );
    let _auth_guard = DirectAuthFailureGuard::for_tool(&tool);

    let err = direct_list_connections(&config, &tool).await.unwrap_err();
    assert_eq!(
        format!("{err:#}"),
        "Composio v3 connected_accounts failed: HTTP 401: Invalid API key"
    );
    let err = direct_list_tools(&config, &tool, &[], None)
        .await
        .unwrap_err();
    assert_eq!(
        format!("{err:#}"),
        "Composio v3 list_tool_schemas: HTTP 500"
    );
}

#[tokio::test]
async fn connections_come_back_without_route_lifted_identity() {
    let _module = module_guard().await;
    let tmp = tempfile::tempdir().unwrap();
    let config = module_test_config(&tmp);
    let app = Router::new().route(
        "/connected_accounts",
        get(|| async {
            Json(json!({"items": [
                {"id": " ca_1 ", "toolkit": "gmail", "status": "ACTIVE", "email": "a@b.c"},
                {"id": "  ", "toolkit": "slack", "status": "ACTIVE"}
            ]}))
        }),
    );
    let tool = direct_tool_for_mock_with_key(start_mock_backend(app).await, "ck_test_identity");
    let _auth_guard = DirectAuthFailureGuard::for_tool(&tool);
    let connections = direct_list_connections(&config, &tool)
        .await
        .unwrap()
        .connections;
    assert_eq!(connections.len(), 1, "blank id dropped");
    assert_eq!(connections[0].id, "ca_1");
    // Identity is the host's to enrich from cached profiles.
    assert!(connections[0].account_email.is_none());
}

#[tokio::test]
async fn direct_reads_do_not_forward_credentials_across_redirects() {
    let _module = module_guard().await;
    let tmp = tempfile::tempdir().unwrap();
    let config = module_test_config(&tmp);
    let redirected_request_seen = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let observed = redirected_request_seen.clone();
    let destination = start_mock_backend(Router::new().route(
        "/tools",
        get(move || {
            let observed = observed.clone();
            async move {
                observed.store(true, Ordering::SeqCst);
                Json(json!({"items": []}))
            }
        }),
    ))
    .await;
    let redirect = format!("{destination}/tools");
    let source = start_mock_backend(Router::new().route(
        "/tools",
        get(move || {
            let redirect = redirect.clone();
            async move { axum::response::Redirect::temporary(&redirect) }
        }),
    ))
    .await;

    let tool = direct_tool_for_mock_with_key(source, "ck_secret_value");
    assert!(direct_list_tools(&config, &tool, &[], None).await.is_err());
    assert!(!redirected_request_seen.load(Ordering::SeqCst));
}

// ── the host's proxy policy reaches the module ────────────────────────────

/// A CONNECT proxy on loopback that tunnels to whatever it is asked for and
/// reports each CONNECT request line it saw.
fn connect_proxy() -> (String, std::sync::mpsc::Receiver<String>) {
    use std::io::{BufRead, BufReader, Write};
    use std::net::{TcpListener, TcpStream};

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut client) = stream else { return };
            let mut reader = BufReader::new(client.try_clone().unwrap());
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                continue;
            }
            loop {
                let mut header = String::new();
                if reader.read_line(&mut header).unwrap_or(0) == 0 || header == "\r\n" {
                    break;
                }
            }
            let _ = sender.send(line.clone());
            let Some(target) = line
                .strip_prefix("CONNECT ")
                .and_then(|rest| rest.split(' ').next())
            else {
                continue;
            };
            let Ok(upstream) = TcpStream::connect(target) else {
                let _ = client.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n");
                continue;
            };
            let _ = client.write_all(b"HTTP/1.1 200 Connection established\r\n\r\n");
            let (mut client_read, mut upstream_write) =
                (client.try_clone().unwrap(), upstream.try_clone().unwrap());
            std::thread::spawn(move || {
                let _ = std::io::copy(&mut client_read, &mut upstream_write);
            });
            let mut upstream_read = upstream;
            let _ = std::io::copy(&mut upstream_read, &mut client);
        }
    });
    (format!("http://127.0.0.1:{port}"), receiver)
}

#[tokio::test]
async fn direct_reads_go_through_the_hosts_runtime_proxy() {
    use crate::config::schema::{ProxyConfig, ProxyScope};
    use crate::config::{runtime_proxy_config, set_runtime_proxy_config};

    let _module = module_guard().await;
    let _env = crate::config::TEST_ENV_LOCK.lock().await;
    let tmp = tempfile::tempdir().unwrap();
    let config = module_test_config(&tmp);
    let (proxy, proxied) = connect_proxy();
    let hits = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route(
            "/connected_accounts",
            get(|State(hits): State<Arc<AtomicUsize>>| async move {
                hits.fetch_add(1, Ordering::SeqCst);
                Json(json!({"items": [{"id": "ca_p", "toolkit": "gmail", "status": "ACTIVE"}]}))
            }),
        )
        .with_state(hits.clone());
    let base = start_mock_backend(app).await;
    let tool = direct_tool_for_mock_with_key(base.clone(), "ck_test_proxy");
    let _auth_guard = DirectAuthFailureGuard::for_tool(&tool);

    let previous = runtime_proxy_config();
    set_runtime_proxy_config(ProxyConfig {
        enabled: true,
        http_proxy: Some(proxy.clone()),
        // Only Composio's traffic: the runtime proxy is process-global, and
        // every other test's loopback requests must stay direct meanwhile.
        scope: ProxyScope::Services,
        services: vec!["tool.composio".into()],
        ..ProxyConfig::default()
    });
    let through = direct_list_connections(&config, &tool).await;
    // A destination on the no-proxy list is called directly even with a proxy.
    set_runtime_proxy_config(ProxyConfig {
        enabled: true,
        http_proxy: Some(proxy),
        no_proxy: vec!["127.0.0.1".into()],
        scope: ProxyScope::Services,
        services: vec!["tool.composio".into()],
        ..ProxyConfig::default()
    });
    let bypassed_proxied_before = proxied.try_iter().count();
    let bypassed = direct_list_connections(&config, &tool).await;
    let bypassed_proxied = proxied.try_iter().count();
    set_runtime_proxy_config(previous);

    assert_eq!(through.unwrap().connections[0].id, "ca_p");
    assert_eq!(
        bypassed_proxied_before, 1,
        "the first read must have been tunnelled by the proxy"
    );
    assert!(bypassed.is_ok());
    assert_eq!(
        bypassed_proxied, 0,
        "a no_proxy destination skips the proxy"
    );
    assert_eq!(
        hits.load(Ordering::SeqCst),
        2,
        "both reads reached Composio"
    );
}
