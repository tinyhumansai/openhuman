//! Gateway module tests.

use super::client::{client_key_from_request, normalize_max_keys};
use super::constants::{
    hash_webhook_secret, linq_memory_key, webhook_memory_key, whatsapp_memory_key, MAX_BODY_SIZE,
    RATE_LIMITER_SWEEP_INTERVAL_SECS, REQUEST_TIMEOUT_SECS,
};
use super::handlers::{
    handle_metrics, handle_webhook, persist_pairing_tokens, verify_whatsapp_signature,
    PROMETHEUS_CONTENT_TYPE,
};
use super::models::{WebhookBody, WhatsAppVerifyQuery};
use super::rate_limit::{GatewayRateLimiter, IdempotencyStore, SlidingWindowRateLimiter};
use super::state::AppState;
use crate::alphahuman::channels::traits::ChannelMessage;
use crate::alphahuman::config::Config;
use crate::alphahuman::memory::{Memory, MemoryCategory, MemoryEntry};
use crate::alphahuman::providers::Provider;
use crate::alphahuman::security::pairing::PairingGuard;
use async_trait::async_trait;
use axum::extract::ConnectInfo;
use axum::http::HeaderValue;
use axum::response::IntoResponse;
use axum::{http::HeaderMap, Json};
use http_body_util::BodyExt;
use parking_lot::Mutex;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Generate a random hex secret at runtime to avoid hard-coded cryptographic values.
fn generate_test_secret() -> String {
    use rand::Rng;
    let bytes: [u8; 32] = rand::rng().random();
    hex::encode(bytes)
}

fn webhook_body(message: &str) -> WebhookBody {
    WebhookBody {
        message: Some(message.to_string()),
        model: None,
        temperature: None,
        memory: None,
    }
}

#[test]
fn security_body_limit_is_64kb() {
    assert_eq!(MAX_BODY_SIZE, 65_536);
}

#[test]
fn security_timeout_is_30_seconds() {
    assert_eq!(REQUEST_TIMEOUT_SECS, 30);
}

#[test]
fn webhook_body_accepts_optional_fields() {
    let valid = r#"{"message": "hello", "model": "foo", "temperature": 0.1, "memory": true}"#;
    let parsed: Result<WebhookBody, _> = serde_json::from_str(valid);
    assert!(parsed.is_ok());
    let parsed = parsed.unwrap();
    assert_eq!(parsed.message.as_deref(), Some("hello"));
    assert_eq!(parsed.model.as_deref(), Some("foo"));
    assert_eq!(parsed.temperature, Some(0.1));
    assert_eq!(parsed.memory, Some(true));
}

#[test]
fn whatsapp_query_fields_are_optional() {
    let q = WhatsAppVerifyQuery {
        mode: None,
        verify_token: None,
        challenge: None,
    };
    assert!(q.mode.is_none());
}

#[test]
fn app_state_is_clone() {
    fn assert_clone<T: Clone>() {}
    assert_clone::<AppState>();
}

#[tokio::test]
async fn metrics_endpoint_returns_hint_when_prometheus_is_disabled() {
    let state = AppState {
        config: Arc::new(Mutex::new(Config::default())),
        provider: Arc::new(MockProvider::default()),
        model: "test-model".into(),
        temperature: 0.0,
        mem: Arc::new(MockMemory),
        auto_save: false,
        webhook_secret_hash: None,
        pairing: Arc::new(PairingGuard::new(false, &[])),
        trust_forwarded_headers: false,
        rate_limiter: Arc::new(GatewayRateLimiter::new(100, 100, 100)),
        idempotency_store: Arc::new(IdempotencyStore::new(Duration::from_secs(300), 1000)),
        whatsapp: None,
        whatsapp_app_secret: None,
        linq: None,
        linq_signing_secret: None,
        observer: Arc::new(crate::alphahuman::observability::NoopObserver),
    };

    let response = handle_metrics(axum::extract::State(state)).await.into_response();
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some(PROMETHEUS_CONTENT_TYPE)
    );

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("Prometheus backend not enabled"));
}

