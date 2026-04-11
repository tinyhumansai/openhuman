//! Local-vs-remote provider resolver for triage turns.
//!
//! ## What this does
//!
//! [`resolve_provider`] picks one of two paths for the next triage turn:
//!
//! - **Local** — wrap the bundled `LocalAiService` (Ollama) in a tiny
//!   [`Provider`]-trait adapter so the existing `agent.run_turn` bus
//!   dispatch flows through unchanged. Used when all of the following
//!   hold:
//!     1. `config.local_ai.enabled == true`
//!     2. Current [`ModelTier`] from config is at or above
//!        `Ram4To8Gb` — the decision locked in during planning (see
//!        `linear-bouncing-lovelace.md`). Tiny 1B models are permitted
//!        because the tolerant parser in `decision.rs` handles their
//!        malformed JSON and the evaluator retries on remote if the
//!        reply can't be parsed.
//!     3. `LocalAiService::status().state == "ready"` — the service
//!        singleton has finished bootstrap and is not installing /
//!        downloading / degraded.
//! - **Remote** — the existing `OpenHumanBackendProvider` via
//!   `create_routed_provider_with_options`, same as commit 1.
//!
//! ## Caching
//!
//! The decision is cached in a process-wide `tokio::sync::Mutex<
//! Option<CachedDecision>>` with a 60 s TTL. The first call in a window
//! pays the config-load cost; subsequent calls reuse. A `Degraded` state
//! (set by [`mark_degraded`]) forces remote for the rest of the window
//! so a flaky local adapter can't thrash.
//!
//! ## No live network probe
//!
//! The plan floated a ~500 ms Ollama `/api/generate` probe inside the
//! availability check. We deliberately skip that here because
//! (a) the `LocalAiService` bootstrap state is already a coarse
//! liveness signal, and (b) if the actual inference fails mid-turn the
//! evaluator's retry-on-remote path covers the gap. That keeps
//! `resolve_provider` free of hidden I/O and unit tests of the state
//! machine don't need to stub a network transport.

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context;
use async_trait::async_trait;
use tokio::sync::Mutex;

use crate::openhuman::config::Config;
use crate::openhuman::local_ai::{
    self,
    presets::{current_tier_from_config, ModelTier},
    LocalAiService,
};
use crate::openhuman::providers::{self, Provider, ProviderRuntimeOptions, INFERENCE_BACKEND_ID};

/// The concrete provider + metadata that [`crate::openhuman::agent::triage::evaluator::run_triage`]
/// should use for this particular triage turn.
pub struct ResolvedProvider {
    /// Ready-to-use provider, already constructed.
    pub provider: Arc<dyn Provider>,
    /// Provider name token — `"openhuman"` for the remote backend and
    /// `"local-ollama"` for the local path. Passed through unchanged
    /// into `AgentTurnRequest::provider_name`.
    pub provider_name: String,
    /// Model identifier — the concrete string `run_tool_call_loop`
    /// will hand to the provider.
    pub model: String,
    /// `true` if this turn is running on the local LLM. Published in
    /// `DomainEvent::TriggerEvaluated.used_local` for observability.
    pub used_local: bool,
}

// ── Cache state machine ─────────────────────────────────────────────────

/// Decision stored in the 60 s cache.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CacheState {
    /// Next turn runs on the local adapter.
    Local,
    /// Next turn runs on the remote backend.
    Remote,
    /// A previous local turn failed — force remote for the rest of the
    /// TTL so we don't thrash when Ollama is flaky.
    Degraded,
}

#[derive(Clone, Copy, Debug)]
struct CachedDecision {
    at: Instant,
    state: CacheState,
}

const CACHE_TTL: Duration = Duration::from_secs(60);

/// Process-wide decision cache. `tokio::sync::Mutex::const_new` allows a
/// `static` without a `Lazy`/`OnceLock` wrapper.
static DECISION_CACHE: Mutex<Option<CachedDecision>> = Mutex::const_new(None);

