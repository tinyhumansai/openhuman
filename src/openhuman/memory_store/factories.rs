//! # Memory Store Factories
//!
//! Factory functions for creating and initializing various memory store
//! implementations.
//!
//! This module provides a centralized way to instantiate memory stores based on
//! configuration, ensuring that the correct embedding providers and storage
//! backends are used. Currently, it primarily focuses on creating
//! `UnifiedMemory` instances.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::openhuman::config::{EmbeddingRouteConfig, MemoryConfig, StorageProviderConfig};
use crate::openhuman::embeddings::{
    self, format_embedding_signature, EmbeddingProvider, DEFAULT_CLOUD_EMBEDDING_DIMENSIONS,
    DEFAULT_CLOUD_EMBEDDING_MODEL, DEFAULT_OLLAMA_DIMENSIONS, DEFAULT_OLLAMA_MODEL,
};
use crate::openhuman::memory::traits::Memory;
use crate::openhuman::memory_store::unified::UnifiedMemory;

/// One-shot guard so the Ollama health-gate fallback only reports to Sentry
/// once per process lifetime. Memory is constructed many times per session
/// (once per agent in the harness), so an unguarded `report_error` would
/// re-create the per-embed flood the gate exists to suppress — just with a
/// different message. The first failed probe trips this flag; subsequent
/// probes log at debug level and skip the Sentry report.
static OLLAMA_HEALTH_REPORTED: AtomicBool = AtomicBool::new(false);

/// Reports the Ollama-unreachable fallback to Sentry at most once per
/// process. Returns `true` on the firing call, `false` afterwards — callers
/// use the return value only for logging context.
fn report_ollama_health_gate_once(base_url: &str) -> bool {
    if OLLAMA_HEALTH_REPORTED
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        log::debug!(
            "[memory::factory] ollama health-gate fallback already reported this process; suppressing duplicate at {base_url}"
        );
        return false;
    }
    // Tags are indexed and grouped on; keep them low-cardinality and free of
    // credentials. Full URL stays in the message body for diagnostics.
    let host_tag = redact_ollama_host(base_url);
    let message = format!(
        "ollama embeddings opted-in but daemon unreachable at {base_url}; falling back to cloud embeddings for this session"
    );
    // Call report_error_message directly to avoid a redundant format!("{:#}") round-trip
    // that report_error would perform on an already-formatted &str.
    crate::core::observability::report_error_message(
        &message,
        "memory",
        "ollama_health_gate",
        &[("ollama_host", host_tag), ("fallback", "cloud")],
    );
    true
}

/// Resets the once-per-process Sentry latch. Test-only — any test that
/// exercises a fallback path should call this first so it can't be flaked by
/// suite ordering (an earlier test that already tripped the latch).
#[cfg(test)]
fn reset_health_gate_for_test() {
    OLLAMA_HEALTH_REPORTED.store(false, Ordering::Release);
}

/// Effective Ollama base URL.
///
/// Delegates to [`crate::openhuman::inference::local::ollama_base_url`] so the probe
/// always agrees with the rest of the Ollama machinery on the daemon address.
/// If a future change adds another env-var override or shifts precedence, the
/// memory health-gate picks it up automatically.
fn ollama_base_url_for_probe() -> String {
    crate::openhuman::inference::local::ollama_base_url()
}

/// Canonical `(provider, model, dimensions)` tuple used everywhere the
/// health-gate falls back from Ollama → cloud. Centralised so both the async
/// and sync gate sites agree if the cloud defaults ever change.
fn cloud_embedding_fallback() -> (String, String, usize) {
    (
        "cloud".to_string(),
        DEFAULT_CLOUD_EMBEDDING_MODEL.to_string(),
        DEFAULT_CLOUD_EMBEDDING_DIMENSIONS,
    )
}