#[tokio::test]
async fn metrics_endpoint_renders_prometheus_output() {
    let prom = Arc::new(crate::alphahuman::observability::PrometheusObserver::new());
    crate::alphahuman::observability::Observer::record_event(
        prom.as_ref(),
        &crate::alphahuman::observability::ObserverEvent::HeartbeatTick,
    );

    let observer: Arc<dyn crate::alphahuman::observability::Observer> = prom;
    let state = AppState {
        config: Arc::new(Mutex::new(Config::default())),
        provider: Arc::new(MockProvider::default()),
        model: "test-model".into(),
        temperature: 0.0,
        mem: Arc::new(MockMemory),
        auto_save: false,
        webhook_secret_hash: None,
        pairing: Arc::new(PairingGuard::new(false, &[])),
        trust_forwarded_headers: false,
        rate_limiter: Arc::new(GatewayRateLimiter::new(100, 100, 100)),
        idempotency_store: Arc::new(IdempotencyStore::new(Duration::from_secs(300), 1000)),
        whatsapp: None,
        whatsapp_app_secret: None,
        linq: None,
        linq_signing_secret: None,
        observer,
    };

    let response = handle_metrics(axum::extract::State(state)).await.into_response();
    assert_eq!(response.status(), axum::http::StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("alphahuman_heartbeat_ticks_total 1"));
}

#[test]
fn gateway_rate_limiter_blocks_after_limit() {
    let limiter = GatewayRateLimiter::new(2, 2, 100);
    assert!(limiter.allow_pair("127.0.0.1"));
    assert!(limiter.allow_pair("127.0.0.1"));
    assert!(!limiter.allow_pair("127.0.0.1"));
}

#[test]
fn rate_limiter_sweep_removes_stale_entries() {
    let limiter = SlidingWindowRateLimiter::new(10, Duration::from_secs(60), 100);
    // Add entries for multiple IPs
    assert!(limiter.allow("ip-1"));
    assert!(limiter.allow("ip-2"));
    assert!(limiter.allow("ip-3"));

    {
        let guard = limiter.requests.lock();
        assert_eq!(guard.0.len(), 3);
    }

    // Force a sweep by backdating last_sweep
    {
        let mut guard = limiter.requests.lock();
        guard.1 = Instant::now()
            .checked_sub(Duration::from_secs(RATE_LIMITER_SWEEP_INTERVAL_SECS + 1))
            .unwrap();
        // Clear timestamps for ip-2 and ip-3 to simulate stale entries
        guard.0.get_mut("ip-2").unwrap().clear();
        guard.0.get_mut("ip-3").unwrap().clear();
    }

    // Next allow() call should trigger sweep and remove stale entries
    assert!(limiter.allow("ip-1"));

    {
        let guard = limiter.requests.lock();
        assert_eq!(guard.0.len(), 1, "Stale entries should have been swept");
        assert!(guard.0.contains_key("ip-1"));
    }
}

#[test]
fn rate_limiter_zero_limit_always_allows() {
    let limiter = SlidingWindowRateLimiter::new(0, Duration::from_secs(60), 10);
    for _ in 0..100 {
        assert!(limiter.allow("any-key"));
    }
}

#[test]
fn idempotency_store_rejects_duplicate_key() {
    let store = IdempotencyStore::new(Duration::from_secs(30), 10);
    assert!(store.record_if_new("req-1"));
    assert!(!store.record_if_new("req-1"));
    assert!(store.record_if_new("req-2"));
}

#[test]
fn rate_limiter_bounded_cardinality_evicts_oldest_key() {
    let limiter = SlidingWindowRateLimiter::new(5, Duration::from_secs(60), 2);
    assert!(limiter.allow("ip-1"));
    assert!(limiter.allow("ip-2"));
    assert!(limiter.allow("ip-3"));

    let guard = limiter.requests.lock();
    assert_eq!(guard.0.len(), 2);
    assert!(guard.0.contains_key("ip-2"));
    assert!(guard.0.contains_key("ip-3"));
}

#[test]
fn idempotency_store_bounded_cardinality_evicts_oldest_key() {
    let store = IdempotencyStore::new(Duration::from_secs(300), 2);
    assert!(store.record_if_new("k1"));
    std::thread::sleep(Duration::from_millis(2));
    assert!(store.record_if_new("k2"));
    std::thread::sleep(Duration::from_millis(2));
    assert!(store.record_if_new("k3"));

    assert_eq!(store.len(), 2);
    assert!(!store.contains_key("k1"));
    assert!(store.contains_key("k2"));
    assert!(store.contains_key("k3"));
}