// ── Public API ──────────────────────────────────────────────────────────

/// Resolve a provider for a single triage turn. Consults the cache,
/// constructs the chosen provider, and returns a fully-formed
/// [`ResolvedProvider`].
pub async fn resolve_provider() -> anyhow::Result<ResolvedProvider> {
    let config = Config::load_or_init()
        .await
        .context("loading config for triage provider resolution")?;
    resolve_provider_with_config(&config).await
}

/// Inner half of [`resolve_provider`] that takes an already-loaded
/// [`Config`]. Exposed so the evaluator's "retry on remote after local
/// failure" path can reuse the same config it already loaded for the
/// first attempt.
pub async fn resolve_provider_with_config(config: &Config) -> anyhow::Result<ResolvedProvider> {
    let state = decide_with_cache(config).await;
    tracing::debug!(
        ?state,
        local_enabled = config.local_ai.enabled,
        "[triage::routing] resolving provider"
    );
    match state {
        CacheState::Local => match build_local_provider(config) {
            Ok(resolved) => Ok(resolved),
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    "[triage::routing] local provider build failed — falling back to remote"
                );
                mark_degraded_internal().await;
                build_remote_provider(config)
            }
        },
        CacheState::Remote | CacheState::Degraded => build_remote_provider(config),
    }
}

/// Force the next turn onto remote regardless of the cached decision.
/// Called by the evaluator after a local turn fails so subsequent
/// triggers in the same TTL window don't re-hit the broken local path.
pub async fn mark_degraded() {
    mark_degraded_internal().await;
}

/// Snapshot of the cache for ops debugging / tests. Returns the raw
/// decision + how much of the TTL is left in milliseconds.
#[derive(Debug, Clone, Copy)]
pub struct CacheSnapshot {
    pub state: &'static str,
    pub ttl_remaining_ms: u128,
}

pub async fn cache_snapshot() -> Option<CacheSnapshot> {
    let cache = DECISION_CACHE.lock().await;
    cache.map(|c| CacheSnapshot {
        state: match c.state {
            CacheState::Local => "local",
            CacheState::Remote => "remote",
            CacheState::Degraded => "degraded",
        },
        ttl_remaining_ms: CACHE_TTL.saturating_sub(c.at.elapsed()).as_millis(),
    })
}

// ── Cache helpers ───────────────────────────────────────────────────────

async fn mark_degraded_internal() {
    let mut cache = DECISION_CACHE.lock().await;
    *cache = Some(CachedDecision {
        at: Instant::now(),
        state: CacheState::Degraded,
    });
}

async fn decide_with_cache(config: &Config) -> CacheState {
    {
        let cache = DECISION_CACHE.lock().await;
        if let Some(cached) = *cache {
            if cached.at.elapsed() < CACHE_TTL {
                return cached.state;
            }
        }
    }
    let state = decide_fresh(config);
    let mut cache = DECISION_CACHE.lock().await;
    *cache = Some(CachedDecision {
        at: Instant::now(),
        state,
    });
    state
}

/// Pure decision function — no I/O beyond `LocalAiService::status()`
/// (which is a `parking_lot::Mutex::lock().clone()` on a cached struct,
/// so effectively free). Cheap enough to call on every cache refresh.
fn decide_fresh(config: &Config) -> CacheState {
    if !config.local_ai.enabled {
        return CacheState::Remote;
    }
    let tier = current_tier_from_config(&config.local_ai);
    if tier_score(tier) < tier_score(ModelTier::Ram4To8Gb) {
        tracing::debug!(?tier, "[triage::routing] tier below floor — forcing remote");
        return CacheState::Remote;
    }
    let service = local_ai::global(config);
    let state = service.status().state;
    if state != "ready" {
        tracing::debug!(
            service_state = %state,
            "[triage::routing] LocalAiService not ready — forcing remote"
        );
        return CacheState::Remote;
    }
    CacheState::Local
}