/// Extracts a low-cardinality `host[:port]` tag from `base_url` for Sentry.
///
/// Sentry tags are indexed and should not carry secrets or per-instance noise:
/// `http://user:token@host:11434/api/tags?key=v` collapses to `host:11434`.
/// Falls back to `"unknown"` if parsing yields an empty string so we never
/// emit an empty tag value.
fn redact_ollama_host(base_url: &str) -> &str {
    let after_scheme = base_url
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(base_url);
    let after_userinfo = after_scheme
        .rsplit_once('@')
        .map_or(after_scheme, |(_, h)| h);
    let host = after_userinfo
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("")
        .trim();
    if host.is_empty() {
        "unknown"
    } else {
        host
    }
}

/// Probe whether an Ollama daemon is reachable at `base_url`.
///
/// Issues a short-timeout `GET <base_url>/api/tags` (the standard Ollama
/// "list models" endpoint) and returns `true` only when it responds with a
/// 2xx status. Transport failures, timeouts, and non-2xx responses all
/// return `false`.
///
/// Kept deliberately small and side-effect-free so it can be called from
/// the memory factory's startup path without pulling in the full
/// `local_ai::service::ollama_admin` machinery.
///
/// Scoped `pub(crate)` to match `local_ai::ollama_base_url`; the only callers
/// are the factory itself and its sibling tests. Stable external API for the
/// health-gate is [`effective_embedding_settings_probed`].
pub(crate) async fn probe_ollama_reachable(base_url: &str) -> bool {
    let url = format!("{}/api/tags", base_url.trim_end_matches('/'));
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            log::debug!(
                "[memory::factory] probe_ollama_reachable: failed to build http client: {e}"
            );
            return false;
        }
    };
    match client.get(&url).send().await {
        Ok(resp) => resp.status().is_success(),
        Err(e) => {
            log::debug!("[memory::factory] probe_ollama_reachable: {url} unreachable: {e}");
            false
        }
    }
}

/// Returns the effective `(provider, model, dimensions)` triple for the
/// embedding backend.
///
/// The user-facing default is `"cloud"` (OpenHuman backend, Voyage-backed) so
/// fresh installs work without a local Ollama daemon. When the user has
/// explicitly opted into local AI for embeddings —
/// [`LocalAiConfig::use_local_for_embeddings`] — we route through the local
/// Ollama embedder regardless of what `memory.embedding_provider` says, since
/// that toggle is a stronger statement of intent than the per-section default.
///
/// Note: this is the *intended* setting. It does not check whether the Ollama
/// daemon is actually running. For the live, health-checked variant that
/// falls back to cloud when Ollama is configured but unreachable, see
/// [`effective_embedding_settings_probed`].
pub fn effective_embedding_settings(
    memory: &MemoryConfig,
    local_embedding_model: Option<&str>,
) -> (String, String, usize) {
    if let Some(raw) = local_embedding_model {
        // Trim once and reuse — the emptiness check and the final model
        // string must agree, otherwise a value like "  bge-m3  " would pass
        // through to Ollama with surrounding whitespace and 404.
        let trimmed = raw.trim();
        let model = if trimmed.is_empty() {
            DEFAULT_OLLAMA_MODEL.to_string()
        } else {
            trimmed.to_string()
        };
        return ("ollama".to_string(), model, DEFAULT_OLLAMA_DIMENSIONS);
    }
    (
        memory.embedding_provider.clone(),
        memory.embedding_model.clone(),
        memory.embedding_dimensions,
    )
}

/// The **active embedding signature** — the canonical key every per-model
/// sidecar read/write is scoped by (#1574).
///
/// Derived from [`effective_embedding_settings`] (the *intended*, non-probed
/// selection) — deliberately **not** [`effective_embedding_settings_probed`].
/// A transient Ollama-down fallback to cloud must never silently redefine the
/// signature: that would re-key every read at a different space and trigger a
/// spurious full re-embed on the next cold-Ollama launch (spec §3 oscillation
/// guard). The string is produced by [`format_embedding_signature`], the same
/// formatter [`EmbeddingProvider::signature`] uses, so a config-derived
/// signature is byte-identical to a live provider's.
pub fn active_embedding_signature(
    memory: &MemoryConfig,
    local_embedding_model: Option<&str>,
) -> String {
    let (provider, model, dims) = effective_embedding_settings(memory, local_embedding_model);
    format_embedding_signature(&provider, &model, dims)
}

