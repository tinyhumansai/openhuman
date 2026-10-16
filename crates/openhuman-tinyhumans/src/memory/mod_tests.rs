use super::*;
use crate::hosted::test_support;
use openhuman_embed::__host::memory_engines::Binding;
use openhuman_embed::__host::security::credentials::session_support::BackendCredential;
use openhuman_embed::__host::security::credentials::{
    AuthService, APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME,
};
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use billing::{active_with_cache, digest, fetch, parse_active, Answer, CACHE_TTL};
use engine::HostBearer;

const KEY: &str = "https://backend.test";

fn config_with_endpoint(
    tmp: &tempfile::TempDir,
    endpoint: &str,
) -> openhuman_embed::__host::config::Config {
    let mut config = test_support::config(tmp, "http://127.0.0.1:9");
    config.memory.engine = TINYHUMANS_ENGINE.to_string();
    config
        .memory
        .engines
        .entry(TINYHUMANS_ENGINE.to_string())
        .or_default()
        .endpoint = Some(endpoint.to_string());
    config
}

fn request<'a>(
    config: &'a openhuman_embed::__host::config::Config,
    root: Option<&'a str>,
) -> openhuman_embed::__host::memory_engines::EngineRequest<'a> {
    openhuman_embed::__host::memory_engines::EngineRequest { config, root }
}

fn store_key(config: &openhuman_embed::__host::config::Config, key: &str) {
    test_support::store_session(config, key);
}

fn off_reason(binding: Binding) -> String {
    match binding {
        Binding::Off { reason, .. } => reason,
        Binding::On(bound) => panic!("expected off, bound {}", bound.id),
    }
}

// ── engine ──────────────────────────────────────────────────────────────────

#[test]
fn without_a_credential_the_engine_is_off_and_reports_its_endpoint() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_endpoint(&tmp, "https://memory.example.test");
    match engine::bind(&request(&config, None)) {
        Binding::Off {
            engine, endpoint, ..
        } => {
            assert_eq!(engine.as_deref(), Some(TINYHUMANS_ENGINE));
            assert_eq!(endpoint.as_deref(), Some("https://memory.example.test"));
        }
        Binding::On(_) => panic!("expected off"),
    }
}

#[test]
fn a_local_session_token_is_off() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_endpoint(&tmp, "https://memory.example.test");
    store_key(&config, "desktop.test.local");
    let reason = off_reason(engine::bind(&request(&config, None)));
    assert!(reason.contains("sign in"), "{reason}");
}

#[test]
fn the_host_credential_binds_below_the_endpoint_and_is_cached() {
    // The engine cache is keyed by a fingerprint that includes the attribution
    // headers, which derive from the process-global product identity and the
    // shell-version variable. Hold both locks so neither changes between the
    // two binds.
    let _identity = crate::backend::product::product_identity_test_lock();
    let _env = crate::backend::url::backend_env_test_lock();
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_endpoint(&tmp, "https://memory.example.test");
    store_key(&config, "jwt.memory.test");
    let first = match engine::bind(&request(&config, None)) {
        Binding::On(bound) => bound,
        Binding::Off { reason, .. } => panic!("expected bound: {reason}"),
    };
    assert_eq!(first.id, TINYHUMANS_ENGINE);
    assert_eq!(first.endpoint, "https://memory.example.test");
    let second = match engine::bind(&request(&config, None)) {
        Binding::On(bound) => bound,
        Binding::Off { reason, .. } => panic!("expected bound: {reason}"),
    };
    assert!(
        std::sync::Arc::ptr_eq(&first.engine, &second.engine),
        "cached engine reused"
    );
}

#[test]
fn the_layout_root_gets_its_own_cached_engine() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_endpoint(&tmp, "https://memory.example.test");
    store_key(&config, "jwt.memory.layout");
    let legacy = engine::bind(&request(&config, None));
    let rooted = engine::bind(&request(&config, Some("org:42")));
    match (legacy, rooted) {
        (Binding::On(legacy), Binding::On(rooted)) => {
            assert!(!std::sync::Arc::ptr_eq(&legacy.engine, &rooted.engine));
        }
        _ => panic!("both layouts bind"),
    }
}

#[test]
fn the_default_endpoint_is_the_control_plane_origin() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = test_support::config(&tmp, "https://api.override.test");
    config.memory.engine = TINYHUMANS_ENGINE.to_string();
    assert_eq!(
        engine::endpoint_for(&config),
        crate::backend::url::effective_backend_api_url(&config.api_url)
    );
}