#[test]
fn client_key_defaults_to_peer_addr_when_untrusted_proxy_mode() {
    let peer = SocketAddr::from(([10, 0, 0, 5], 3000));
    let mut headers = HeaderMap::new();
    headers.insert(
        "X-Forwarded-For",
        HeaderValue::from_static("198.51.100.10, 203.0.113.11"),
    );

    let key = client_key_from_request(Some(peer), &headers, false);
    assert_eq!(key, "10.0.0.5");
}

#[test]
fn client_key_uses_forwarded_ip_only_in_trusted_proxy_mode() {
    let peer = SocketAddr::from(([10, 0, 0, 5], 3000));
    let mut headers = HeaderMap::new();
    headers.insert(
        "X-Forwarded-For",
        HeaderValue::from_static("198.51.100.10, 203.0.113.11"),
    );

    let key = client_key_from_request(Some(peer), &headers, true);
    assert_eq!(key, "198.51.100.10");
}

#[test]
fn client_key_falls_back_to_peer_when_forwarded_header_invalid() {
    let peer = SocketAddr::from(([10, 0, 0, 5], 3000));
    let mut headers = HeaderMap::new();
    headers.insert("X-Forwarded-For", HeaderValue::from_static("garbage-value"));

    let key = client_key_from_request(Some(peer), &headers, true);
    assert_eq!(key, "10.0.0.5");
}

#[test]
fn normalize_max_keys_uses_fallback_for_zero() {
    assert_eq!(normalize_max_keys(0, 10_000), 10_000);
    assert_eq!(normalize_max_keys(0, 0), 1);
}

#[test]
fn normalize_max_keys_preserves_nonzero_values() {
    assert_eq!(normalize_max_keys(2_048, 10_000), 2_048);
    assert_eq!(normalize_max_keys(1, 10_000), 1);
}

#[tokio::test]
async fn persist_pairing_tokens_writes_config_tokens() {
    let temp = tempfile::tempdir().unwrap();
    let config_path = temp.path().join("config.toml");
    let workspace_path = temp.path().join("workspace");

    let mut config = Config::default();
    config.config_path = config_path.clone();
    config.workspace_dir = workspace_path;
    config.save().await.unwrap();

    let guard = PairingGuard::new(true, &[]);
    let code = guard.pairing_code().unwrap();
    let token = guard.try_pair(&code).await.unwrap().unwrap();
    assert!(guard.is_authenticated(&token));

    let shared_config = Arc::new(Mutex::new(config));
    persist_pairing_tokens(shared_config.clone(), &guard)
        .await
        .unwrap();

    let saved = tokio::fs::read_to_string(config_path).await.unwrap();
    let parsed: Config = toml::from_str(&saved).unwrap();
    assert_eq!(parsed.gateway.paired_tokens.len(), 1);
    let persisted = &parsed.gateway.paired_tokens[0];
    assert_eq!(persisted.len(), 64);
    assert!(persisted.chars().all(|c| c.is_ascii_hexdigit()));

    let in_memory = shared_config.lock();
    assert_eq!(in_memory.gateway.paired_tokens.len(), 1);
    assert_eq!(&in_memory.gateway.paired_tokens[0], persisted);
}

#[test]
fn webhook_memory_key_is_unique() {
    let key1 = webhook_memory_key();
    let key2 = webhook_memory_key();

    assert!(key1.starts_with("webhook_msg_"));
    assert!(key2.starts_with("webhook_msg_"));
    assert_ne!(key1, key2);
}

#[test]
fn whatsapp_memory_key_includes_sender_and_message_id() {
    let msg = ChannelMessage {
        id: "wamid-123".into(),
        sender: "+1234567890".into(),
        reply_target: "+1234567890".into(),
        content: "hello".into(),
        channel: "whatsapp".into(),
        timestamp: 1,
        thread_ts: None,
    };

    let key = whatsapp_memory_key(&msg);
    assert_eq!(key, "whatsapp_+1234567890_wamid-123");
}

#[test]
fn linq_memory_key_includes_sender_and_message_id() {
    let msg = ChannelMessage {
        id: "linq-456".into(),
        sender: "+1987654321".into(),
        reply_target: "+1987654321".into(),
        content: "hello".into(),
        channel: "linq".into(),
        timestamp: 1,
        thread_ts: None,
    };

    let key = linq_memory_key(&msg);
    assert_eq!(key, "linq_+1987654321_linq-456");
}