/// Async, health-checked variant of [`effective_embedding_settings`].
///
/// If the intended provider is `"ollama"` but the daemon doesn't respond at
/// `<base_url>/api/tags` within a short timeout, this falls back to the cloud
/// embedder and logs a single warning. This avoids the failure mode behind
/// OPENHUMAN-TAURI-B7: a user who's flipped `local_ai.usage.embeddings = true`
/// in Settings but doesn't actually have Ollama running ends up firing one
/// `ollama_embed` Sentry event per embed call (226+ events in a day with zero
/// impacted users — pure noise that drowns out real signals). With this
/// gate, embed calls never even reach `OllamaEmbedding` in that state; the
/// cloud embedder serves the session and the user gets a working app.
///
/// The probe deliberately uses a 2s timeout — long enough to tolerate a
/// briefly-busy daemon, short enough to not block startup if Ollama is
/// genuinely down.
pub async fn effective_embedding_settings_probed(
    memory: &MemoryConfig,
    local_embedding_model: Option<&str>,
) -> (String, String, usize) {
    let intended = effective_embedding_settings(memory, local_embedding_model);
    if intended.0 != "ollama" {
        return intended;
    }
    let base_url = ollama_base_url_for_probe();
    if probe_ollama_reachable(&base_url).await {
        log::debug!(
            "[memory::factory] ollama healthy at {base_url}; using local embeddings (model={}, dims={})",
            intended.1,
            intended.2,
        );
        return intended;
    }
    // Ollama is configured but not reachable. Report once per process at this
    // gate so a genuine misconfiguration still surfaces in Sentry — but no
    // more than once, so re-instantiating memory across agents/sessions
    // doesn't recreate the per-embed flood we're fixing. Then fall back to
    // cloud so the user has a working app.
    log::warn!(
        "[memory::factory] ollama unreachable at {base_url}; falling back to cloud embedder for this session"
    );
    report_ollama_health_gate_once(&base_url);
    cloud_embedding_fallback()
}

/// Returns the effective name of the memory backend being used.
///
/// Currently, this always returns "namespace" as the unified memory system
/// is the standard.
pub fn effective_memory_backend_name(
    _memory_backend: &str,
    _storage_provider: Option<&StorageProviderConfig>,
) -> String {
    "namespace".to_string()
}

/// Create a standard memory instance based on the provided configuration.
pub fn create_memory(
    config: &MemoryConfig,
    workspace_dir: &Path,
) -> anyhow::Result<Box<dyn Memory>> {
    create_memory_full(config, &[], None, None, workspace_dir)
}

/// Create a memory instance honouring the unified per-workload embedding
/// provider.
///
/// `local_embedding_model` is the parsed Ollama model id when
/// `Config::workload_local_model("embeddings")` returned `Some`, otherwise
/// `None`. Used by top-level entry points (agent harness, channels runtime)
/// that have the full `Config` in scope. The local-AI opt-in flips the
/// embedder to Ollama when `Some`.
pub fn create_memory_with_local_ai(
    memory: &MemoryConfig,
    local_embedding_model: Option<&str>,
    embedding_routes: &[EmbeddingRouteConfig],
    storage_provider: Option<&StorageProviderConfig>,
    workspace_dir: &Path,
) -> anyhow::Result<Box<dyn Memory>> {
    create_memory_full(
        memory,
        embedding_routes,
        storage_provider,
        local_embedding_model,
        workspace_dir,
    )
}

