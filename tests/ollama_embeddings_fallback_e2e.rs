//! Integration tests for the Local Ollama embeddings health-gate to cloud
//! fallback (PR #1555).
//!
//! Covers three scenarios exercised via the public API of
//! `openhuman_core::openhuman::memory`:
//!
//! 1. Local embeddings enabled + Ollama unreachable  → falls back to cloud
//!    provider with the correct cloud model dimensions.
//! 2. Local embeddings enabled + Ollama healthy      → stays on local provider.
//! 3. Local embeddings DISABLED                      → cloud settings unchanged
//!    regardless of Ollama state.
//!
//! `probe_ollama_reachable` and the once-per-process health-gate latch are
//! `pub(crate)`-private; the tests drive the observable behaviour through
//! `effective_embedding_settings` (sync, for scenario 3) and
//! `effective_embedding_settings_probed` (async, for scenarios 1–2), both of
//! which are `pub` and re-exported at `openhuman_core::openhuman::memory`.
//!
//! Run with: `cargo test --test ollama_embeddings_fallback_e2e`

use std::net::SocketAddr;
use std::sync::{Mutex, OnceLock};

use axum::{routing::get, Json, Router};

use openhuman_core::openhuman::config::MemoryConfig;
use openhuman_core::openhuman::embeddings::{
    DEFAULT_CLOUD_EMBEDDING_DIMENSIONS, DEFAULT_CLOUD_EMBEDDING_MODEL, DEFAULT_OLLAMA_DIMENSIONS,
    DEFAULT_OLLAMA_MODEL,
};
use openhuman_core::openhuman::memory_store::factories::{
    effective_embedding_settings, effective_embedding_settings_probed,
};

// ── Env isolation ─────────────────────────────────────────────────────────────

/// Serialises all tests in this file: `OPENHUMAN_OLLAMA_BASE_URL` is a
/// process-global env var that the production code reads at call time, so
/// concurrent mutation across tests would produce non-deterministic results.
static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|p| p.into_inner())
}

/// RAII guard: sets `OPENHUMAN_OLLAMA_BASE_URL` while the lock is held and
/// restores (or removes) the original value on drop.
struct OllamaUrlGuard {
    _lock: std::sync::MutexGuard<'static, ()>,
    prev: Option<String>,
}

impl OllamaUrlGuard {
    fn set(url: &str) -> Self {
        let lock = env_lock();
        let prev = std::env::var("OPENHUMAN_OLLAMA_BASE_URL").ok();
        // SAFETY: guarded by ENV_LOCK — no concurrent env mutation in this test binary.
        unsafe { std::env::set_var("OPENHUMAN_OLLAMA_BASE_URL", url) };
        Self { _lock: lock, prev }
    }
}

impl Drop for OllamaUrlGuard {
    fn drop(&mut self) {
        // SAFETY: same guard justification as OllamaUrlGuard::set.
        match self.prev.take() {
            Some(v) => unsafe { std::env::set_var("OPENHUMAN_OLLAMA_BASE_URL", v) },
            None => unsafe { std::env::remove_var("OPENHUMAN_OLLAMA_BASE_URL") },
        }
    }
}

// ── Mock Ollama helper ────────────────────────────────────────────────────────