#[tokio::test]
async fn host_bearer_reads_the_credential_per_request() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_support::config(&tmp, "http://127.0.0.1:9");
    let source = HostBearer {
        config: std::sync::Arc::new(config.clone()),
    };
    let error = openhuman_embed::__host::memory_engines::BearerSource::bearer(&source)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        openhuman_embed::memory::api::Error::Unauthorized(_)
    ));

    store_key(&config, "jwt.bearer.test");
    assert_eq!(
        openhuman_embed::__host::memory_engines::BearerSource::bearer(&source)
            .await
            .unwrap(),
        "jwt.bearer.test"
    );
}

#[tokio::test]
async fn host_bearer_rejects_a_local_session_token() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_support::config(&tmp, "http://127.0.0.1:9");
    AuthService::from_config(&config)
        .store_provider_token(
            APP_SESSION_PROVIDER,
            DEFAULT_AUTH_PROFILE_NAME,
            "desktop.test.local",
            std::collections::HashMap::new(),
            true,
        )
        .ok();
    store_key(&config, "desktop.test.local");
    let source = HostBearer {
        config: std::sync::Arc::new(config),
    };
    let error = openhuman_embed::__host::memory_engines::BearerSource::bearer(&source)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        openhuman_embed::memory::api::Error::Unauthorized(_)
    ));
}

#[test]
fn registering_twice_leaves_one_engine() {
    let _registry = crate::registry_test_lock();

    register(&InstallOptions::default());
    register(&InstallOptions::default());
    assert!(openhuman_embed::__host::memory_engines::unregister_memory_engine(TINYHUMANS_ENGINE));
}

// ── billing ─────────────────────────────────────────────────────────────────

#[test]
fn parse_active_reads_the_flag_and_refuses_anything_else() {
    assert_eq!(
        parse_active(&json!({"active": true, "until": "x"})),
        Ok(true)
    );
    assert_eq!(
        parse_active(&json!({"active": false, "until": null})),
        Ok(false)
    );
    assert!(parse_active(&json!({"active": "yes"})).is_err());
    assert!(parse_active(&json!({})).is_err());
    assert_eq!(
        parse_active(&json!({"success": true, "data": {"active": true}})),
        Ok(true),
        "an answer still in its envelope reads the same"
    );
    assert!(parse_active(&json!({"success": true, "data": {}})).is_err());
}

