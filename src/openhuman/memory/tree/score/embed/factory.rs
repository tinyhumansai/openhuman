//! Build an [`Embedder`] from [`Config`] settings.
//!
//! Resolution order:
//! 1. **Explicit override** — `memory_tree.embedding_endpoint` +
//!    `memory_tree.embedding_model` both Some → [`OllamaEmbedder`] with
//!    those exact values. For power users / E2E test rigs that want to
//!    point at a non-default Ollama endpoint.
//! 2. **Local-AI usage flag** — `config.local_ai.use_local_for_embeddings()`
//!    (i.e. `runtime_enabled && usage.embeddings`) → [`OllamaEmbedder`]
//!    against [`ollama_base_url`] with the user's chosen
//!    `config.local_ai.embedding_model_id`. This is the path driven by
//!    the "Memory embeddings" checkbox in Local AI Settings.
//! 3. **Default** — [`CloudEmbedder`] (OpenHuman backend / Voyage,
//!    1024 dims). Auth failures surface at the first `embed()` call so
//!    ingest's existing retry-with-backoff logic handles them.
//!
//! NOTE on dimensions: the memory tree on-disk format is hard-coded at
//! [`EMBEDDING_DIM`](super::EMBEDDING_DIM) (1024). If the user picks a
//! local embedding model whose output is a different dimensionality,
//! the trait's post-call validator rejects each embed with a clear
//! `expected N dims, got M` error. Switching the local model picker in
//! Local AI Settings is the fix.
//!
//! The historical `InertEmbedder` (zero vectors) path is retained for
//! tests only — it is no longer the production lax-mode fallback.
//!
//! Env var overrides applied in [`crate::openhuman::config::load`]:
//! - `OPENHUMAN_MEMORY_EMBED_ENDPOINT`
//! - `OPENHUMAN_MEMORY_EMBED_MODEL`
//! - `OPENHUMAN_MEMORY_EMBED_TIMEOUT_MS`

use anyhow::{Context, Result};

use std::time::Duration;

use super::{Embedder, InertEmbedder, ProviderEmbedder, EMBEDDING_DIM};
use crate::openhuman::config::Config;
use crate::openhuman::inference::local::ollama_base_url;
use tinyagents::harness::embeddings::{OllamaEmbeddingModel, RECOMMENDED_OLLAMA_CONTEXT_TOKENS};

/// Cheap heuristic for "is a backend session reachable?" — the cloud
/// embedder needs one and bails on first embed call without it. We use
/// the *presence* of `auth-profiles.json` next to the config file as a
/// proxy: production after login has it, test harnesses and fresh
/// pre-login installs don't. The CloudEmbedder still re-validates the
/// JWT at every embed call, so a stale file just surfaces at embed
/// time (not factory build), preserving the prior failure behavior.
fn cloud_session_available(config: &Config) -> bool {
    config
        .config_path
        .parent()
        .map(|dir| dir.join("auth-profiles.json").exists())
        .unwrap_or(false)
}

/// Construct the active embedder for this process, honouring
/// `config.memory_tree.*` and `embedding_strict`.
///
/// Returns a boxed trait object so ingest / seal can call one code path
/// regardless of which provider is active. The returned box is created
/// per call — cheap because `OllamaEmbedder` owns a cloned `reqwest::Client`
/// internally and `InertEmbedder` is a ZST.
pub fn build_embedder_from_config(config: &Config) -> Result<Box<dyn Embedder>> {
    // Read path: walk the shared ladder, then terminate at InertEmbedder (zero
    // vectors) so retrieval / semantic rerank can still run with no provider.
    Ok(match resolve_embedder_choice(config)? {
        EmbedderChoice::Ollama {
            endpoint,
            model,
            timeout_ms,
        } => {
            log::debug!(
                "[memory_tree::embed::factory] read → Ollama endpoint={endpoint} model={model} timeout_ms={timeout_ms}"
            );
            Box::new(build_ollama_embedder(&endpoint, &model, timeout_ms)?)
        }
        EmbedderChoice::OptOut => {
            log::info!(
                "[memory_tree::embed::factory] embeddings_provider=none — \
                 using InertEmbedder (vector search disabled)"
            );
            Box::new(InertEmbedder::new())
        }
        EmbedderChoice::OpenAiCompat(openai) => {
            log::debug!(
                "[memory_tree::embed::factory] read → user OpenAI-compatible embeddings ({})",
                openai.name()
            );
            Box::new(openai)
        }
        EmbedderChoice::Cloud => {
            log::debug!(
                "[memory_tree::embed::factory] read → cloud (Voyage) — flip \
                 'Memory embeddings' in Local AI Settings to switch to local"
            );
            Box::new(build_cloud_embedder(config))
        }
        EmbedderChoice::NoProvider => {
            log::warn!(
                "[memory_tree::embed::factory] no backend session found — \
                 using InertEmbedder (zero vectors). Log in to OpenHuman, or \
                 enable 'Memory embeddings' in Local AI Settings, to fix."
            );
            Box::new(InertEmbedder::new())
        }
    })
}