/// Spawns a minimal Axum server that mimics the Ollama `/api/tags` endpoint
/// (200 OK + JSON body). Returns the base URL, e.g. `"http://127.0.0.1:NNNNN"`.
async fn start_mock_ollama_200() -> String {
    let app = Router::new().route(
        "/api/tags",
        get(|| async { Json(serde_json::json!({ "models": [] })) }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://127.0.0.1:{}", addr.port())
}

// ── Scenario 1: opted-in, Ollama unreachable → cloud fallback ────────────────

/// Port 1 on loopback is always refused on all supported platforms.
const UNREACHABLE_URL: &str = "http://127.0.0.1:1";

/// Scenario 1: local embeddings enabled + Ollama unreachable.
///
/// Verifies:
/// - effective provider flips to `"cloud"`.
/// - cloud model and dimensions match the well-known defaults.
/// - the diagnostic branch is exercised (the gate fires at most once
///   per process, but the fallback outcome is observable every call).
#[tokio::test]
async fn local_embeddings_enabled_ollama_unreachable_falls_back_to_cloud() {
    let _env = OllamaUrlGuard::set(UNREACHABLE_URL);

    let mem = MemoryConfig::default();
    // Pass the default Ollama model name as `local_embedding_model` —
    // same as `Config::workload_local_model("embeddings")` would when the
    // `local_ai.usage.embeddings` flag is set.
    let local_model = DEFAULT_OLLAMA_MODEL;

    let (provider, model, dims) =
        effective_embedding_settings_probed(&mem, Some(local_model)).await;

    assert_eq!(
        provider, "cloud",
        "opted-in local embeddings with unreachable Ollama must fall back to cloud provider"
    );
    assert_eq!(
        model, DEFAULT_CLOUD_EMBEDDING_MODEL,
        "fallback must use the canonical cloud embedding model"
    );
    assert_eq!(
        dims, DEFAULT_CLOUD_EMBEDDING_DIMENSIONS,
        "fallback dimensions must match the canonical cloud embedding dimensions"
    );
}

// ── Scenario 2: opted-in, Ollama healthy → stays on local provider ───────────

/// Scenario 2: local embeddings enabled + Ollama daemon responds 200 OK.
///
/// Verifies:
/// - effective provider remains `"ollama"`.
/// - dimensions are the Ollama default (not the cloud default).
#[tokio::test]
async fn local_embeddings_enabled_ollama_healthy_stays_on_local_provider() {
    let mock_url = start_mock_ollama_200().await;
    let _env = OllamaUrlGuard::set(&mock_url);

    let mem = MemoryConfig::default();
    let local_model = DEFAULT_OLLAMA_MODEL;

    let (provider, model, dims) =
        effective_embedding_settings_probed(&mem, Some(local_model)).await;

    assert_eq!(
        provider, "ollama",
        "healthy Ollama must keep the local provider; got provider={provider} model={model} dims={dims}"
    );
    assert_eq!(
        dims, DEFAULT_OLLAMA_DIMENSIONS,
        "local provider must use Ollama default dimensions, not cloud defaults"
    );
    assert_ne!(
        provider, "cloud",
        "healthy Ollama must not fall back to cloud"
    );
}

// ── Scenario 3: local embeddings DISABLED → cloud unchanged ──────────────────

/// Scenario 3a: no local-AI opt-in → the probed function keeps cloud settings
/// without touching Ollama at all (the probe is skipped when intended provider
/// is already `"cloud"`).
#[tokio::test]
async fn local_embeddings_disabled_probed_keeps_cloud_settings() {
    // We deliberately point the URL at an unreachable host to prove that the
    // probe is never issued on this path — if it were, the test would still
    // pass due to fallback, but using an obviously-bad URL makes the intent
    // explicit: Ollama state is irrelevant when local embeddings are off.
    let _env = OllamaUrlGuard::set(UNREACHABLE_URL);

    let mem = MemoryConfig::default(); // embedding_provider = "cloud" by default
    let (provider, _, _) = effective_embedding_settings_probed(&mem, None).await;

    assert_eq!(
        provider, "cloud",
        "with no local-AI opt-in the probed variant must keep the cloud provider"
    );
}

/// Scenario 3b: synchronous variant — `effective_embedding_settings` (the
/// *intended*, non-probed selection) also keeps the MemoryConfig values when
/// `local_embedding_model` is `None`, regardless of Ollama state.
#[test]
fn local_embeddings_disabled_sync_keeps_memory_config_settings() {
    let mut mem = MemoryConfig::default();
    mem.embedding_provider = "cloud".to_string();
    mem.embedding_model = DEFAULT_CLOUD_EMBEDDING_MODEL.to_string();
    mem.embedding_dimensions = DEFAULT_CLOUD_EMBEDDING_DIMENSIONS;

    // None = local embeddings not opted in.
    let (provider, model, dims) = effective_embedding_settings(&mem, None);

    assert_eq!(
        provider, "cloud",
        "sync selection with no opt-in must honour MemoryConfig.embedding_provider"
    );
    assert_eq!(
        model, DEFAULT_CLOUD_EMBEDDING_MODEL,
        "sync selection must honour MemoryConfig.embedding_model"
    );
    assert_eq!(
        dims, DEFAULT_CLOUD_EMBEDDING_DIMENSIONS,
        "sync selection must honour MemoryConfig.embedding_dimensions"
    );
}

/// Scenario 3c: Ollama health state is irrelevant when local embeddings are
/// disabled — even with a custom `MemoryConfig` that names a cloud-like
/// provider, the output must match the config as-is (no Ollama probe).
#[tokio::test]
async fn local_embeddings_disabled_custom_config_untouched() {
    let _env = OllamaUrlGuard::set(UNREACHABLE_URL);

    let mut mem = MemoryConfig::default();
    mem.embedding_provider = "openai".to_string();
    mem.embedding_model = "text-embedding-3-small".to_string();
    mem.embedding_dimensions = 1536;

    // local_embedding_model = None → probed variant must return the config as-is.
    let (provider, model, dims) = effective_embedding_settings_probed(&mem, None).await;

    assert_eq!(provider, "openai");
    assert_eq!(model, "text-embedding-3-small");
    assert_eq!(
        dims, 1536,
        "custom cloud dimensions must pass through unchanged"
    );
}