#[tokio::test]
async fn an_answer_is_reused_within_the_ttl_and_asked_again_after() {
    let cache = Answer::new();
    let base = Instant::now();
    let calls = AtomicUsize::new(0);
    let ask = |active: bool| {
        calls.fetch_add(1, Ordering::SeqCst);
        async move { Ok(active) }
    };
    assert!(active_with_cache(&cache, KEY, CACHE_TTL, || base, || ask(true)).await);
    assert!(
        active_with_cache(
            &cache,
            KEY,
            CACHE_TTL,
            || base + Duration::from_secs(30),
            || ask(false)
        )
        .await,
        "still the cached answer"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(
        !active_with_cache(
            &cache,
            KEY,
            CACHE_TTL,
            || base + Duration::from_secs(61),
            || ask(false)
        )
        .await,
        "asked again once stale"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn an_error_is_not_free_and_is_not_asked_again_at_once() {
    let cache = Answer::new();
    let base = Instant::now();
    let calls = AtomicUsize::new(0);
    let failing = || {
        calls.fetch_add(1, Ordering::SeqCst);
        async { Err::<bool, _>("GET /memory/free-period failed (404 Not Found)".to_string()) }
    };
    assert!(!active_with_cache(&cache, KEY, CACHE_TTL, || base, failing).await);
    assert!(!active_with_cache(&cache, KEY, CACHE_TTL, || base, failing).await);
    assert_eq!(calls.load(Ordering::SeqCst), 1, "a failure is cached too");
}

#[tokio::test]
async fn each_backend_keeps_its_own_answer() {
    let cache = Answer::new();
    let base = Instant::now();
    let calls = AtomicUsize::new(0);
    let ask = |active: bool| {
        calls.fetch_add(1, Ordering::SeqCst);
        async move { Ok(active) }
    };
    assert!(active_with_cache(&cache, "https://a.test", CACHE_TTL, || base, || ask(true)).await);
    assert!(!active_with_cache(&cache, "https://b.test", CACHE_TTL, || base, || ask(false)).await);
    assert!(active_with_cache(&cache, "https://a.test", CACHE_TTL, || base, || ask(false)).await);
    assert_eq!(calls.load(Ordering::SeqCst), 2, "a's answer survived b's");
}

#[tokio::test]
async fn concurrent_callers_share_one_request() {
    let cache = Answer::new();
    let base = Instant::now();
    let calls = AtomicUsize::new(0);
    let ask = || {
        calls.fetch_add(1, Ordering::SeqCst);
        async {
            tokio::time::sleep(Duration::from_millis(20)).await;
            Ok(true)
        }
    };
    let (a, b) = tokio::join!(
        active_with_cache(&cache, KEY, CACHE_TTL, || base, ask),
        active_with_cache(&cache, KEY, CACHE_TTL, || base, ask),
    );
    assert!(a && b);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn an_answer_is_timed_from_when_it_arrives() {
    let cache = Answer::new();
    let base = Instant::now();
    let calls = AtomicUsize::new(0);
    let ticks = AtomicUsize::new(0);
    // The fetch takes longer than the TTL: asked at `base`, answered at
    // `base + 90s`.
    let clock = || {
        if ticks.fetch_add(1, Ordering::SeqCst) == 0 {
            base
        } else {
            base + Duration::from_secs(90)
        }
    };
    let ask = || {
        calls.fetch_add(1, Ordering::SeqCst);
        async { Ok(true) }
    };
    assert!(active_with_cache(&cache, KEY, CACHE_TTL, clock, ask).await);
    let later = || base + Duration::from_secs(120);
    assert!(active_with_cache(&cache, KEY, CACHE_TTL, later, ask).await);
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "still fresh 30s after it arrived"
    );
}

#[test]
fn the_cache_key_digest_is_the_full_sha256() {
    let key = digest("th_account_a");
    assert_eq!(key.len(), 64, "{key}");
    assert_ne!(key, digest("th_account_b"));
}

async fn backend_answering(status: u16, body: serde_json::Value) -> wiremock::MockServer {
    use wiremock::matchers::{method, path};
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(method("GET"))
        .and(path("/memory/free-period"))
        .respond_with(wiremock::ResponseTemplate::new(status).set_body_json(body))
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn the_backend_says_the_period_is_on_or_off() {
    let on = backend_answering(
        200,
        json!({"success": true, "data": {"active": true, "until": "2026-11-06T00:00:00.000Z"}}),
    )
    .await;
    let off = backend_answering(
        200,
        json!({"success": true, "data": {"active": false, "until": null}}),
    )
    .await;
    let credential = BackendCredential::Session("jwt.free.test".to_string());
    assert_eq!(fetch(&credential, &on.uri()).await, Ok(true));
    assert_eq!(fetch(&credential, &off.uri()).await, Ok(false));
}

#[tokio::test]
async fn a_backend_without_the_route_is_an_error_read_as_not_free() {
    let server = backend_answering(404, json!({"success": false, "error": "Not Found"})).await;
    let credential = BackendCredential::Session("jwt.free.test".to_string());
    assert!(fetch(&credential, &server.uri()).await.is_err());
    let cache = Answer::new();
    let uri = server.uri();
    assert!(
        !active_with_cache(&cache, &uri, CACHE_TTL, Instant::now, || fetch(
            &credential,
            &uri
        ))
        .await
    );
}

/// The hosted engine bound for `config` (which must hold a credential).
fn bound_for(
    config: &openhuman_embed::__host::config::Config,
) -> openhuman_embed::__host::memory_engines::BoundEngine {
    match engine::bind(&request(config, None)) {
        Binding::On(bound) => bound,
        Binding::Off { reason, .. } => panic!("expected bound: {reason}"),
    }
}

async fn registered_free_period(config: openhuman_embed::__host::config::Config) -> bool {
    let bound = bound_for(&config);
    billing::free_period_active(config, bound).await
}

#[tokio::test]
async fn the_engines_own_endpoint_is_asked_not_the_apps() {
    let tmp = tempfile::tempdir().unwrap();
    let server = backend_answering(
        200,
        json!({"success": true, "data": {"active": true, "until": null}}),
    )
    .await;
    let config = config_with_endpoint(&tmp, &server.uri());
    store_key(&config, "jwt.endpoint.test");
    assert!(registered_free_period(config).await);
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn each_account_keeps_its_own_answer_on_the_hosted_engine() {
    let server = backend_answering(
        200,
        json!({"success": true, "data": {"active": true, "until": null}}),
    )
    .await;
    for key in ["jwt.account.a", "jwt.account.b"] {
        let tmp = tempfile::tempdir().unwrap();
        let config = config_with_endpoint(&tmp, &server.uri());
        store_key(&config, key);
        assert!(registered_free_period(config).await);
    }
    assert_eq!(
        server.received_requests().await.unwrap().len(),
        2,
        "the second account asked for its own answer"
    );
}

#[tokio::test]
async fn a_signed_out_account_is_not_free() {
    let tmp = tempfile::tempdir().unwrap();
    let signed_in = config_with_endpoint(&tmp, "http://127.0.0.1:9");
    store_key(&signed_in, "jwt.signed.out");
    let bound = bound_for(&signed_in);
    let other = tempfile::tempdir().unwrap();
    let signed_out = config_with_endpoint(&other, "http://127.0.0.1:9");
    assert!(!billing::free_period_active(signed_out, bound).await);
}
