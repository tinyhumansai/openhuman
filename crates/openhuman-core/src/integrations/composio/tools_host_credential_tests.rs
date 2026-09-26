use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{json, Value};
use tinytools::Tool;

use crate::config::{ComposioHostCredential, Config};
use crate::integrations::composio::client::{create_composio_client, ComposioClientKind};
use crate::integrations::composio::connected_integrations::{
    cache_key, composio_cache_test_lock, CachedIntegrations, INTEGRATIONS_CACHE,
};
use crate::integrations::composio::tools::{live_composio_config, ComposioListConnectionsTool};

const KEY_A: &str = "ck_tenant_alpha_0001";
const KEY_B: &str = "ck_tenant_bravo_0002";
const SHARED_STORE_KEY: &str = "ck_shared_store_9999";
const LEAKY_KEY: &str = "ck_tenant_leaky_0003";

type SeenKeys = Arc<Mutex<Vec<String>>>;

async fn connected_accounts(
    State(seen): State<SeenKeys>,
    headers: HeaderMap,
) -> (StatusCode, Json<Value>) {
    let key = headers
        .get("x-api-key")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    seen.lock().unwrap().push(key.clone());
    match key.as_str() {
        KEY_A => (
            StatusCode::OK,
            Json(
                json!({"items": [{"id": "ca_alpha", "status": "ACTIVE", "toolkit": {"slug": "gmail"}}]}),
            ),
        ),
        KEY_B => (
            StatusCode::OK,
            Json(
                json!({"items": [{"id": "ca_bravo", "status": "ACTIVE", "toolkit": {"slug": "slack"}}]}),
            ),
        ),
        other => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": {"message": format!("unknown api key {other}")}})),
        ),
    }
}