/// Synchronous health-check shim around [`probe_ollama_reachable`].
///
/// Production call sites (`create_memory_with_local_ai` and friends) live in
/// sync code that doesn't want to plumb `async` through the whole agent
/// harness builder chain. They always run inside a multi-thread tokio
/// runtime (the core's main runtime), so we can park the worker via
/// [`tokio::task::block_in_place`] and drive the probe future to completion.
///
/// When no tokio runtime is available OR the runtime is single-threaded
/// (current-thread flavour), we skip the probe entirely and assume the
/// daemon is reachable. `block_in_place` panics on a current-thread runtime
/// — see <https://docs.rs/tokio/latest/tokio/task/fn.block_in_place.html> —
/// so probing in that context would crash the caller. Skipping preserves
/// the pre-health-gate behaviour (which is what tests rely on) and is safe
/// because the existing `OllamaEmbedding` error path still surfaces a
/// transport failure if the daemon truly is down.
fn probe_ollama_reachable_blocking(base_url: &str) -> bool {
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        log::debug!(
            "[memory::factory] probe_ollama_reachable_blocking: no tokio runtime in context; skipping probe"
        );
        return true;
    };
    if !matches!(
        handle.runtime_flavor(),
        tokio::runtime::RuntimeFlavor::MultiThread
    ) {
        log::debug!(
            "[memory::factory] probe_ollama_reachable_blocking: runtime is current-thread (block_in_place would panic); skipping probe"
        );
        return true;
    }
    tokio::task::block_in_place(move || handle.block_on(probe_ollama_reachable(base_url)))
}

/// The most comprehensive factory function for creating a memory instance.
///
/// This function resolves the embedding provider — applying the Ollama
/// health-gate when the user has opted into local embeddings — then
/// initializes the provider and creates a `UnifiedMemory` instance.
fn create_memory_full(
    config: &MemoryConfig,
    _embedding_routes: &[EmbeddingRouteConfig],
    _storage_provider: Option<&StorageProviderConfig>,
    local_embedding_model: Option<&str>,
    workspace_dir: &Path,
) -> anyhow::Result<Box<dyn Memory>> {
    // 1. Resolve the intended provider from config.
    let intended = effective_embedding_settings(config, local_embedding_model);
    let local_ai_opt_in = local_embedding_model
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);

    // 2. Health-gate: if the user has opted into Ollama embeddings but the
    //    daemon isn't reachable, fall back to cloud for this session.
    //    Prevents OPENHUMAN-TAURI-B7's 226-event Sentry flood: instead of
    //    one Sentry event per embed attempt, we report once at the gate
    //    (low cardinality, high signal) and serve the session from cloud.
    let gate_triggered;
    let (provider, model, dims) = if intended.0 == "ollama" {
        let base_url = ollama_base_url_for_probe();
        if probe_ollama_reachable_blocking(&base_url) {
            log::debug!(
                "[memory::factory] ollama healthy at {base_url}; using local embeddings (model={}, dims={})",
                intended.1,
                intended.2,
            );
            gate_triggered = false;
            intended
        } else {
            log::warn!(
                "[memory::factory] ollama unreachable at {base_url}; falling back to cloud embedder for this session"
            );
            report_ollama_health_gate_once(&base_url);
            gate_triggered = true;
            cloud_embedding_fallback()
        }
    } else {
        gate_triggered = false;
        intended
    };

    log::debug!(
        "[memory::factory] effective embedding settings: provider={provider} model={model} dims={dims} \
         (local_ai_opt_in={local_ai_opt_in} gate_triggered={gate_triggered})",
    );

    // 3. Create the embedding provider.
    let embedder: Arc<dyn EmbeddingProvider> = Arc::from(
        embeddings::create_embedding_provider(&provider, &model, dims).inspect_err(|err| {
            log::warn!(
                "[memory::factory] create_embedding_provider failed provider={provider} model={model} dims={dims}: {err}",
            );
        })?,
    );

    // 4. Instantiate UnifiedMemory which handles SQLite and vector storage.
    let mem = UnifiedMemory::new(workspace_dir, embedder, config.sqlite_open_timeout_secs)?;
    Ok(Box::new(mem))
}

