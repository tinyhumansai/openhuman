//! The backend (proxy) route and the direct route answer the same question the
//! same way.
//!
//! Both routes are served by one module (`ListConnectedIntegrations`), but they
//! reach different upstreams with different wire shapes: the hosted backend's
//! `/agent-integrations/composio/*` envelopes, and Composio's own v3 API. One
//! mock serves both for the same account, and the host's overview — what the
//! agent prompt is built from — must come out equal, or a user who switches
//! route would see a different set of connections and tools.

use super::*;

/// The account both upstreams describe: one active Gmail connection with three
/// curated actions, one of them (`DELETE`) admin-scoped.
fn parity_router() -> Router {
    Router::new()
        // Hosted backend route.
        .route(
            "/agent-integrations/composio/toolkits",
            get(|| async { Json(json!({"success": true, "data": {"toolkits": ["gmail"]}})) }),
        )
        .route(
            "/agent-integrations/composio/connections",
            get(|| async {
                Json(json!({"success": true, "data": {"connections": [
                    {"id": "c1", "toolkit": "gmail", "status": "ACTIVE"}
                ]}}))
            }),
        )
        .route(
            "/agent-integrations/composio/tools",
            get(|| async { Json(json!({"success": true, "data": {"tools": backend_tools()}})) }),
        )
        // Composio v3, the direct route.
        .route(
            "/connected_accounts",
            get(|| async {
                Json(json!({"items": [
                    {"id": "c1", "toolkit_slug": "gmail", "status": "ACTIVE"}
                ]}))
            }),
        )
        .route(
            "/tools",
            get(|| async { Json(json!({"items": direct_tools()})) }),
        )
}

const SLUGS: [&str; 3] = [
    "GMAIL_FETCH_EMAILS",
    "GMAIL_SEND_EMAIL",
    "GMAIL_DELETE_MESSAGE",
];

fn backend_tools() -> Vec<Value> {
    SLUGS
        .iter()
        .map(|slug| json!({"type": "function", "function": {"name": slug, "description": "x"}}))
        .collect()
}

fn direct_tools() -> Vec<Value> {
    SLUGS
        .iter()
        .map(|slug| json!({"slug": slug, "description": "x"}))
        .collect()
}

/// What a route's overview says about the account, ignoring wording that
/// legitimately differs between upstreams (descriptions).
#[derive(Debug, PartialEq, Eq)]
struct Shape {
    toolkit: String,
    connected: bool,
    non_active_status: Option<String>,
    tools: Vec<String>,
    gated: Vec<(String, String)>,
    connections: Vec<(String, bool)>,
}

fn shapes(integrations: Vec<crate::agent::prompts::ConnectedIntegration>) -> Vec<Shape> {
    let mut out: Vec<Shape> = integrations
        .into_iter()
        .map(|i| {
            let mut tools: Vec<String> = i.tools.into_iter().map(|t| t.name).collect();
            tools.sort();
            let mut gated: Vec<(String, String)> = i
                .gated_tools
                .into_iter()
                .map(|g| (g.name, g.required_scope))
                .collect();
            gated.sort();
            Shape {
                toolkit: i.toolkit,
                connected: i.connected,
                non_active_status: i.non_active_status,
                tools,
                gated,
                connections: i
                    .connections
                    .into_iter()
                    .map(|c| (c.connection_id, c.is_default))
                    .collect(),
            }
        })
        .collect();
    out.sort_by(|a, b| a.toolkit.cmp(&b.toolkit));
    out
}

#[tokio::test]
async fn backend_and_direct_routes_report_the_same_connected_integrations() {
    // Lock order: module, then the integrations cache, then process env.
    let _module = module_guard().await;
    let _cache = cache_guard_async().await;
    let _env = crate::config::TEST_ENV_LOCK.lock().await;
    let _backend_env = crate::config::app_env::env_test_lock_async().await;

    let base = start_mock_backend(parity_router()).await;
    let _backend_url = EnvVarGuard::set("BACKEND_URL", &base);
    let _direct_v3 = EnvVarGuard::set("OPENHUMAN_COMPOSIO_DIRECT_BASE_V3", &base);
    let _direct_v2 = EnvVarGuard::set("OPENHUMAN_COMPOSIO_DIRECT_BASE_V2", &base);

    let backend_tmp = tempfile::tempdir().unwrap();
    let backend_config = config_with_backend(&backend_tmp, base.clone());
    let _workspace = EnvVarGuard::workspace_unlocked(backend_tmp.path());
    backend_config.save().await.unwrap();
    invalidate_connected_integrations_cache();
    let backend = shapes(fetch_connected_integrations(&backend_config).await);

    let direct_tmp = tempfile::tempdir().unwrap();
    let mut direct_config = direct_mode_config(&direct_tmp);
    direct_config.api_url = Some(base);
    invalidate_connected_integrations_cache();
    let direct = shapes(fetch_connected_integrations(&direct_config).await);

    assert_eq!(backend.len(), 1, "backend overview: {backend:?}");
    assert_eq!(backend[0].toolkit, "gmail");
    assert!(backend[0].connected);
    assert_eq!(backend[0].connections, vec![("c1".to_owned(), true)]);
    // The default preference hides the admin-scoped delete and gates it.
    assert_eq!(backend[0].tools, ["GMAIL_FETCH_EMAILS", "GMAIL_SEND_EMAIL"]);
    assert_eq!(
        backend[0].gated,
        [("GMAIL_DELETE_MESSAGE".to_owned(), "admin".to_owned())]
    );
    assert_eq!(
        direct, backend,
        "the direct route must describe the account exactly as the backend route does"
    );
}
