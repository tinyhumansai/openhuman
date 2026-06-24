//! Memory-tree [`Embedder`] backed by a user-configured OpenAI-compatible
//! embeddings provider (#002 FR-015).
//!
//! ## Why this exists
//!
//! The memory-tree embedder factory historically resolved only: explicit
//! Ollama override → `ollama:` workload prefix → managed `CloudEmbedder`
//! (backend→Voyage) → skip. So a user who configured **OpenAI** (or any
//! custom OpenAI-compatible endpoint) in Settings → AI → Embeddings was
//! silently ignored: their `embeddings_provider = "openai"` matched no branch
//! and fell through to the managed backend, which then hit "managed budget"
//! while the user's own key sat unused. This adapter closes that gap.
//!
//! ## How
//!
//! It wraps the unified [`EmbeddingProvider`] built by
//! [`create_embedding_provider_with_credentials`] (the same construction the
//! Settings "Test connection" + main embed RPC use, so there is one source of
//! truth for OpenAI/custom embeddings) and adapts it to the memory-tree
//! [`Embedder`] trait. Dimensions are pinned to [`EMBEDDING_DIM`] (1024) — the
//! tree's on-disk format is fixed there — and the OpenAI request path now
//! sends the `dimensions` parameter (see `embeddings::openai`) so a reducible
//! model (`text-embedding-3-large`) returns 1024 instead of its native 3072.
//! A returned vector of the wrong size surfaces as the trait's standard
//! "expected N dims" error, which the worker classifies as
//! `embedding_dim_mismatch`.

use anyhow::{Context, Result};
use async_trait::async_trait;

use super::{Embedder, EMBEDDING_DIM};
use crate::openhuman::config::Config;
use crate::openhuman::embeddings::EmbeddingProvider;

/// Adapter from the unified [`EmbeddingProvider`] to the memory-tree
/// [`Embedder`] trait for the OpenAI / custom-OpenAI providers.
pub struct OpenAiCompatEmbedder {
    inner: Box<dyn EmbeddingProvider>,
    /// Short label for logs (e.g. "openai", "custom").
    label: &'static str,
}