/// The embedder the resolution ladder selects, independent of whether the
/// caller is a read path (retrieval) or a write path (ingest/seal). Both
/// public factories walk [`resolve_embedder_choice`] and differ ONLY at the
/// terminal + degraded-flag side-effects — so "identical resolution for every
/// real provider" is a structural guarantee, not two hand-maintained copies
/// that could drift (reviewer sanil-23, #3076: a read/write provider mismatch
/// would silently corrupt recall).
enum EmbedderChoice {
    /// Explicit Ollama override, or the unified `ollama:<model>` workload setting.
    Ollama {
        endpoint: String,
        model: String,
        timeout_ms: u64,
    },
    /// `embeddings_provider = "none"` — vector search off by deliberate user
    /// choice (NOT a degradation). Both paths use `InertEmbedder`.
    OptOut,
    /// User-configured OpenAI / custom OpenAI-compatible endpoint (#002 FR-015).
    OpenAiCompat(super::openai_compat::OpenAiCompatEmbedder),
    /// Logged-in managed cloud (Voyage).
    Cloud,
    /// No usable provider. Read path → `InertEmbedder` (zero vectors); write
    /// path → `None` (skip) + mark `semantic_recall` degraded.
    NoProvider,
}

/// Walk the provider-resolution ladder once. The order is the single source of
/// truth for both factories; the only read/write differences are encoded by the
/// callers at the terminal, never here.
fn resolve_embedder_choice(config: &Config) -> Result<EmbedderChoice> {
    let tree_cfg = &config.memory_tree;

    // 1. Explicit Ollama override (power-user / E2E rig).
    if let (Some(endpoint), Some(model)) = (
        tree_cfg.embedding_endpoint.as_deref(),
        tree_cfg.embedding_model.as_deref(),
    ) {
        if !endpoint.trim().is_empty() && !model.trim().is_empty() {
            return Ok(EmbedderChoice::Ollama {
                endpoint: endpoint.to_string(),
                model: model.to_string(),
                timeout_ms: tree_cfg.embedding_timeout_ms.unwrap_or(0),
            });
        }
    }

    // 2. Deliberate opt-out — vector search off by user choice.
    if config
        .embeddings_provider
        .as_deref()
        .map(|s| s.trim())
        .is_some_and(|s| s == "none")
    {
        return Ok(EmbedderChoice::OptOut);
    }

    // 3. Local Ollama via the unified workload setting.
    if let Some(model) = config.workload_local_model("embeddings") {
        return Ok(EmbedderChoice::Ollama {
            endpoint: ollama_base_url(),
            model,
            timeout_ms: tree_cfg.embedding_timeout_ms.unwrap_or(0),
        });
    }

    // 4. #002 FR-015: user-configured OpenAI / custom OpenAI-compatible.
    if let Some(openai) = super::openai_compat::OpenAiCompatEmbedder::try_from_config(config)? {
        return Ok(EmbedderChoice::OpenAiCompat(openai));
    }

    // 5. Logged-in managed cloud (Voyage).
    if cloud_session_available(config) {
        return Ok(EmbedderChoice::Cloud);
    }

    // 6. Nothing usable.
    Ok(EmbedderChoice::NoProvider)
}