fn tier_score(tier: ModelTier) -> u8 {
    match tier {
        ModelTier::Ram1Gb => 1,
        ModelTier::Ram2To4Gb => 2,
        ModelTier::Ram4To8Gb => 3,
        ModelTier::Ram8To16Gb => 4,
        ModelTier::Ram16PlusGb => 5,
        // Custom means the user wired their own model IDs explicitly
        // and should be trusted — same priority as the highest preset.
        ModelTier::Custom => 5,
    }
}

// ── Provider builders ───────────────────────────────────────────────────

/// Build the default remote routed backend provider. Same wiring as
/// `local_ai::ops::agent_chat_simple` uses so we stay consistent with
/// the existing direct-chat path.
fn build_remote_provider(config: &Config) -> anyhow::Result<ResolvedProvider> {
    let default_model = config
        .default_model
        .clone()
        .unwrap_or_else(|| crate::openhuman::config::DEFAULT_MODEL.to_string());
    let options = ProviderRuntimeOptions {
        auth_profile_override: None,
        openhuman_dir: config.config_path.parent().map(std::path::PathBuf::from),
        secrets_encrypt: config.secrets.encrypt,
        reasoning_enabled: config.runtime.reasoning_enabled,
    };
    let provider_box = providers::create_routed_provider_with_options(
        config.api_key.as_deref(),
        config.api_url.as_deref(),
        &config.reliability,
        &config.model_routes,
        default_model.as_str(),
        &options,
    )
    .context("building routed remote provider for triage")?;
    // `Box<dyn Provider>` → `Arc<dyn Provider>` is a single reallocation
    // — the `Provider` trait is `Send + Sync` so this is type-safe.
    let provider: Arc<dyn Provider> = Arc::from(provider_box);
    tracing::debug!(
        provider = %INFERENCE_BACKEND_ID,
        model = %default_model,
        "[triage::routing] resolved remote provider"
    );
    Ok(ResolvedProvider {
        provider,
        provider_name: INFERENCE_BACKEND_ID.to_string(),
        model: default_model,
        used_local: false,
    })
}

/// Build a [`Provider`]-trait adapter over the bundled
/// [`LocalAiService`] so local triage turns can flow through the same
/// `agent.run_turn` bus dispatch the remote path uses. Constructed
/// fresh per turn — the underlying `LocalAiService` is an `Arc`
/// singleton so the wrapper itself is essentially free.
fn build_local_provider(config: &Config) -> anyhow::Result<ResolvedProvider> {
    let service = local_ai::global(config);
    let model = crate::openhuman::local_ai::model_ids::effective_chat_model_id(config);
    let adapter = LocalAiAdapter {
        service,
        config: config.clone(),
    };
    tracing::debug!(
        model = %model,
        "[triage::routing] resolved local provider"
    );
    Ok(ResolvedProvider {
        provider: Arc::new(adapter),
        provider_name: "local-ollama".to_string(),
        model,
        used_local: true,
    })
}

/// Minimal [`Provider`] implementation over [`LocalAiService`].
///
/// We only implement the single required method — `chat_with_system` —
/// and let the trait's default impls handle `chat_with_history`,
/// `chat`, `simple_chat`, etc. For triage the path reduces to:
///
/// ```text
/// agent.run_turn → run_tool_call_loop → (no tools) → chat_with_history
///                → (default) → chat_with_system → LocalAiService::inference_with_temperature
/// ```
///
/// The triage agent has `named = []`, so the tool path is never taken.
struct LocalAiAdapter {
    service: Arc<LocalAiService>,
    /// Owned snapshot — we need it for every `inference_with_temperature`
    /// call because that API takes `&Config` for model-id resolution.
    config: Config,
}