async fn start_mock_composio() -> (String, SeenKeys) {
    let seen: SeenKeys = Arc::new(Mutex::new(Vec::new()));
    let app = Router::new()
        .route("/v3/connected_accounts", get(connected_accounts))
        .with_state(seen.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://127.0.0.1:{}", addr.port()), seen)
}

/// A runtime config whose disk file and credential store both point
/// somewhere other than the pinned credential.
fn shared_runtime_config(tmp: &tempfile::TempDir) -> Config {
    let config_path = tmp.path().join("config.toml");
    std::fs::write(&config_path, "[composio]\nmode = \"backend\"\n").unwrap();
    let mut config = Config::default();
    config.config_path = config_path;
    config.workspace_dir = tmp.path().join("workspace");
    crate::security::credentials::AuthService::from_config(&config)
        .store_provider_token(
            crate::security::credentials::COMPOSIO_DIRECT_PROVIDER,
            crate::security::credentials::DEFAULT_AUTH_PROFILE_NAME,
            SHARED_STORE_KEY,
            std::collections::HashMap::new(),
            true,
        )
        .unwrap();
    config
}

fn pinned(base: &Config, key: &str, entity: &str, mock: Option<&str>) -> Config {
    let mut config = base.clone();
    let mut credential = ComposioHostCredential::direct(key).entity_id(entity);
    if let Some(root) = mock {
        credential = credential.base_urls(format!("{root}/v2"), format!("{root}/v3"));
    }
    config.composio.pin_host_credential(credential);
    config
}

fn direct_key_fingerprint(config: &Config) -> u64 {
    match create_composio_client(config).expect("pinned credential resolves") {
        ComposioClientKind::Direct(tool) => tool.auth_key_fingerprint(),
        ComposioClientKind::Backend(_) => panic!("a pinned credential must resolve direct"),
    }
}

#[test]
fn pinned_credential_beats_the_shared_credential_store() {
    let tmp = tempfile::tempdir().unwrap();
    let base = shared_runtime_config(&tmp);
    let agent = pinned(&base, KEY_A, "tenant-a", None);

    let mut unpinned_direct = base.clone();
    unpinned_direct.composio.mode = crate::config::schema::COMPOSIO_MODE_DIRECT.into();
    unpinned_direct.composio.api_key = Some(KEY_A.into());

    let fp = crate::integrations::composio::direct_auth::fingerprint_api_key;
    assert_eq!(direct_key_fingerprint(&agent), fp(KEY_A));
    assert_eq!(
        direct_key_fingerprint(&unpinned_direct),
        fp(SHARED_STORE_KEY)
    );
}

#[tokio::test]
async fn pinned_config_is_not_replaced_by_the_shared_config_file() {
    let tmp = tempfile::tempdir().unwrap();
    let base = shared_runtime_config(&tmp);
    let agent = pinned(&base, KEY_A, "tenant-a", None);

    let live = live_composio_config(&agent).await.unwrap();
    assert_eq!(
        live.composio.mode,
        crate::config::schema::COMPOSIO_MODE_DIRECT
    );
    assert_eq!(live.composio.entity_id, "tenant-a");
    assert_eq!(
        live.composio.host_credential.as_ref().map(|c| c.api_key()),
        Some(KEY_A)
    );

    let unpinned = live_composio_config(&base).await.unwrap();
    assert_eq!(
        unpinned.composio.mode,
        crate::config::schema::COMPOSIO_MODE_BACKEND
    );
    assert!(unpinned.composio.host_credential.is_none());
}

#[tokio::test]
async fn two_agents_on_one_runtime_each_call_composio_with_their_own_key() {
    let tmp = tempfile::tempdir().unwrap();
    let base = shared_runtime_config(&tmp);
    let (mock, seen) = start_mock_composio().await;
    let agent_a = pinned(&base, KEY_A, "tenant-a", Some(&mock));
    let agent_b = pinned(&base, KEY_B, "tenant-b", Some(&mock));

    let out_a = ComposioListConnectionsTool::new(Arc::new(agent_a))
        .execute(json!({}))
        .await
        .unwrap();
    let out_b = ComposioListConnectionsTool::new(Arc::new(agent_b))
        .execute(json!({}))
        .await
        .unwrap();

    assert!(!out_a.is_error, "{}", out_a.output());
    assert!(!out_b.is_error, "{}", out_b.output());
    assert!(out_a.output().contains("ca_alpha"), "{}", out_a.output());
    assert!(!out_a.output().contains("ca_bravo"), "{}", out_a.output());
    assert!(out_b.output().contains("ca_bravo"), "{}", out_b.output());
    assert!(!out_b.output().contains("ca_alpha"), "{}", out_b.output());
    assert_eq!(
        *seen.lock().unwrap(),
        vec![KEY_A.to_string(), KEY_B.to_string()]
    );
}

#[test]
fn connected_integrations_cache_is_keyed_per_credential() {
    let _guard = composio_cache_test_lock();
    let tmp = tempfile::tempdir().unwrap();
    let base = shared_runtime_config(&tmp);
    let agent_a = pinned(&base, KEY_A, "tenant-a", None);
    let agent_b = pinned(&base, KEY_B, "tenant-b", None);
    let same_key_other_entity = pinned(&base, KEY_A, "tenant-c", None);

    assert_ne!(cache_key(&agent_a), cache_key(&agent_b));
    assert_ne!(cache_key(&agent_a), cache_key(&base));
    assert_ne!(cache_key(&agent_a), cache_key(&same_key_other_entity));
    assert_eq!(cache_key(&agent_a), cache_key(&agent_a.clone()));
    assert!(!cache_key(&agent_a).contains(KEY_A));

    INTEGRATIONS_CACHE.write().unwrap().insert(
        cache_key(&agent_a),
        CachedIntegrations {
            entries: vec![crate::agent::prompts::ConnectedIntegration {
                toolkit: "gmail".into(),
                description: String::new(),
                tools: Vec::new(),
                gated_tools: Vec::new(),
                connected: true,
                connections: Vec::new(),
                non_active_status: None,
            }],
            cached_at: std::time::Instant::now(),
        },
    );

    let seen_by_a = crate::integrations::composio::cached_active_integrations(&agent_a);
    let seen_by_b = crate::integrations::composio::cached_active_integrations(&agent_b);
    INTEGRATIONS_CACHE
        .write()
        .unwrap()
        .remove(&cache_key(&agent_a));

    assert_eq!(seen_by_a.map(|v| v.len()), Some(1));
    assert!(seen_by_b.is_none());
}

#[tokio::test]
async fn composio_key_never_reaches_tool_output_or_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let base = shared_runtime_config(&tmp);
    let (mock, seen) = start_mock_composio().await;
    let agent = pinned(&base, LEAKY_KEY, "tenant-l", Some(&mock));

    let outcome = ComposioListConnectionsTool::new(Arc::new(agent))
        .execute(json!({}))
        .await;
    let rendered = match outcome {
        Ok(result) => result.output(),
        Err(error) => format!("{error:#}"),
    };

    assert_eq!(*seen.lock().unwrap(), vec![LEAKY_KEY.to_string()]);
    assert!(!rendered.contains(LEAKY_KEY), "{rendered}");
    assert!(rendered.contains("[REDACTED]"), "{rendered}");
}