/// Create a memory instance specifically for migration purposes.
///
/// The unified namespace memory core has a single workspace-scoped
/// store, so migration writes into the same `UnifiedMemory` instance the
/// rest of the app reads from — there is no separate "migration
/// backend". This helper delegates to [`create_memory`] so the
/// migration importer (`migrate_openclaw_memory`) gets a real, writable
/// memory handle and the Apply path can actually run end-to-end.
///
/// Prior to #1440 this function unconditionally bailed with "memory
/// migration is disabled for the unified namespace memory core", which
/// left the OpenClaw importer broken even though the rest of the
/// pipeline (source discovery, dry-run report, backup) worked.
pub fn create_memory_for_migration(
    config: &MemoryConfig,
    workspace_dir: &Path,
) -> anyhow::Result<Box<dyn Memory>> {
    create_memory(config, workspace_dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{routing::get, Json, Router};
    use std::ffi::OsString;
    use std::net::SocketAddr;

    /// RAII helper that swaps `OPENHUMAN_OLLAMA_BASE_URL` to `value` for the
    /// duration of the scope while holding the local-AI domain test mutex.
    /// The previous value (if any) is restored on drop.
    struct EnvGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
        prev: Option<OsString>,
    }

    impl EnvGuard {
        fn set(value: &str) -> Self {
            let lock = crate::openhuman::inference::local::inference_test_guard();
            let prev = std::env::var_os("OPENHUMAN_OLLAMA_BASE_URL");
            // SAFETY: env mutation is wrapped because Rust 2024 marks it
            // unsafe; the call is gated by the local-AI domain mutex so no
            // other local-AI test is observing the env concurrently.
            unsafe {
                std::env::set_var("OPENHUMAN_OLLAMA_BASE_URL", value);
            }
            Self { _lock: lock, prev }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            // SAFETY: same justification as `set` — still under the same lock.
            unsafe {
                match self.prev.take() {
                    Some(v) => std::env::set_var("OPENHUMAN_OLLAMA_BASE_URL", v),
                    None => std::env::remove_var("OPENHUMAN_OLLAMA_BASE_URL"),
                }
            }
        }
    }

    // ── effective_embedding_settings (unprobed selection priority) ────────

    #[test]
    fn embedding_settings_defaults_to_cloud_when_no_local_ai() {
        let mem = MemoryConfig::default();
        let (provider, model, dims) = effective_embedding_settings(&mem, None);
        assert_eq!(
            provider, "cloud",
            "no local-AI config must default to cloud"
        );
        assert!(!model.is_empty(), "cloud model must be non-empty");
        assert!(dims > 0, "cloud dimensions must be positive");
    }

    #[test]
    fn embedding_settings_uses_memory_config_when_local_disabled() {
        let mut mem = MemoryConfig::default();
        mem.embedding_provider = "openai".to_string();
        mem.embedding_model = "text-embedding-3-small".to_string();
        mem.embedding_dimensions = 1536;

        // Local embedding model = None means workload routes to cloud.
        let (provider, model, dims) = effective_embedding_settings(&mem, None);
        assert_eq!(
            provider, "openai",
            "when local embeddings disabled, memory config must be used"
        );
        assert_eq!(model, "text-embedding-3-small");
        assert_eq!(dims, 1536);
    }

    #[test]
    fn embedding_settings_local_overrides_memory_config() {
        // memory.embedding_provider says "cloud" — but a Some(local_model)
        // is the stronger signal and must override it.
        let mem = MemoryConfig::default(); // cloud by default
        let (provider, model, dims) =
            effective_embedding_settings(&mem, Some("nomic-embed-text:latest"));
        assert_eq!(
            provider, "ollama",
            "Some(local_model) must override memory.embedding_provider"
        );
        assert_eq!(model, "nomic-embed-text:latest");
        assert_eq!(
            dims,
            crate::openhuman::embeddings::DEFAULT_OLLAMA_DIMENSIONS,
            "dimensions must default to Ollama default"
        );
    }

    #[test]
    fn embedding_settings_local_with_empty_model_uses_default() {
        // When the user has opted in but the model field is empty/whitespace,
        // the default Ollama model must be used rather than passing "" to Ollama.
        let mem = MemoryConfig::default();
        let (provider, model, dims) = effective_embedding_settings(&mem, Some("   "));
        assert_eq!(provider, "ollama");
        assert_eq!(
            model,
            crate::openhuman::embeddings::DEFAULT_OLLAMA_MODEL,
            "empty model ID must fall back to default Ollama model"
        );
        assert_eq!(
            dims,
            crate::openhuman::embeddings::DEFAULT_OLLAMA_DIMENSIONS
        );
    }

    /// #1574 invariant: a config-derived `active_embedding_signature` MUST be
    /// byte-identical to the live provider's `.signature()` for the same
    /// (provider, model, dims). Drift here silently splits one embedding space
    /// into two — copied/queried vectors would never match.
    #[test]
    fn active_signature_matches_live_provider_signature() {
        for local in [None, Some("nomic-embed-text:latest"), Some("bge-m3")] {
            let mem = MemoryConfig::default();
            let (provider, model, dims) = effective_embedding_settings(&mem, local);
            let live = embeddings::create_embedding_provider(&provider, &model, dims)
                .expect("provider builds for test triple");
            assert_eq!(
                active_embedding_signature(&mem, local),
                live.signature(),
                "config-derived signature must equal live provider signature (local={local:?})"
            );
        }
    }

    #[test]
    fn active_signature_ignores_probe_fallback() {
        // active_embedding_signature keys off the *intended* selection
        // (effective_embedding_settings), NOT the health-checked variant — so
        // a transient Ollama-down fallback can't flip it to cloud. The dim is
        // base/config-dependent (not what this test pins); the provider+model
        // staying the intended ollama/bge-m3 is the probe-stability property.
        let mem = MemoryConfig::default();
        let sig = active_embedding_signature(&mem, Some("bge-m3"));
        assert!(
            sig.starts_with("provider=ollama;model=bge-m3;dims="),
            "intended local selection must survive (no cloud fallback); got {sig}"
        );
        // And it must equal the non-probed settings, formatted identically.
        let (p, m, d) = effective_embedding_settings(&mem, Some("bge-m3"));
        assert_eq!(sig, format_embedding_signature(&p, &m, d));
    }

    #[test]
    fn effective_memory_backend_name_always_returns_namespace() {
        assert_eq!(effective_memory_backend_name("sqlite", None), "namespace");
        assert_eq!(effective_memory_backend_name("anything", None), "namespace");
        assert_eq!(effective_memory_backend_name("", None), "namespace");
    }

    #[test]
    fn create_memory_for_migration_returns_writable_memory_on_unified_core() {
        // Regression for #1440: prior to that PR this factory unconditionally
        // bailed with "memory migration is disabled for the unified namespace
        // memory core", which broke the OpenClaw importer's Apply path even
        // though the dry-run / preview path worked. Now it delegates to
        // `create_memory` so the migration importer gets a real workspace-
        // scoped memory handle. Box<dyn Memory> doesn't impl Debug, so we
        // match instead of unwrap.
        let tmp = tempfile::tempdir().unwrap();
        let cfg = MemoryConfig::default();
        match create_memory_for_migration(&cfg, tmp.path()) {
            Ok(_) => {}
            Err(e) => panic!("expected Ok for unified namespace core, got: {e}"),
        }
    }

    /// Spin up a mock Ollama-shaped server that responds 200 OK on `/api/tags`.
    async fn start_mock_ollama() -> String {
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

    /// The parsed local-embedding model string that
    /// `Config::workload_local_model("embeddings")` would have produced when
    /// the legacy `local_ai.usage.embeddings = true` flag was set. Used so
    /// the existing test scenarios continue to drive the local code path.
    fn local_embedding_for_test() -> &'static str {
        crate::openhuman::embeddings::DEFAULT_OLLAMA_MODEL
    }

    #[tokio::test]
    async fn probe_returns_true_when_ollama_responds_200() {
        let url = start_mock_ollama().await;
        assert!(probe_ollama_reachable(&url).await);
    }

    #[tokio::test]
    async fn probe_returns_false_for_unreachable_host() {
        // Port 1 on loopback is reliably refused.
        assert!(!probe_ollama_reachable("http://127.0.0.1:1").await);
    }

    #[tokio::test]
    async fn probe_returns_false_on_non_2xx() {
        // Mock that responds 500.
        let app = Router::new().route(
            "/api/tags",
            get(|| async { (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "boom") }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let url = format!("http://127.0.0.1:{}", addr.port());
        assert!(!probe_ollama_reachable(&url).await);
    }

    #[tokio::test]
    async fn probed_settings_keep_cloud_when_provider_is_cloud() {
        // No local-AI opt-in → intended provider is cloud, probe is skipped.
        let mem = MemoryConfig::default();
        let (provider, _, _) = effective_embedding_settings_probed(&mem, None).await;
        assert_eq!(provider, "cloud");
    }

    /// Sets `OPENHUMAN_OLLAMA_BASE_URL` to a deliberately unreachable address
    /// under the local-AI domain mutex, then verifies that the probed settings
    /// fall back to cloud when the user has opted into local embeddings.
    #[tokio::test]
    async fn probed_settings_fall_back_to_cloud_when_ollama_unreachable() {
        let _env = EnvGuard::set("http://127.0.0.1:1");
        // Independent of suite ordering: an earlier fallback test must not
        // leave the latch tripped and silently turn this assertion green.
        reset_health_gate_for_test();

        let mem = MemoryConfig::default();

        let (provider, model, dims) =
            effective_embedding_settings_probed(&mem, Some(local_embedding_for_test())).await;

        assert_eq!(
            provider, "cloud",
            "opted-in but unreachable Ollama must fall back to cloud"
        );
        assert_eq!(model, DEFAULT_CLOUD_EMBEDDING_MODEL);
        assert_eq!(dims, DEFAULT_CLOUD_EMBEDDING_DIMENSIONS);
    }

    #[tokio::test]
    async fn probed_settings_keep_ollama_when_daemon_responds() {
        let url = start_mock_ollama().await;
        let _env = EnvGuard::set(&url);

        let mem = MemoryConfig::default();

        let (provider, _model, dims) =
            effective_embedding_settings_probed(&mem, Some(local_embedding_for_test())).await;

        assert_eq!(provider, "ollama", "healthy Ollama must be honoured");
        assert_eq!(dims, DEFAULT_OLLAMA_DIMENSIONS);
    }

    #[test]
    fn redact_ollama_host_strips_scheme_userinfo_path_and_query() {
        // Strips scheme.
        assert_eq!(
            redact_ollama_host("http://localhost:11434"),
            "localhost:11434"
        );
        // Strips userinfo (would be the credential leak vector).
        assert_eq!(
            redact_ollama_host("http://user:secret@10.0.0.1:11434"),
            "10.0.0.1:11434"
        );
        // Strips path / query / fragment.
        assert_eq!(
            redact_ollama_host("https://host:11434/api/tags?key=v#frag"),
            "host:11434"
        );
        // Scheme-less inputs survive (matches `local_ai::ollama_base_url`'s
        // contract: it may or may not prepend `http://`).
        assert_eq!(redact_ollama_host("host:1234"), "host:1234");
        // Empty / malformed inputs fall back to a safe constant.
        assert_eq!(redact_ollama_host(""), "unknown");
    }

    /// First call to `report_ollama_health_gate_once` fires the report;
    /// subsequent calls in the same process must be suppressed. We can't
    /// observe the Sentry side effect directly here, but the boolean return
    /// value is the gate's contract — covers the once-per-process guarantee.
    ///
    /// Acquires the local-AI domain mutex to serialize with `probed_settings_*`
    /// tests that also touch the latch; without that, parallel test execution
    /// can reset the flag between this test's two
    /// `report_ollama_health_gate_once` calls and turn the second one into a
    /// fresh "first", flaking the suppression assertion.
    #[test]
    fn ollama_health_gate_reports_at_most_once_per_process() {
        let _lock = crate::openhuman::inference::local::inference_test_guard();
        reset_health_gate_for_test();

        assert!(
            report_ollama_health_gate_once("http://127.0.0.1:1"),
            "first call must fire the report"
        );
        assert!(
            !report_ollama_health_gate_once("http://127.0.0.1:1"),
            "second call must be suppressed"
        );
        assert!(
            !report_ollama_health_gate_once("http://example.invalid:11434"),
            "different URL also suppressed — gate is process-scoped, not per-URL"
        );
    }
}