#[derive(Default)]
struct MockMemory;

#[async_trait]
impl Memory for MockMemory {
    fn name(&self) -> &str {
        "mock"
    }

    async fn store(
        &self,
        _key: &str,
        _content: &str,
        _category: MemoryCategory,
        _session_id: Option<&str>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn recall(
        &self,
        _query: &str,
        _limit: usize,
        _session_id: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        Ok(Vec::new())
    }

    async fn get(&self, _key: &str) -> anyhow::Result<Option<MemoryEntry>> {
        Ok(None)
    }

    async fn list(
        &self,
        _category: Option<&MemoryCategory>,
        _session_id: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        Ok(Vec::new())
    }

    async fn forget(&self, _key: &str) -> anyhow::Result<bool> {
        Ok(false)
    }

    async fn count(&self) -> anyhow::Result<usize> {
        Ok(0)
    }

    async fn health_check(&self) -> bool {
        true
    }
}

#[derive(Default)]
struct MockProvider {
    calls: AtomicUsize,
}

#[async_trait]
impl Provider for MockProvider {
    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok("ok".into())
    }
}

#[derive(Default)]
struct TrackingMemory {
    keys: Mutex<Vec<String>>,
}

#[async_trait]
impl Memory for TrackingMemory {
    fn name(&self) -> &str {
        "tracking"
    }

    async fn store(
        &self,
        key: &str,
        _content: &str,
        _category: MemoryCategory,
        _session_id: Option<&str>,
    ) -> anyhow::Result<()> {
        self.keys.lock().push(key.to_string());
        Ok(())
    }

    async fn recall(
        &self,
        _query: &str,
        _limit: usize,
        _session_id: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        Ok(Vec::new())
    }

    async fn get(&self, _key: &str) -> anyhow::Result<Option<MemoryEntry>> {
        Ok(None)
    }

    async fn list(
        &self,
        _category: Option<&MemoryCategory>,
        _session_id: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        Ok(Vec::new())
    }

    async fn forget(&self, _key: &str) -> anyhow::Result<bool> {
        Ok(false)
    }

    async fn count(&self) -> anyhow::Result<usize> {
        let size = self.keys.lock().len();
        Ok(size)
    }

    async fn health_check(&self) -> bool {
        true
    }
}

fn test_connect_info() -> ConnectInfo<SocketAddr> {
    ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 30_300)))
}