#[test]
fn redaction_covers_every_result_channel() {
    use crate::integrations::composio::tools::redact_composio_outcome;
    let tmp = tempfile::tempdir().unwrap();
    let base = shared_runtime_config(&tmp);
    let agent = pinned(&base, KEY_A, "tenant-a", None);

    let mut result = tinytools::ToolResult::json(json!({"nested": [format!("key={KEY_A}")]}))
        .with_markdown(format!("md {KEY_A}"))
        .with_metadata(json!({"k": KEY_A}));
    result
        .content
        .push(tinytools::ToolContent::Text { text: KEY_A.into() });
    let redacted = redact_composio_outcome(&agent, Ok(result)).unwrap();
    let serialized = serde_json::to_string(&redacted).unwrap();
    assert!(!serialized.contains(KEY_A), "{serialized}");

    let err = redact_composio_outcome(&agent, Err(anyhow::anyhow!("bad key {KEY_A}"))).unwrap_err();
    assert!(!format!("{err:#}").contains(KEY_A));

    let debug = format!("{:?}", agent.composio);
    assert!(!debug.contains(KEY_A), "{debug}");
}

/// Regression for CodeRabbit finding bdd9216d on PR #6689: two agents that
/// share a `config_path` (the normal multi-agent-per-runtime shape) and are
/// both unpinned direct mode resolve to the SAME cache key regardless of
/// which stored credential is actually active, because `cache_key` never
/// hashed the effective `COMPOSIO_DIRECT_PROVIDER` key. The generic
/// credential RPCs can rotate that stored key in place without publishing
/// `ComposioConfigChanged`, so — before this fix — an agent reading the
/// cache right after a rotation would transparently receive the previous
/// credential's cached connection list. Two DIFFERENT credentials must
/// never collide on one cache identity.
#[test]
fn cache_key_changes_when_the_shared_stored_key_rotates_in_unpinned_direct_mode() {
    let tmp = tempfile::tempdir().unwrap();
    let mut base = shared_runtime_config(&tmp);
    base.composio.mode = crate::config::schema::COMPOSIO_MODE_DIRECT.into();

    let key_before = cache_key(&base);

    crate::security::credentials::AuthService::from_config(&base)
        .store_provider_token(
            crate::security::credentials::COMPOSIO_DIRECT_PROVIDER,
            crate::security::credentials::DEFAULT_AUTH_PROFILE_NAME,
            "ck_tenant_rotated_0004",
            std::collections::HashMap::new(),
            true,
        )
        .unwrap();

    let key_after = cache_key(&base);
    assert_ne!(
        key_before, key_after,
        "cache identity must change when the effective stored Composio credential rotates"
    );
    assert!(!key_after.contains("ck_tenant_rotated_0004"), "{key_after}");
}

/// Sanity counterpart: an unchanged stored key must keep producing the same
/// cache identity (no key material, no unrelated flapping).
#[test]
fn cache_key_is_stable_for_unpinned_direct_mode_when_nothing_rotates() {
    let tmp = tempfile::tempdir().unwrap();
    let mut base = shared_runtime_config(&tmp);
    base.composio.mode = crate::config::schema::COMPOSIO_MODE_DIRECT.into();

    assert_eq!(cache_key(&base), cache_key(&base));
}