/// Build the embedder used by **write** paths (ingest extract + seal), with an
/// explicit "no usable embedder" signal (#002 FR-002).
///
/// Identical resolution to [`build_embedder_from_config`] for every real
/// provider (explicit Ollama override, local Ollama, cloud session). The one
/// difference is the terminal fallback: where the read-path factory returns an
/// [`InertEmbedder`] (zero vectors) so retrieval can still run, the write path
/// returns **`Ok(None)`** so callers **skip** embedding instead of persisting a
/// fake all-zero vector that would silently poison semantic recall and present
/// a degraded result as success. The chunk/summary is written embedding-less
/// (re-embeddable later once a provider is configured), and the process-global
/// `semantic_recall` degraded flag is set with a typed cause so the status /
/// doctor surface can name the fix.
///
/// `embeddings_provider = "none"` is treated as a deliberate opt-out, not a
/// degradation: it returns the [`InertEmbedder`] (vector search intentionally
/// off) without setting the degraded flag — same as the read path.
pub fn build_write_embedder(config: &Config) -> Result<Option<Box<dyn Embedder>>> {
    use crate::openhuman::memory::tree::health::{
        clear_semantic_recall_degraded, mark_semantic_recall_degraded, FailureCode,
    };

    // Write path: same ladder as the read factory, terminating at `None` (skip,
    // don't persist zero vectors) + a typed degraded flag when no provider is
    // usable. Every real-provider branch clears the flag; the deliberate
    // "none" opt-out leaves it untouched (off by choice, not degradation).
    Ok(match resolve_embedder_choice(config)? {
        EmbedderChoice::Ollama {
            endpoint,
            model,
            timeout_ms,
        } => {
            clear_semantic_recall_degraded();
            Some(Box::new(build_ollama_embedder(
                &endpoint, &model, timeout_ms,
            )?))
        }
        EmbedderChoice::OptOut => {
            clear_semantic_recall_degraded();
            log::info!(
                "[memory_tree::embed::factory] embeddings_provider=none — write path \
                 uses InertEmbedder (vector search disabled by choice)"
            );
            Some(Box::new(InertEmbedder::new()))
        }
        EmbedderChoice::OpenAiCompat(openai) => {
            clear_semantic_recall_degraded();
            Some(Box::new(openai))
        }
        EmbedderChoice::Cloud => {
            clear_semantic_recall_degraded();
            Some(Box::new(build_cloud_embedder(config)))
        }
        EmbedderChoice::NoProvider => {
            log::warn!(
                "[memory_tree::embed::factory] no usable embeddings provider — skipping \
                 embedding (chunk persists embedding-less, re-embeddable later). Set up \
                 local Ollama embeddings or log in to OpenHuman to enable semantic recall."
            );
            mark_semantic_recall_degraded(FailureCode::EmbeddingsUnconfigured);
            None
        }
    })
}

/// Render a ladder-resolution error safely for a log line.
///
/// The only user-controlled values these errors interpolate are the configured
/// provider string and model (see `openai_compat::try_from_config`), and one of
/// them is an endpoint: in its `custom:<url>` form `memory.embedding_provider`
/// *is* a URL, which may carry `user:pass@` userinfo. Configured
/// `cloud_providers` endpoints can reach the message the same way through the
/// underlying constructor's own context.
///
/// So rather than dropping the reason — which would cost the diagnostic that
/// makes this log worth having ("dimension mismatch", "build failed") — replace
/// each known endpoint substring with its [`redact_endpoint`] form. Scrubbing
/// the exact strings we already hold is precise, where a generic URL-matching
/// pass over free text would be guesswork (CodeRabbit, #5402 / CWE-532).
fn redact_ladder_error(config: &Config, err: &anyhow::Error) -> String {
    use crate::openhuman::memory::util::redact::redact_endpoint;

    // Candidates: the inline `custom:<url>` endpoint (when that is the
    // configured form) plus every configured OpenAI-compatible endpoint
    // (LM Studio, vLLM, …), any of which the ladder may have been resolving.
    let mut endpoints: Vec<&str> = config
        .memory
        .embedding_provider
        .trim()
        .strip_prefix("custom:")
        .into_iter()
        .chain(config.cloud_providers.iter().map(|e| e.endpoint.as_str()))
        .map(str::trim)
        .filter(|e| !e.is_empty())
        .collect();

    // Longest first. Substring replacement is order-sensitive: if a short
    // endpoint is a strict prefix of a longer one (`https://host` vs
    // `https://host/v1?key=…`), scrubbing the short one first rewrites the
    // longer one's prefix, so its own replacement no longer matches and the
    // credential-bearing suffix survives in the log (CodeRabbit, #5402).
    endpoints.sort_by_key(|e| std::cmp::Reverse(e.len()));
    endpoints.dedup();

    let mut msg = format!("{err:#}");
    for endpoint in endpoints {
        msg = msg.replace(endpoint, &redact_endpoint(endpoint));
    }
    msg
}

/// Slug naming the embedder ingestion will **actually** use, walking the same
/// [`resolve_embedder_choice`] ladder the read and write factories walk.
///
/// This exists because `config.memory.embedding_provider` is *not* authoritative
/// for how embeddings are funded, and reading it as if it were produces a false
/// alarm. The ladder resolves local Ollama from `memory_tree.embedding_endpoint`
/// or from the unified `workload_local_model("embeddings")` setting (the "Memory
/// embeddings" toggle in Local AI Settings), and **neither path rewrites
/// `memory.embedding_provider`** — so a user running fully local still reads as
/// `"cloud"` there. Any surface that asks "do these embeddings bill against the
/// managed budget?" must ask this function, not that field (reviewer M3gA-Mind,
/// #5402: otherwise a local-embeddings user whose *chat* budget crosses 90% is
/// told memory has stopped growing while it is growing fine).
///
/// Slugs are stable wire values consumed by the frontend:
/// - `"ollama"` — local daemon; user-funded.
/// - `"custom"` — user's own OpenAI-compatible endpoint / key; user-funded.
/// - `"cloud"` — managed OpenHuman backend; **bills the managed cycle budget**.
/// - `"none"` — deliberate opt-out (`embeddings_provider = "none"`).
/// - `"unconfigured"` — no usable provider (signed out); nothing is billed.
/// - `"unknown"` — the ladder itself failed to resolve. Deliberately not
///   `"cloud"`: an unresolvable config must never manufacture a budget warning.
pub fn effective_embedder_slug(config: &Config) -> &'static str {
    let slug = match resolve_embedder_choice(config) {
        Ok(EmbedderChoice::Ollama { .. }) => "ollama",
        Ok(EmbedderChoice::OptOut) => "none",
        Ok(EmbedderChoice::OpenAiCompat(_)) => "custom",
        Ok(EmbedderChoice::Cloud) => "cloud",
        Ok(EmbedderChoice::NoProvider) => "unconfigured",
        Err(err) => {
            log::warn!(
                "[memory_tree::embed::factory] effective_embedder_slug: ladder failed to \
                 resolve ({}) — reporting 'unknown' (treated as NOT managed)",
                redact_ladder_error(config, &err)
            );
            "unknown"
        }
    };
    log::debug!("[memory_tree::embed::factory] effective_embedder_slug → {slug}");
    slug
}