#[async_trait]
impl Provider for LocalAiAdapter {
    async fn chat_with_system(
        &self,
        system_prompt: Option<&str>,
        message: &str,
        _model: &str,
        temperature: f64,
    ) -> anyhow::Result<String> {
        // `_model` is ignored — `LocalAiService` resolves its own chat
        // model via `effective_chat_model_id(config)`. If we ever want
        // per-call model override we'll thread it through a tweaked
        // config clone, but the triage path only cares about the
        // configured default anyway.
        let system = system_prompt.unwrap_or("You are a helpful assistant.");
        // Clamp temperature to f32 — LocalAiService API uses f32 whereas
        // the Provider trait uses f64.
        let temp = temperature as f32;
        // Small max_tokens is fine: triage replies are a few dozen
        // tokens at most (short JSON decision block).
        self.service
            .inference_with_temperature(&self.config, system, message, Some(512), true, temp)
            .await
            .map_err(|e| anyhow::anyhow!("local AI inference failed: {e}"))
    }
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Reset the cache between tests so they don't observe each
    /// other's state. Called at the top of every cache-state test.
    async fn clear_cache() {
        let mut cache = DECISION_CACHE.lock().await;
        *cache = None;
    }

    #[test]
    fn tier_score_orders_ascending_by_capability() {
        assert!(tier_score(ModelTier::Ram1Gb) < tier_score(ModelTier::Ram2To4Gb));
        assert!(tier_score(ModelTier::Ram2To4Gb) < tier_score(ModelTier::Ram4To8Gb));
        assert!(tier_score(ModelTier::Ram4To8Gb) < tier_score(ModelTier::Ram8To16Gb));
        assert!(tier_score(ModelTier::Ram8To16Gb) < tier_score(ModelTier::Ram16PlusGb));
        assert_eq!(
            tier_score(ModelTier::Custom),
            tier_score(ModelTier::Ram16PlusGb)
        );
    }

    #[test]
    fn tier_score_floor_is_ram_4_to_8_gb() {
        // Anything below the floor must be rejected.
        let floor = tier_score(ModelTier::Ram4To8Gb);
        assert!(tier_score(ModelTier::Ram1Gb) < floor);
        assert!(tier_score(ModelTier::Ram2To4Gb) < floor);
        // And anything at or above must pass.
        assert!(tier_score(ModelTier::Ram4To8Gb) >= floor);
        assert!(tier_score(ModelTier::Ram8To16Gb) >= floor);
        assert!(tier_score(ModelTier::Ram16PlusGb) >= floor);
        assert!(tier_score(ModelTier::Custom) >= floor);
    }

    fn test_config() -> Config {
        Config::default()
    }

    #[test]
    fn decide_fresh_returns_remote_when_local_disabled() {
        let mut config = test_config();
        config.local_ai.enabled = false;
        assert_eq!(decide_fresh(&config), CacheState::Remote);
    }

    #[tokio::test]
    async fn mark_degraded_forces_remote_on_next_resolve() {
        clear_cache().await;
        mark_degraded().await;
        let snap = cache_snapshot()
            .await
            .expect("cache seeded by mark_degraded");
        assert_eq!(snap.state, "degraded");
        assert!(snap.ttl_remaining_ms > 0);
    }

    #[tokio::test]
    async fn cache_snapshot_returns_none_for_empty_cache() {
        clear_cache().await;
        assert!(cache_snapshot().await.is_none());
    }

    #[tokio::test]
    async fn decide_with_cache_respects_ttl_window() {
        clear_cache().await;
        // Prime the cache manually so we don't need to stub config IO.
        {
            let mut guard = DECISION_CACHE.lock().await;
            *guard = Some(CachedDecision {
                at: Instant::now(),
                state: CacheState::Degraded,
            });
        }
        // Within TTL, decide_with_cache should return the cached state
        // without recomputing. Since the cached state is `Degraded` and
        // default config would normally pick `Remote`, the fact that we
        // observe `Degraded` proves the cache was hit.
        let state = decide_with_cache(&test_config()).await;
        assert_eq!(state, CacheState::Degraded);
    }
}