impl OpenAiCompatEmbedder {
    /// Try to build the adapter from the user's configured embeddings settings.
    ///
    /// Returns `Ok(None)` when `config.memory.embedding_provider` is **not** an
    /// OpenAI-compatible provider (so the caller's resolution chain continues
    /// to the next branch), and `Ok(Some(_))` when it is. Errors only on an
    /// actual construction failure (which the caller can treat as
    /// fail-fast-worthy).
    ///
    /// Always requests [`EMBEDDING_DIM`] regardless of the user's configured
    /// dimensions — the tree format is fixed at 1024, and the OpenAI path now
    /// honours the `dimensions` param so 3-large complies.
    pub fn try_from_config(config: &Config) -> Result<Option<Self>> {
        let provider = config.memory.embedding_provider.trim();

        // Decide which OpenAI-compatible endpoint to route to, if any:
        //   * `openai`             → OpenAI's hosted API.
        //   * `custom` / `custom:` → an inline custom endpoint.
        //   * any configured `cloud_providers` slug (e.g. `lmstudio`, `vllm`)
        //     → that entry's endpoint, treated as a custom OpenAI-compatible
        //     server.
        //
        // The third case is the #3781 fix. The chat/LLM factory already
        // resolves these slugs via `config.cloud_providers`
        // (`make_cloud_provider_by_slug`), so the memory_tree LLM extractor
        // honours a local `lmstudio` backend. The embedder, however, only knew
        // `openai`/`custom` — so a local LM Studio embeddings backend
        // (`[memory] embedding_provider = "lmstudio"`) was silently ignored and
        // bucket sealing fell through to the managed cloud budget, 400ing with
        // "Insufficient budget" and failing jobs as unrecoverable. Mirroring the
        // chat factory's slug resolution here gives sealing/ingest embeddings
        // the same local-endpoint parity the extractor already has.
        //
        // Anything else returns `Ok(None)` so the caller's resolution ladder
        // continues to the managed cloud default.
        let (slug, label, custom_endpoint): (&str, &'static str, Option<&str>) =
            if provider == "openai" {
                ("openai", "openai", None)
            } else if provider == "custom" || provider.starts_with("custom:") {
                ("custom", "custom", provider.strip_prefix("custom:"))
            } else {
                // Bare slug, tolerating a trailing `:model` for symmetry with the
                // top-level `embeddings_provider = "slug:model"` form.
                let bare = provider.split(':').next().unwrap_or(provider).trim();
                // Reserved / managed / native-API slugs are owned by other
                // branches of the resolution ladder (managed cloud, Voyage,
                // Cohere, native Ollama, deliberate opt-out) — never the
                // OpenAI-compatible adapter. Let the caller fall through.
                if bare.is_empty()
                    || matches!(
                        bare,
                        "managed" | "cloud" | "openhuman" | "voyage" | "cohere" | "ollama" | "none"
                    )
                {
                    return Ok(None);
                }
                match config
                    .cloud_providers
                    .iter()
                    .find(|e| e.slug == bare)
                    .map(|e| e.endpoint.trim())
                    .filter(|ep| !ep.is_empty())
                {
                    // A configured OpenAI-compatible provider (LM Studio, vLLM,
                    // text-embeddings-inference, …) → route as `custom` against
                    // its endpoint.
                    Some(endpoint) => ("custom", "custom", Some(endpoint)),
                    // Unknown slug, or one with no endpoint configured — fall
                    // through to the managed cloud default rather than erroring.
                    None => return Ok(None),
                }
            };

        // Credential lookup keys on the bare slug (`embeddings:<slug>`); local
        // servers like LM Studio usually need no key, so an empty result is fine
        // and matches the existing `custom` behaviour. `resolve_api_key` already
        // normalises a `custom:<url>` argument down to the `custom` slug.
        let cred_slug = provider.split(':').next().unwrap_or(provider).trim();
        let api_key = crate::openhuman::embeddings::resolve_api_key(config, cred_slug);

        // Model: prefer the explicit `embedding_model`; otherwise fall back to an
        // inline `slug:model` suffix on the provider string. The `custom:<url>`
        // form is exempt — its suffix is an endpoint URL, not a model name, so
        // splitting it would mis-route the URL as the model. Leave the model
        // empty in that case and let the endpoint default apply.
        let model = {
            let explicit = config.memory.embedding_model.trim();
            if !explicit.is_empty() {
                explicit
            } else if provider.starts_with("custom:") {
                ""
            } else {
                provider
                    .split_once(':')
                    .map(|(_, m)| m.trim())
                    .unwrap_or("")
            }
        };

        let inner = crate::openhuman::embeddings::create_embedding_provider_with_credentials(
            slug,
            model,
            EMBEDDING_DIM,
            &api_key,
            custom_endpoint,
        )
        .with_context(|| {
            format!("build {label} embedder for memory tree (provider='{provider}')")
        })?;

        log::debug!(
            "[memory_tree::embed::openai_compat] using {label} provider (config='{}') \
             endpoint={:?} model={} dims={}",
            provider,
            custom_endpoint,
            model,
            EMBEDDING_DIM
        );
        Ok(Some(Self { inner, label }))
    }
}

#[async_trait]
impl Embedder for OpenAiCompatEmbedder {
    fn name(&self) -> &'static str {
        self.label
    }

    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let v = self
            .inner
            .embed_one(text)
            .await
            .with_context(|| format!("{} embeddings failed", self.label))?;
        if v.len() != EMBEDDING_DIM {
            anyhow::bail!(
                "{} embedder returned {} dims, expected {}",
                self.label,
                v.len(),
                EMBEDDING_DIM
            );
        }
        Ok(v)
    }

    /// Collapse N per-text round-trips into a single batched request by
    /// delegating to the inner provider's native batch `embed`. Falls back to
    /// per-text embedding (preserving per-position error attribution) on a
    /// whole-batch failure or a length mismatch.
    async fn embed_batch(&self, texts: &[&str]) -> Vec<Result<Vec<f32>>> {
        super::embed_batch_via_provider(self.inner.as_ref(), self.label, texts).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn cfg_with_provider(p: &str) -> (TempDir, Config) {
        let tmp = TempDir::new().unwrap();
        let mut cfg = Config::default();
        cfg.workspace_dir = tmp.path().to_path_buf();
        cfg.config_path = tmp.path().join("config.toml");
        cfg.memory.embedding_provider = p.to_string();
        cfg.memory.embedding_model = "text-embedding-3-large".to_string();
        (tmp, cfg)
    }

    #[test]
    fn none_for_non_openai_providers() {
        // managed / voyage / ollama / none must fall through (Ok(None)).
        for p in ["managed", "cloud", "voyage", "ollama:bge-m3", "none"] {
            let (_tmp, cfg) = cfg_with_provider(p);
            let got = OpenAiCompatEmbedder::try_from_config(&cfg).expect("no error");
            assert!(got.is_none(), "{p} should fall through, got Some");
        }
    }

    #[test]
    fn some_for_openai() {
        let (_tmp, cfg) = cfg_with_provider("openai");
        let got = OpenAiCompatEmbedder::try_from_config(&cfg).expect("no error");
        let e = got.expect("openai should build an adapter");
        assert_eq!(e.name(), "openai");
    }

    #[test]
    fn some_for_custom() {
        let (_tmp, cfg) = cfg_with_provider("custom:https://embed.example/v1");
        let got = OpenAiCompatEmbedder::try_from_config(&cfg).expect("no error");
        let e = got.expect("custom should build an adapter");
        assert_eq!(e.name(), "custom");
    }

    /// When `embedding_model` is unset, a `custom:<url>` provider must NOT treat
    /// the endpoint URL suffix as an inline model name (CodeRabbit #3781). The
    /// adapter still builds; the model is simply left empty.
    #[test]
    fn some_for_custom_endpoint_does_not_use_url_as_model() {
        let (_tmp, mut cfg) = cfg_with_provider("custom:https://embed.example/v1");
        cfg.memory.embedding_model = String::new(); // force the inline fallback path
        let got = OpenAiCompatEmbedder::try_from_config(&cfg).expect("no error");
        let e = got.expect("custom endpoint with no model should still build");
        assert_eq!(e.name(), "custom");
    }

    /// Build a `cloud_providers` entry the way AI Settings persists a local
    /// OpenAI-compatible server.
    fn lmstudio_entry(
        endpoint: &str,
    ) -> crate::openhuman::config::schema::cloud_providers::CloudProviderCreds {
        crate::openhuman::config::schema::cloud_providers::CloudProviderCreds {
            id: "p_lmstudio_test".to_string(),
            slug: "lmstudio".to_string(),
            endpoint: endpoint.to_string(),
            ..Default::default()
        }
    }

    /// #3781: a configured `lmstudio` slug (OpenAI-compatible, like LM Studio at
    /// localhost:1234) must resolve to its `cloud_providers` endpoint and route
    /// as a `custom` OpenAI-compatible embedder — NOT fall through to managed
    /// cloud. This is the headline bug: sealing ignored the local backend.
    #[test]
    fn some_for_configured_lmstudio_slug() {
        let (_tmp, mut cfg) = cfg_with_provider("lmstudio");
        cfg.memory.embedding_model = "bge-m3".to_string();
        cfg.cloud_providers = vec![lmstudio_entry("http://localhost:1234/v1")];

        let got = OpenAiCompatEmbedder::try_from_config(&cfg).expect("no error");
        let e = got.expect("configured lmstudio slug must build an adapter, not fall through");
        assert_eq!(e.name(), "custom");
    }

    /// The `slug:model` form (mirroring the top-level
    /// `embeddings_provider = "lmstudio:bge-m3"` shape) also resolves, taking the
    /// model from the inline suffix when `embedding_model` is unset.
    #[test]
    fn some_for_lmstudio_slug_with_inline_model() {
        let (_tmp, mut cfg) = cfg_with_provider("lmstudio:bge-m3");
        cfg.memory.embedding_model = String::new(); // force inline-suffix fallback
        cfg.cloud_providers = vec![lmstudio_entry("http://localhost:1234/v1")];

        let got = OpenAiCompatEmbedder::try_from_config(&cfg).expect("no error");
        let e = got.expect("lmstudio:model slug must resolve");
        assert_eq!(e.name(), "custom");
    }

    /// A custom slug with no matching `cloud_providers` entry must fall through
    /// (Ok(None)) so the caller's ladder continues to the managed default —
    /// rather than erroring or hijacking the resolution.
    #[test]
    fn none_for_unconfigured_custom_slug() {
        let (_tmp, cfg) = cfg_with_provider("lmstudio"); // no cloud_providers entry
        let got = OpenAiCompatEmbedder::try_from_config(&cfg).expect("no error");
        assert!(
            got.is_none(),
            "unconfigured slug should fall through, not build an adapter"
        );
    }

    /// An entry that exists but has a blank endpoint is unusable → fall through.
    #[test]
    fn none_for_configured_slug_with_blank_endpoint() {
        let (_tmp, mut cfg) = cfg_with_provider("lmstudio");
        cfg.cloud_providers = vec![lmstudio_entry("   ")];
        let got = OpenAiCompatEmbedder::try_from_config(&cfg).expect("no error");
        assert!(got.is_none(), "blank endpoint should fall through");
    }

    /// Reserved/managed/native slugs must keep falling through even if a stray
    /// `cloud_providers` entry exists for them — they are owned by other ladder
    /// branches, not the OpenAI-compatible adapter.
    #[test]
    fn reserved_slugs_still_fall_through() {
        use crate::openhuman::config::schema::cloud_providers::CloudProviderCreds;
        for p in ["managed", "cloud", "voyage", "cohere", "ollama", "none"] {
            let (_tmp, mut cfg) = cfg_with_provider(p);
            cfg.cloud_providers = vec![CloudProviderCreds {
                id: format!("p_{p}"),
                slug: p.to_string(),
                endpoint: "http://localhost:1234/v1".to_string(),
                ..Default::default()
            }];
            let got = OpenAiCompatEmbedder::try_from_config(&cfg).expect("no error");
            assert!(got.is_none(), "{p} must fall through, got Some");
        }
    }
}