fn build_ollama_embedder(endpoint: &str, model: &str, timeout_ms: u64) -> Result<ProviderEmbedder> {
    let timeout = Duration::from_millis(if timeout_ms == 0 { 10_000 } else { timeout_ms });
    let client = reqwest::Client::builder()
        .connect_timeout(timeout)
        .build()
        .context("build Ollama embeddings HTTP client")?;
    let model = OllamaEmbeddingModel::try_new(endpoint, model, EMBEDDING_DIM)?
        .with_context_options(
            RECOMMENDED_OLLAMA_CONTEXT_TOKENS,
            RECOMMENDED_OLLAMA_CONTEXT_TOKENS,
        )
        .with_client(client);
    Ok(ProviderEmbedder::new(
        crate::openhuman::inference::embeddings::TinyAgentsEmbeddingProvider::boxed(model),
        "ollama",
    ))
}

fn build_cloud_embedder(config: &Config) -> ProviderEmbedder {
    let openhuman_dir = config.config_path.parent().map(std::path::PathBuf::from);
    let provider = crate::openhuman::inference::embeddings::cloud::OpenHumanCloudEmbedding::new(
        None,
        openhuman_dir,
        config.secrets.encrypt,
        crate::openhuman::inference::embeddings::cloud::DEFAULT_CLOUD_EMBEDDING_MODEL,
        crate::openhuman::inference::embeddings::cloud::DEFAULT_CLOUD_EMBEDDING_DIMENSIONS,
    );
    ProviderEmbedder::new(Box::new(provider), "cloud")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_config() -> (TempDir, Config) {
        let tmp = TempDir::new().unwrap();
        let mut cfg = Config::default();
        cfg.workspace_dir = tmp.path().to_path_buf();
        // Plant config_path in the tempdir so cloud_session_available()
        // checks a writable directory; tests that need to simulate a
        // logged-in user just `touch` auth-profiles.json next to it.
        cfg.config_path = tmp.path().join("config.toml");
        (tmp, cfg)
    }

    /// Drop a stub `auth-profiles.json` next to the test config so
    /// `cloud_session_available()` returns true. Contents don't matter
    /// — the factory only checks presence.
    fn touch_auth_profile(cfg: &Config) {
        let path = cfg
            .config_path
            .parent()
            .map(|p| p.join("auth-profiles.json"))
            .expect("config_path has a parent");
        std::fs::write(&path, "{}").expect("write stub auth-profiles.json");
    }

    #[test]
    fn ollama_chosen_when_endpoint_and_model_set() {
        let (_tmp, mut cfg) = test_config();
        cfg.memory_tree.embedding_endpoint = Some("http://localhost:11434".into());
        cfg.memory_tree.embedding_model = Some("bge-m3".into());
        cfg.memory_tree.embedding_timeout_ms = Some(5000);
        let e = build_embedder_from_config(&cfg).expect("Ollama path should build");
        assert_eq!(e.name(), "ollama");
    }

    // ── build_write_embedder (T010, #002 FR-002) ─────────────────────────
    //
    // These assert the write-path factory's "skip vs embed" contract. The
    // degraded flag is a process-global atomic, so the flag-sensitive tests
    // serialize on a shared mutex to avoid stomping each other under cargo's
    // parallel test runner.
    // Delegate to the health module's shared guard so factory tests serialise
    // against the rpc/extract tests that touch the SAME process-global flags
    // (a factory-local mutex would only serialise within this module, leaving
    // a cross-module race). The guard also resets the flags on entry.
    fn degraded_flag_lock() -> std::sync::MutexGuard<'static, ()> {
        crate::openhuman::memory::tree::health::test_guard()
    }

    #[test]
    fn write_embedder_none_when_no_provider_and_marks_degraded() {
        use crate::openhuman::memory::tree::health::{
            clear_semantic_recall_degraded, current_degraded_state, FailureCode,
        };
        let _guard = degraded_flag_lock();
        clear_semantic_recall_degraded();
        let (_tmp, mut cfg) = test_config();
        cfg.memory_tree.embedding_endpoint = None;
        cfg.memory_tree.embedding_model = None;
        // No auth-profiles.json, no local workload model → no usable provider.
        let e = build_write_embedder(&cfg).expect("factory must not error");
        assert!(
            e.is_none(),
            "no provider → skip embedding (None), not inert"
        );
        let d = current_degraded_state();
        assert!(
            d.semantic_recall,
            "semantic recall must be flagged degraded"
        );
        assert_eq!(
            d.cause.map(|c| c.code),
            Some(FailureCode::EmbeddingsUnconfigured)
        );
        clear_semantic_recall_degraded();
    }

    #[test]
    fn write_embedder_some_cloud_with_session_and_clears_degraded() {
        use crate::openhuman::memory::tree::health::{
            current_degraded_state, mark_semantic_recall_degraded, FailureCode,
        };
        let _guard = degraded_flag_lock();
        // Pretend a prior run left recall degraded; a working provider clears it.
        mark_semantic_recall_degraded(FailureCode::EmbeddingsUnconfigured);
        let (_tmp, mut cfg) = test_config();
        cfg.memory_tree.embedding_endpoint = None;
        cfg.memory_tree.embedding_model = None;
        touch_auth_profile(&cfg);
        let e = build_write_embedder(&cfg)
            .expect("factory must not error")
            .expect("cloud session → Some(embedder)");
        assert_eq!(e.name(), "cloud");
        assert!(
            !current_degraded_state().semantic_recall,
            "a usable provider must clear the degraded flag"
        );
    }

    #[test]
    fn write_embedder_some_ollama_override() {
        let (_tmp, mut cfg) = test_config();
        cfg.memory_tree.embedding_endpoint = Some("http://localhost:11434".into());
        cfg.memory_tree.embedding_model = Some("bge-m3".into());
        let e = build_write_embedder(&cfg)
            .expect("factory must not error")
            .expect("override → Some(embedder)");
        assert_eq!(e.name(), "ollama");
    }

    #[test]
    fn write_embedder_none_provider_is_inert_not_skip() {
        use crate::openhuman::memory::tree::health::{
            clear_semantic_recall_degraded, current_degraded_state,
        };
        let _guard = degraded_flag_lock();
        clear_semantic_recall_degraded();
        let (_tmp, mut cfg) = test_config();
        cfg.embeddings_provider = Some("none".into());
        // Deliberate opt-out → InertEmbedder (vector search off by choice),
        // and NOT flagged as a degradation.
        let e = build_write_embedder(&cfg)
            .expect("factory must not error")
            .expect("provider=none → Some(inert), not skip");
        assert_eq!(e.name(), "inert");
        assert!(
            !current_degraded_state().semantic_recall,
            "explicit opt-out is not a degradation"
        );
    }

    #[test]
    fn unset_endpoint_with_session_routes_to_cloud() {
        let (_tmp, mut cfg) = test_config();
        cfg.memory_tree.embedding_endpoint = None;
        cfg.memory_tree.embedding_model = None;
        cfg.memory_tree.embedding_strict = false;
        touch_auth_profile(&cfg);
        let e = build_embedder_from_config(&cfg).expect("cloud default should build");
        assert_eq!(e.name(), "cloud");
    }

    #[test]
    fn unset_endpoint_without_session_falls_back_to_inert() {
        // Test harness / pre-login: no auth-profiles.json on disk,
        // factory degrades to InertEmbedder so callers don't crash on
        // first embed call.
        let (_tmp, mut cfg) = test_config();
        cfg.memory_tree.embedding_endpoint = None;
        cfg.memory_tree.embedding_model = None;
        cfg.memory_tree.embedding_strict = false;
        let e = build_embedder_from_config(&cfg).expect("inert fallback should build");
        assert_eq!(e.name(), "inert");
    }

    #[test]
    fn empty_strings_count_as_unset_with_session() {
        let (_tmp, mut cfg) = test_config();
        cfg.memory_tree.embedding_endpoint = Some("".into());
        cfg.memory_tree.embedding_model = Some("".into());
        cfg.memory_tree.embedding_strict = false;
        touch_auth_profile(&cfg);
        let e = build_embedder_from_config(&cfg).expect("cloud default should build");
        assert_eq!(e.name(), "cloud");
    }

    #[test]
    fn strict_mode_no_longer_bails_with_cloud_default() {
        // Strict mode used to bail when endpoint/model were unset because
        // the only fallback was InertEmbedder. Now the lax-and-strict
        // paths share the cloud fallback; strict bail is a no-op here
        // and auth failures surface at first embed() call instead.
        let (_tmp, mut cfg) = test_config();
        cfg.memory_tree.embedding_endpoint = None;
        cfg.memory_tree.embedding_model = None;
        cfg.memory_tree.embedding_strict = true;
        touch_auth_profile(&cfg);
        let e = build_embedder_from_config(&cfg).expect("cloud default should build");
        assert_eq!(e.name(), "cloud");
    }

    #[test]
    fn local_ai_usage_embeddings_routes_to_ollama() {
        // After #1710 the local-vs-cloud decision for embeddings is
        // driven by `embeddings_provider` (via
        // `Config::workload_uses_local("embeddings")`), not the legacy
        // `local_ai.usage.embeddings` flag. Set the new workload field
        // so the local branch is taken; `embedding_model_id` is still
        // the model name source for the Ollama provider.
        let (_tmp, mut cfg) = test_config();
        cfg.memory_tree.embedding_endpoint = None;
        cfg.memory_tree.embedding_model = None;
        cfg.embeddings_provider = Some("ollama:all-minilm:latest".into());
        cfg.local_ai.runtime_enabled = true;
        cfg.local_ai.embedding_model_id = "all-minilm:latest".to_string();
        let e = build_embedder_from_config(&cfg).expect("ollama path should build");
        assert_eq!(e.name(), "ollama");
    }

    #[test]
    fn local_ai_usage_off_with_session_falls_back_to_cloud() {
        // runtime_enabled=true but usage.embeddings=false → cloud (with session).
        let (_tmp, mut cfg) = test_config();
        cfg.memory_tree.embedding_endpoint = None;
        cfg.memory_tree.embedding_model = None;
        cfg.local_ai.runtime_enabled = true;
        cfg.local_ai.usage.embeddings = false;
        touch_auth_profile(&cfg);
        let e = build_embedder_from_config(&cfg).expect("cloud default should build");
        assert_eq!(e.name(), "cloud");
    }

    #[test]
    fn none_provider_returns_inert() {
        let (_tmp, mut cfg) = test_config();
        cfg.embeddings_provider = Some("none".into());
        touch_auth_profile(&cfg);
        let e = build_embedder_from_config(&cfg).expect("none should build");
        assert_eq!(e.name(), "inert");
    }

    #[test]
    fn write_embedder_routes_to_openai_when_memory_provider_is_openai() {
        // #002 FR-015 regression: the headline bug was that a user-configured
        // OpenAI embeddings provider (`config.memory.embedding_provider =
        // "openai"`) matched no factory branch and silently fell through to the
        // managed-budget backend. Lock the routing in at the FACTORY level —
        // `openai_compat`'s own tests only cover `try_from_config` in isolation,
        // so a factory refactor could re-break this with those tests still green.
        //
        // Note the two distinct config fields the factory reads: the top-level
        // `embeddings_provider` (here unset, so the "none"/`ollama:` branches do
        // not match) vs `memory.embedding_provider` (the unified Embeddings-
        // settings field that drives the OpenAI/custom detection).
        let _guard = degraded_flag_lock();
        use crate::openhuman::memory::tree::health::{
            current_degraded_state, mark_semantic_recall_degraded, FailureCode,
        };
        mark_semantic_recall_degraded(FailureCode::EmbeddingsUnconfigured);
        let (_tmp, mut cfg) = test_config();
        cfg.memory_tree.embedding_endpoint = None;
        cfg.memory_tree.embedding_model = None;
        cfg.embeddings_provider = None; // top-level workload routing: unset
        cfg.memory.embedding_provider = "openai".to_string();
        cfg.memory.embedding_model = "text-embedding-3-large".to_string();
        let e = build_write_embedder(&cfg)
            .expect("factory must not error")
            .expect("openai provider → Some(embedder), must NOT fall through to skip/cloud");
        assert_eq!(
            e.name(),
            "openai",
            "must route to the user's OpenAI embeddings, not the managed backend"
        );
        assert!(
            !current_degraded_state().semantic_recall,
            "a usable OpenAI provider must clear the degraded flag"
        );
    }

    #[test]
    fn write_embedder_routes_to_lmstudio_local_endpoint() {
        // #3781 regression at the factory/seal level: a configured local
        // OpenAI-compatible embeddings backend (LM Studio at localhost:1234,
        // registered as a `cloud_providers` slug) must drive bucket sealing —
        // the same way the LLM extractor already resolves the `lmstudio` slug —
        // and NOT fall through to the managed cloud budget (which 400s with
        // "Insufficient budget" and fails the seal job unrecoverably).
        use crate::openhuman::config::schema::cloud_providers::CloudProviderCreds;
        use crate::openhuman::memory::tree::health::{
            current_degraded_state, mark_semantic_recall_degraded, FailureCode,
        };
        let _guard = degraded_flag_lock();
        mark_semantic_recall_degraded(FailureCode::EmbeddingsUnconfigured);
        let (_tmp, mut cfg) = test_config();
        cfg.memory_tree.embedding_endpoint = None;
        cfg.memory_tree.embedding_model = None;
        cfg.embeddings_provider = None; // top-level workload routing: unset
        cfg.memory.embedding_provider = "lmstudio".to_string();
        cfg.memory.embedding_model = "bge-m3".to_string();
        cfg.cloud_providers = vec![CloudProviderCreds {
            id: "p_lmstudio".to_string(),
            slug: "lmstudio".to_string(),
            endpoint: "http://localhost:1234/v1".to_string(),
            ..Default::default()
        }];
        let e = build_write_embedder(&cfg)
            .expect("factory must not error")
            .expect("lmstudio backend → Some(embedder), must NOT fall through to cloud");
        assert_eq!(
            e.name(),
            "custom",
            "must route to the local OpenAI-compatible endpoint, not the managed backend"
        );
        assert!(
            !current_degraded_state().semantic_recall,
            "a usable local provider must clear the degraded flag"
        );
    }

    #[test]
    fn read_embedder_routes_to_openai_when_memory_provider_is_openai() {
        // Same FR-015 routing, read path (`build_embedder_from_config`).
        let (_tmp, mut cfg) = test_config();
        cfg.memory_tree.embedding_endpoint = None;
        cfg.memory_tree.embedding_model = None;
        cfg.embeddings_provider = None;
        cfg.memory.embedding_provider = "openai".to_string();
        cfg.memory.embedding_model = "text-embedding-3-large".to_string();
        let e = build_embedder_from_config(&cfg).expect("openai path should build");
        assert_eq!(e.name(), "openai");
    }

    #[test]
    fn explicit_endpoint_override_wins_over_local_ai_flag() {
        // Power-user override beats the checkbox.
        let (_tmp, mut cfg) = test_config();
        cfg.memory_tree.embedding_endpoint = Some("http://staging-embed:11434".into());
        cfg.memory_tree.embedding_model = Some("bge-m3".into());
        cfg.local_ai.runtime_enabled = true;
        cfg.local_ai.usage.embeddings = true;
        let e = build_embedder_from_config(&cfg).expect("override path should build");
        assert_eq!(e.name(), "ollama");
    }

    /// The regression this whole helper exists for (reviewer M3gA-Mind, #5402):
    /// a user who enabled local embeddings through Local AI Settings still has
    /// `memory.embedding_provider == "cloud"` (nothing rewrites it), so any
    /// surface reading that field concludes they bill against the managed
    /// budget and warns them their memory has stopped growing — while it is
    /// growing fine, fully locally, costing them nothing.
    #[test]
    fn effective_slug_reports_ollama_when_local_ai_overrides_cloud_setting() {
        let (_tmp, mut cfg) = test_config();
        cfg.memory.embedding_provider = "cloud".to_string();
        cfg.embeddings_provider = Some("ollama:all-minilm:latest".into());
        cfg.local_ai.runtime_enabled = true;
        cfg.local_ai.embedding_model_id = "all-minilm:latest".to_string();
        touch_auth_profile(&cfg);

        // The stale per-section field still says cloud …
        assert_eq!(cfg.memory.embedding_provider, "cloud");
        // … but the ladder — and therefore the wire field — says local.
        assert_eq!(effective_embedder_slug(&cfg), "ollama");
    }

    #[test]
    fn effective_slug_reports_ollama_for_explicit_endpoint_override() {
        let (_tmp, mut cfg) = test_config();
        cfg.memory.embedding_provider = "cloud".to_string();
        cfg.memory_tree.embedding_endpoint = Some("http://localhost:11434".into());
        cfg.memory_tree.embedding_model = Some("bge-m3".into());
        touch_auth_profile(&cfg);
        assert_eq!(effective_embedder_slug(&cfg), "ollama");
    }

    #[test]
    fn effective_slug_reports_cloud_only_for_a_real_managed_session() {
        let (_tmp, mut cfg) = test_config();
        cfg.memory.embedding_provider = "cloud".to_string();
        touch_auth_profile(&cfg);
        assert_eq!(effective_embedder_slug(&cfg), "cloud");
    }

    #[test]
    fn effective_slug_reports_unconfigured_without_a_session() {
        // No auth-profiles.json → nothing is billed, so this must not read as
        // managed even though the per-section field defaults to cloud.
        let (_tmp, mut cfg) = test_config();
        cfg.memory.embedding_provider = "cloud".to_string();
        assert_eq!(effective_embedder_slug(&cfg), "unconfigured");
    }

    #[test]
    fn effective_slug_reports_none_for_deliberate_opt_out() {
        let (_tmp, mut cfg) = test_config();
        cfg.embeddings_provider = Some("none".into());
        touch_auth_profile(&cfg);
        assert_eq!(effective_embedder_slug(&cfg), "none");
    }

    /// The ladder error quotes `memory.embedding_provider` verbatim, and in the
    /// `custom:<url>` form that string is a full endpoint URL — potentially with
    /// `user:pass@` userinfo. Logging it raw would write credentials to disk
    /// (CodeRabbit, #5402 / CWE-532). Scrub the endpoint, keep the reason.
    #[test]
    fn ladder_error_log_redacts_custom_endpoint_credentials() {
        let (_tmp, mut cfg) = test_config();
        // No model + a non-tree dimension → `try_from_config` bails, and its
        // message interpolates the provider string.
        cfg.memory.embedding_provider = "custom:https://user:pass@embed.example.com/v1".to_string();
        cfg.memory.embedding_model = String::new();
        cfg.memory.embedding_dimensions = 512;

        // `EmbedderChoice` is not `Debug` (it holds a live embedder), so unwrap
        // the error by hand rather than via `expect_err`.
        let err = match resolve_embedder_choice(&cfg) {
            Err(e) => e,
            Ok(_) => panic!("a non-tree dimension with no model must fail to resolve"),
        };
        let raw = format!("{err:#}");
        assert!(
            raw.contains("user:pass"),
            "precondition: the unredacted error really does carry the credentials — \
             otherwise this test proves nothing. Got: {raw}"
        );

        let rendered = redact_ladder_error(&cfg, &err);
        assert!(
            !rendered.contains("user:pass"),
            "userinfo must not reach the log: {rendered}"
        );
        assert!(
            !rendered.contains("/v1"),
            "path must not reach the log: {rendered}"
        );
        assert!(
            rendered.contains("embed.example.com"),
            "host is kept so the line stays diagnosable: {rendered}"
        );
        assert!(
            rendered.contains("1024"),
            "the failure reason must survive redaction: {rendered}"
        );

        // And the caller degrades to not-managed rather than to `cloud`.
        assert_eq!(effective_embedder_slug(&cfg), "unknown");
    }

    /// Substring replacement is order-sensitive. With a short endpoint that is a
    /// strict prefix of the long one, scrubbing shortest-first rewrites the long
    /// endpoint's prefix, its own replacement then fails to match, and the
    /// credential-bearing suffix survives in the log. Longest-first is the fix
    /// (CodeRabbit, #5402).
    #[test]
    fn ladder_error_redaction_handles_prefix_overlapping_endpoints() {
        use crate::openhuman::config::schema::cloud_providers::CloudProviderCreds;
        let (_tmp, mut cfg) = test_config();
        // The SHORT endpoint is the one the old code scrubbed first (the inline
        // `custom:` form led the list), and it is a strict prefix of the long
        // one. That ordering is what let the long endpoint's secret survive:
        // scrubbing `https://embed.example.com` first rewrote the long string's
        // prefix, so the long string's own replacement no longer matched.
        cfg.memory.embedding_provider = "custom:https://embed.example.com".to_string();
        cfg.cloud_providers = vec![CloudProviderCreds {
            id: "p_long".to_string(),
            slug: "longpfx".to_string(),
            endpoint: "https://embed.example.com/v1?key=super-secret".to_string(),
            ..Default::default()
        }];

        // Synthesize the error rather than driving the ladder: this pins the
        // redaction function's ordering contract for ANY message carrying both
        // endpoints, which is the property at risk. Which ladder branch happens
        // to surface a `cloud_providers` endpoint today is beside the point.
        let err = anyhow::anyhow!(
            "build custom embedder failed (provider='custom:https://embed.example.com', \
             endpoint='https://embed.example.com/v1?key=super-secret')"
        );
        let rendered = redact_ladder_error(&cfg, &err);

        assert!(
            !rendered.contains("super-secret"),
            "the long endpoint's query must not survive the short endpoint's scrub: {rendered}"
        );
        assert!(
            !rendered.contains("/v1"),
            "the long endpoint's path must not survive either: {rendered}"
        );
        assert!(
            rendered.contains("embed.example.com"),
            "host is still kept: {rendered}"
        );
    }

    #[test]
    fn effective_slug_reports_custom_for_byo_openai_compatible() {
        use crate::openhuman::config::schema::cloud_providers::CloudProviderCreds;
        let (_tmp, mut cfg) = test_config();
        cfg.embeddings_provider = None;
        cfg.memory.embedding_provider = "lmstudio".to_string();
        cfg.memory.embedding_model = "bge-m3".to_string();
        cfg.cloud_providers = vec![CloudProviderCreds {
            id: "p_lmstudio".to_string(),
            slug: "lmstudio".to_string(),
            endpoint: "http://localhost:1234/v1".to_string(),
            ..Default::default()
        }];
        touch_auth_profile(&cfg);
        assert_eq!(effective_embedder_slug(&cfg), "custom");
    }
}