#[tokio::test]
async fn webhook_idempotency_skips_duplicate_provider_calls() {
    let provider_impl = Arc::new(MockProvider::default());
    let provider: Arc<dyn Provider> = provider_impl.clone();
    let memory: Arc<dyn Memory> = Arc::new(MockMemory);

    let state = AppState {
        config: Arc::new(Mutex::new(Config::default())),
        provider,
        model: "test-model".into(),
        temperature: 0.0,
        mem: memory,
        auto_save: false,
        webhook_secret_hash: None,
        pairing: Arc::new(PairingGuard::new(false, &[])),
        trust_forwarded_headers: false,
        rate_limiter: Arc::new(GatewayRateLimiter::new(100, 100, 100)),
        idempotency_store: Arc::new(IdempotencyStore::new(Duration::from_secs(300), 1000)),
        whatsapp: None,
        whatsapp_app_secret: None,
        linq: None,
        linq_signing_secret: None,
        observer: Arc::new(crate::alphahuman::observability::NoopObserver),
    };

    let mut headers = HeaderMap::new();
    headers.insert("X-Idempotency-Key", HeaderValue::from_static("abc-123"));

    let body = Ok(Json(webhook_body("hello")));
    let first = handle_webhook(
        axum::extract::State(state.clone()),
        test_connect_info(),
        headers.clone(),
        body,
    )
    .await
    .into_response();
    assert_eq!(first.status(), axum::http::StatusCode::OK);

    let body = Ok(Json(webhook_body("hello")));
    let second = handle_webhook(
        axum::extract::State(state),
        test_connect_info(),
        headers,
        body,
    )
    .await
    .into_response();
    assert_eq!(second.status(), axum::http::StatusCode::OK);

    let payload = second.into_body().collect().await.unwrap().to_bytes();
    let parsed: serde_json::Value = serde_json::from_slice(&payload).unwrap();
    assert_eq!(parsed["status"], "duplicate");
    assert_eq!(parsed["idempotent"], true);
    assert_eq!(provider_impl.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn webhook_autosave_stores_distinct_keys_per_request() {
    let provider_impl = Arc::new(MockProvider::default());
    let provider: Arc<dyn Provider> = provider_impl.clone();

    let tracking_impl = Arc::new(TrackingMemory::default());
    let memory: Arc<dyn Memory> = tracking_impl.clone();

    let state = AppState {
        config: Arc::new(Mutex::new(Config::default())),
        provider,
        model: "test-model".into(),
        temperature: 0.0,
        mem: memory,
        auto_save: true,
        webhook_secret_hash: None,
        pairing: Arc::new(PairingGuard::new(false, &[])),
        trust_forwarded_headers: false,
        rate_limiter: Arc::new(GatewayRateLimiter::new(100, 100, 100)),
        idempotency_store: Arc::new(IdempotencyStore::new(Duration::from_secs(300), 1000)),
        whatsapp: None,
        whatsapp_app_secret: None,
        linq: None,
        linq_signing_secret: None,
        observer: Arc::new(crate::alphahuman::observability::NoopObserver),
    };

    let headers = HeaderMap::new();

    let body1 = Ok(Json(webhook_body("hello one")));
    let first = handle_webhook(
        axum::extract::State(state.clone()),
        test_connect_info(),
        headers.clone(),
        body1,
    )
    .await
    .into_response();
    assert_eq!(first.status(), axum::http::StatusCode::OK);

    let body2 = Ok(Json(webhook_body("hello two")));
    let second = handle_webhook(
        axum::extract::State(state),
        test_connect_info(),
        headers,
        body2,
    )
    .await
    .into_response();
    assert_eq!(second.status(), axum::http::StatusCode::OK);

    let keys = tracking_impl.keys.lock().clone();
    assert_eq!(keys.len(), 2);
    assert_ne!(keys[0], keys[1]);
    assert!(keys[0].starts_with("webhook_msg_"));
    assert!(keys[1].starts_with("webhook_msg_"));
    assert_eq!(provider_impl.calls.load(Ordering::SeqCst), 2);
}

#[test]
fn webhook_secret_hash_is_deterministic_and_nonempty() {
    let secret_a = generate_test_secret();
    let secret_b = generate_test_secret();
    let one = hash_webhook_secret(&secret_a);
    let two = hash_webhook_secret(&secret_a);
    let other = hash_webhook_secret(&secret_b);

    assert_eq!(one, two);
    assert_ne!(one, other);
    assert_eq!(one.len(), 64);
}

#[tokio::test]
async fn webhook_secret_hash_rejects_missing_header() {
    let provider_impl = Arc::new(MockProvider::default());
    let provider: Arc<dyn Provider> = provider_impl.clone();
    let memory: Arc<dyn Memory> = Arc::new(MockMemory);
    let secret = generate_test_secret();

    let state = AppState {
        config: Arc::new(Mutex::new(Config::default())),
        provider,
        model: "test-model".into(),
        temperature: 0.0,
        mem: memory,
        auto_save: false,
        webhook_secret_hash: Some(Arc::from(hash_webhook_secret(&secret))),
        pairing: Arc::new(PairingGuard::new(false, &[])),
        trust_forwarded_headers: false,
        rate_limiter: Arc::new(GatewayRateLimiter::new(100, 100, 100)),
        idempotency_store: Arc::new(IdempotencyStore::new(Duration::from_secs(300), 1000)),
        whatsapp: None,
        whatsapp_app_secret: None,
        linq: None,
        linq_signing_secret: None,
        observer: Arc::new(crate::alphahuman::observability::NoopObserver),
    };

    let response = handle_webhook(
        axum::extract::State(state),
        test_connect_info(),
        HeaderMap::new(),
        Ok(Json(webhook_body("hello"))),
    )
    .await
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
    assert_eq!(provider_impl.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn webhook_secret_hash_rejects_invalid_header() {
    let provider_impl = Arc::new(MockProvider::default());
    let provider: Arc<dyn Provider> = provider_impl.clone();
    let memory: Arc<dyn Memory> = Arc::new(MockMemory);
    let valid_secret = generate_test_secret();
    let wrong_secret = generate_test_secret();

    let state = AppState {
        config: Arc::new(Mutex::new(Config::default())),
        provider,
        model: "test-model".into(),
        temperature: 0.0,
        mem: memory,
        auto_save: false,
        webhook_secret_hash: Some(Arc::from(hash_webhook_secret(&valid_secret))),
        pairing: Arc::new(PairingGuard::new(false, &[])),
        trust_forwarded_headers: false,
        rate_limiter: Arc::new(GatewayRateLimiter::new(100, 100, 100)),
        idempotency_store: Arc::new(IdempotencyStore::new(Duration::from_secs(300), 1000)),
        whatsapp: None,
        whatsapp_app_secret: None,
        linq: None,
        linq_signing_secret: None,
        observer: Arc::new(crate::alphahuman::observability::NoopObserver),
    };

    let mut headers = HeaderMap::new();
    headers.insert(
        "X-Webhook-Secret",
        HeaderValue::from_str(&wrong_secret).unwrap(),
    );

    let response = handle_webhook(
        axum::extract::State(state),
        test_connect_info(),
        headers,
        Ok(Json(webhook_body("hello"))),
    )
    .await
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
    assert_eq!(provider_impl.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn webhook_secret_hash_accepts_valid_header() {
    let provider_impl = Arc::new(MockProvider::default());
    let provider: Arc<dyn Provider> = provider_impl.clone();
    let memory: Arc<dyn Memory> = Arc::new(MockMemory);
    let secret = generate_test_secret();

    let state = AppState {
        config: Arc::new(Mutex::new(Config::default())),
        provider,
        model: "test-model".into(),
        temperature: 0.0,
        mem: memory,
        auto_save: false,
        webhook_secret_hash: Some(Arc::from(hash_webhook_secret(&secret))),
        pairing: Arc::new(PairingGuard::new(false, &[])),
        trust_forwarded_headers: false,
        rate_limiter: Arc::new(GatewayRateLimiter::new(100, 100, 100)),
        idempotency_store: Arc::new(IdempotencyStore::new(Duration::from_secs(300), 1000)),
        whatsapp: None,
        whatsapp_app_secret: None,
        linq: None,
        linq_signing_secret: None,
        observer: Arc::new(crate::alphahuman::observability::NoopObserver),
    };

    let mut headers = HeaderMap::new();
    headers.insert("X-Webhook-Secret", HeaderValue::from_str(&secret).unwrap());

    let response = handle_webhook(
        axum::extract::State(state),
        test_connect_info(),
        headers,
        Ok(Json(webhook_body("hello"))),
    )
    .await
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    assert_eq!(provider_impl.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn webhook_missing_message_returns_bad_request() {
    let provider_impl = Arc::new(MockProvider::default());
    let provider: Arc<dyn Provider> = provider_impl.clone();
    let memory: Arc<dyn Memory> = Arc::new(MockMemory);

    let state = AppState {
        config: Arc::new(Mutex::new(Config::default())),
        provider,
        model: "test-model".into(),
        temperature: 0.0,
        mem: memory,
        auto_save: false,
        webhook_secret_hash: None,
        pairing: Arc::new(PairingGuard::new(false, &[])),
        trust_forwarded_headers: false,
        rate_limiter: Arc::new(GatewayRateLimiter::new(100, 100, 100)),
        idempotency_store: Arc::new(IdempotencyStore::new(Duration::from_secs(300), 1000)),
        whatsapp: None,
        whatsapp_app_secret: None,
        linq: None,
        linq_signing_secret: None,
        observer: Arc::new(crate::alphahuman::observability::NoopObserver),
    };

    let response = handle_webhook(
        axum::extract::State(state),
        test_connect_info(),
        HeaderMap::new(),
        Ok(Json(WebhookBody {
            message: None,
            model: None,
            temperature: None,
            memory: None,
        })),
    )
    .await
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    assert_eq!(provider_impl.calls.load(Ordering::SeqCst), 0);
}

// ══════════════════════════════════════════════════════════
// WhatsApp Signature Verification Tests (CWE-345 Prevention)
// ══════════════════════════════════════════════════════════

fn compute_whatsapp_signature_hex(secret: &str, body: &[u8]) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(body);
    hex::encode(mac.finalize().into_bytes())
}

fn compute_whatsapp_signature_header(secret: &str, body: &[u8]) -> String {
    format!("sha256={}", compute_whatsapp_signature_hex(secret, body))
}

#[test]
fn whatsapp_signature_valid() {
    let app_secret = generate_test_secret();
    let body = b"test body content";

    let signature_header = compute_whatsapp_signature_header(&app_secret, body);

    assert!(verify_whatsapp_signature(
        &app_secret,
        body,
        &signature_header
    ));
}

#[test]
fn whatsapp_signature_invalid_wrong_secret() {
    let app_secret = generate_test_secret();
    let wrong_secret = generate_test_secret();
    let body = b"test body content";

    let signature_header = compute_whatsapp_signature_header(&wrong_secret, body);

    assert!(!verify_whatsapp_signature(
        &app_secret,
        body,
        &signature_header
    ));
}

#[test]
fn whatsapp_signature_invalid_wrong_body() {
    let app_secret = generate_test_secret();
    let original_body = b"original body";
    let tampered_body = b"tampered body";

    let signature_header = compute_whatsapp_signature_header(&app_secret, original_body);

    // Verify with tampered body should fail
    assert!(!verify_whatsapp_signature(
        &app_secret,
        tampered_body,
        &signature_header
    ));
}

#[test]
fn whatsapp_signature_missing_prefix() {
    let app_secret = generate_test_secret();
    let body = b"test body";

    // Signature without "sha256=" prefix
    let signature_header = "abc123def456";

    assert!(!verify_whatsapp_signature(
        &app_secret,
        body,
        signature_header
    ));
}

#[test]
fn whatsapp_signature_empty_header() {
    let app_secret = generate_test_secret();
    let body = b"test body";

    assert!(!verify_whatsapp_signature(&app_secret, body, ""));
}

#[test]
fn whatsapp_signature_invalid_hex() {
    let app_secret = generate_test_secret();
    let body = b"test body";

    // Invalid hex characters
    let signature_header = "sha256=not_valid_hex_zzz";

    assert!(!verify_whatsapp_signature(
        &app_secret,
        body,
        signature_header
    ));
}

#[test]
fn whatsapp_signature_empty_body() {
    let app_secret = generate_test_secret();
    let body = b"";

    let signature_header = compute_whatsapp_signature_header(&app_secret, body);

    assert!(verify_whatsapp_signature(
        &app_secret,
        body,
        &signature_header
    ));
}

#[test]
fn whatsapp_signature_unicode_body() {
    let app_secret = generate_test_secret();
    let body = "Hello 🦀 World".as_bytes();

    let signature_header = compute_whatsapp_signature_header(&app_secret, body);

    assert!(verify_whatsapp_signature(
        &app_secret,
        body,
        &signature_header
    ));
}

#[test]
fn whatsapp_signature_json_payload() {
    let app_secret = generate_test_secret();
    let body = br#"{"entry":[{"changes":[{"value":{"messages":[{"from":"1234567890","text":{"body":"Hello"}}]}}]}]}]"#;

    let signature_header = compute_whatsapp_signature_header(&app_secret, body);

    assert!(verify_whatsapp_signature(
        &app_secret,
        body,
        &signature_header
    ));
}

#[test]
fn whatsapp_signature_case_sensitive_prefix() {
    let app_secret = generate_test_secret();
    let body = b"test body";

    let hex_sig = compute_whatsapp_signature_hex(&app_secret, body);

    // Wrong case prefix should fail
    let wrong_prefix = format!("SHA256={hex_sig}");
    assert!(!verify_whatsapp_signature(&app_secret, body, &wrong_prefix));

    // Correct prefix should pass
    let correct_prefix = format!("sha256={hex_sig}");
    assert!(verify_whatsapp_signature(
        &app_secret,
        body,
        &correct_prefix
    ));
}

#[test]
fn whatsapp_signature_truncated_hex() {
    let app_secret = generate_test_secret();
    let body = b"test body";

    let hex_sig = compute_whatsapp_signature_hex(&app_secret, body);
    let truncated = &hex_sig[..32]; // Only half the signature
    let signature_header = format!("sha256={truncated}");

    assert!(!verify_whatsapp_signature(
        &app_secret,
        body,
        &signature_header
    ));
}

#[test]
fn whatsapp_signature_extra_bytes() {
    let app_secret = generate_test_secret();
    let body = b"test body";

    let hex_sig = compute_whatsapp_signature_hex(&app_secret, body);
    let extended = format!("{hex_sig}deadbeef");
    let signature_header = format!("sha256={extended}");

    assert!(!verify_whatsapp_signature(
        &app_secret,
        body,
        &signature_header
    ));
}
