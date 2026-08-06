//! Resolved model / voice IDs from [`crate::openhuman::config::Config`].
//!
//! Most `effective_*` functions enforce the MVP model allowlist: if a resolved
//! model ID is not in the allowlist the function silently falls back to the
//! default MVP model and logs a warning. `effective_chat_model_id` and
//! `effective_embedding_model_id` intentionally bypass that allowlist for LM
//! Studio so user-managed model IDs (e.g. an LM-Studio-served
//! `text-embedding-bge-m3`) are passed through unchanged; the generic
//! `effective_*` helpers still enforce the MVP tier restriction for
//! OpenHuman-managed Ollama assets.

use crate::openhuman::config::Config;
use crate::openhuman::inference::local::provider::{provider_from_config, LocalAiProvider};
use crate::openhuman::inference::vision_models::{self, VISION_MODEL_SUGGESTIONS};

pub(crate) const DEFAULT_OLLAMA_MODEL: &str = "gemma3:1b-it-qat";

/// The pinned Moondream build that the `moondream` / `moondream:1.8b`
/// shorthands resolve to, and the low-RAM tier's bundled vision model.
///
/// Must name a genuinely vision-capable model: it is what an alias rewrite
/// lands on, and what the "for example …" suggestions point users at.
///
/// Moondream is the smallest vision model pullable with no extra setup
/// (~1.7 GB across model + projector layers), which keeps it affordable on the
/// low-RAM tiers where vision is most likely to be enabled on demand.
///
/// There is deliberately no `DEFAULT_OLLAMA_VISION_MODEL` any more (#5146 P1).
/// It existed only as the substitute for a chat-only `vision_model_id`, and
/// that substitution is exactly the bug: the user's explicit choice was
/// overridden, this model was auto-pulled behind their back, and the request
/// then failed with `ollama vision returned empty content`. A misconfigured
/// vision model is now an actionable error, so there is nothing left to
/// default *to*.
pub(crate) const DEFAULT_LOW_VISION_MODEL: &str = "moondream:1.8b-v2-q4_K_S";
pub(crate) const DEFAULT_OLLAMA_EMBED_MODEL: &str = "bge-m3";

/// Chat models allowed in the current local Ollama build.
/// Any resolved chat model ID not listed here is redirected to `MVP_DEFAULT_CHAT_MODEL`.
///
/// Every id here must be pullable from the public Ollama library as written —
/// an entry that does not resolve makes the allowlist silently redirect the
/// user back to the default, or leaves them with a model that `ollama pull`
/// cannot fetch (GH #5055).
///
/// This list must also cover every `chat_model_id` in
/// [`crate::openhuman::inference::presets`]: a preset whose model is missing
/// here is silently downgraded to `MVP_DEFAULT_CHAT_MODEL`, so the user picks
/// a tier and quietly gets the 1B model.
/// `preset_chat_models_are_allowlisted_and_resolve_unchanged` pins that
/// invariant.
///
/// Verified against the live registry (#5146 §1.3):
/// `GET https://registry.ollama.ai/v2/library/<name>/manifests/<tag>` returns
/// `200` for all five entries. Note that `gemma4` **does** now exist on the
/// Ollama library (it did not when #5055 removed it) and is multimodal at
/// every size, which is why the 16 GB+ tier can use one model for chat and
/// vision. `gemma3n:e4b-it-q8_0` stays allowlisted for back-compat with users
/// who already pulled it under the previous default.
const MVP_ALLOWED_CHAT_MODELS: &[&str] = &[
    "gemma3:270m-it-qat",
    "gemma3:1b-it-qat",
    "gemma3:4b-it-qat",
    "gemma4:e4b-it-q8_0",
    "gemma3n:e4b-it-q8_0",
];
const MVP_DEFAULT_CHAT_MODEL: &str = "gemma3:1b-it-qat";

/// Embedding models allowed in MVP (2–4 GB tier uses all-minilm).
// bge-m3 (1024-dim, 8192-token context) is the canonical local embedder
// for memory tree's fixed on-disk format. all-minilm (384-dim) is kept
// for back-compat with users who pulled it under an older default, but
// new selections should default to bge-m3.
const MVP_ALLOWED_EMBEDDING_MODELS: &[&str] = &["bge-m3", "all-minilm:latest"];

fn enforce_mvp_chat_allowlist(resolved: &str) -> String {
    let lower = resolved.to_ascii_lowercase();
    for allowed in MVP_ALLOWED_CHAT_MODELS {
        if lower == allowed.to_ascii_lowercase() {
            return resolved.to_string();
        }
    }
    tracing::warn!(
        resolved,
        fallback = MVP_DEFAULT_CHAT_MODEL,
        "[local_ai] chat model not in MVP allowlist, redirecting to default"
    );
    MVP_DEFAULT_CHAT_MODEL.to_string()
}

/// Guarantee a vision request never reaches a chat-only model: `Ok(id)` when
/// `resolved` accepts image input, `Err(actionable message)` when it does not.
///
/// The tier restriction is enforced upstream by
/// [`crate::openhuman::inference::presets::vision_mode_for_config`], which
/// reports `VisionMode::Disabled` for the tiers that ship no vision model. What
/// is left for this function is the capability question alone.
///
/// # Why this errors instead of substituting (#5146 P1)
///
/// It used to swap in the default vision model and return that, which produced
/// the worst failure in the whole vision path: a user who set a chat-only
/// `vision_model_id` got a *different* model silently selected, that model
/// auto-pulled (~1.7 GB with no visible progress), and then — since the
/// substitute answers many prompt phrasings with an empty string — the cryptic
/// `ollama vision returned empty content`. Three surprises deep, none of them
/// naming the actual mistake.
///
/// Substituting is the wrong shape regardless of which default is chosen: the
/// user made an explicit choice and it was silently overridden, the same class
/// of bug as a silent provider switch (#5146 §2.1). Say what is wrong and let
/// them fix it. The message deliberately mirrors the tinyagents Ollama
/// embeddings adapter, which names the offending model and a concrete next step
/// in one line.
///
/// An earlier incarnation of this guard was an allowlist
/// (`MVP_ALLOWED_VISION_MODELS = &[""]`) that matched only the empty string and
/// so rewrote *every* configured vision model to `""`, including capable ones —
/// which is how the nameless `POST /api/pull` in `ensure_ollama_model_available`
/// came about. Both that bug and its replacement failed the same way: they
/// answered "which model?" with something the user never asked for.
fn enforce_vision_capability(resolved: &str) -> Result<String, String> {
    if vision_models::is_vision_capable(resolved) {
        return Ok(resolved.to_string());
    }
    tracing::warn!(
        resolved,
        "[local_ai] configured vision model is chat-only; refusing to substitute"
    );
    let suggestions = VISION_MODEL_SUGGESTIONS.join("`, `");
    Err(format!(
        "the selected vision model `{resolved}` is not vision-capable — it cannot accept image \
         input. Set `local_ai.vision_model_id` to a vision-capable model (for example \
         `{suggestions}`) and pull it with `ollama pull <model>`, or route the vision workload \
         to a cloud provider with `vision_provider`."
    ))
}

fn enforce_mvp_embedding_allowlist(resolved: &str) -> String {
    let lower = resolved.to_ascii_lowercase();
    for allowed in MVP_ALLOWED_EMBEDDING_MODELS {
        if lower == allowed.to_ascii_lowercase() {
            return resolved.to_string();
        }
    }
    tracing::warn!(
        resolved,
        fallback = MVP_ALLOWED_EMBEDDING_MODELS[0],
        "[local_ai] embedding model not in MVP allowlist, redirecting to default"
    );
    MVP_ALLOWED_EMBEDDING_MODELS[0].to_string()
}

pub(crate) fn effective_chat_model_id(config: &Config) -> String {
    let provider = provider_from_config(config);
    if provider == LocalAiProvider::LmStudio {
        let model_id = raw_chat_model_id(config);
        tracing::debug!(
            provider = provider.as_str(),
            has_model = !model_id.is_empty(),
            "[local_ai] effective_chat_model_id: using provider-managed model id"
        );
        return model_id;
    }

    let raw = if !config.local_ai.chat_model_id.trim().is_empty() {
        config.local_ai.chat_model_id.trim()
    } else {
        config.local_ai.model_id.trim()
    };
    if raw.is_empty() {
        return enforce_mvp_chat_allowlist(DEFAULT_OLLAMA_MODEL);
    }
    let lower = raw.to_ascii_lowercase();
    if lower.ends_with(".gguf")
        || lower.contains("huggingface.co/")
        || lower == "qwen3-1.7b"
        || lower == "qwen2.5-1.5b-instruct"
    {
        return enforce_mvp_chat_allowlist(DEFAULT_OLLAMA_MODEL);
    }
    enforce_mvp_chat_allowlist(raw)
}

fn raw_chat_model_id(config: &Config) -> String {
    // For LM Studio the user must set `local_ai.chat_model_id` explicitly —
    // there is no sensible Ollama-branded default to fall back to. Return an
    // empty string so callers (diagnostics, status) surface the missing-model
    // warning rather than silently requesting "gemma3:1b-it-qat" from LM Studio.
    let raw = if !config.local_ai.chat_model_id.trim().is_empty() {
        config.local_ai.chat_model_id.trim()
    } else {
        config.local_ai.model_id.trim()
    };
    if raw.is_empty() {
        tracing::debug!(
            provider = "lm_studio",
            "[local_ai] raw_chat_model_id: no LM Studio chat model configured"
        );
    }
    raw.to_string()
}

/// Apply the alias rewrite that maps a family name onto the pinned tag we
/// actually ship (`moondream` -> `moondream:1.8b-v2-q4_K_S`).
///
/// This is *not* a substitution: it resolves to the same model the user asked
/// for, so it never needs to be reported to them.
fn apply_vision_alias(raw: &str) -> &str {
    let lower = raw.to_ascii_lowercase();
    if lower == "moondream:1.8b" || lower == "moondream" {
        DEFAULT_LOW_VISION_MODEL
    } else {
        raw
    }
}

/// Resolve the vision model for status / reporting surfaces.
///
/// An empty return means "there is no **usable** vision model" — either none is
/// configured (a legitimate state; the low tiers ship no vision model) or the
/// configured one cannot accept images. A non-empty return is always a
/// vision-capable id.
///
/// Since #5146 P1 this no longer substitutes a default for a chat-only
/// configured model. That matters beyond reporting: several callers feed this
/// straight into `ensure_ollama_model_available`, so a substituted id here was
/// how a model the user never chose got auto-pulled. Returning empty keeps the
/// pull paths off a model nobody asked for, and
/// `ensure_ollama_model_available` rejects a blank id outright rather than
/// pulling a nameless model.
///
/// Call [`resolve_vision_model_id`] instead when about to issue an actual
/// vision request — it distinguishes "not configured" from "not vision-capable"
/// and returns an actionable message for each.
///
/// The capability predicate is consulted directly here rather than by calling
/// [`enforce_vision_capability`] and discarding its `Err`: that helper emits a
/// `tracing::warn!` and formats the full suggestion message, and this resolver
/// feeds polled status/diagnostics surfaces. Routing through it would log a
/// warning and burn a `format!` on *every poll* for anyone with a misconfigured
/// `vision_model_id`. The warning belongs at request time, where it is
/// actionable; `effective_and_resolved_vision_ids_agree_on_usability` keeps the
/// two paths pinned to the same verdict.
pub(crate) fn effective_vision_model_id(config: &Config) -> String {
    let raw = config.local_ai.vision_model_id.trim();
    if raw.is_empty() {
        return String::new();
    }
    let resolved = apply_vision_alias(raw);
    if vision_models::is_vision_capable(resolved) {
        resolved.to_string()
    } else {
        String::new()
    }
}

/// Resolve the vision model for a real vision request.
///
/// Never returns an empty id, and never silently swaps the user's choice. The
/// two failure modes get distinct, actionable messages (#5146 §Part 1, P1):
///
/// - **nothing configured** — say what to set and which models to pull;
/// - **configured but chat-only** — name the offending model, because "pull
///   `moondream:…`" is a non-sequitur to someone who configured `gemma3:1b`.
pub(crate) fn resolve_vision_model_id(config: &Config) -> Result<String, String> {
    let raw = config.local_ai.vision_model_id.trim();
    if raw.is_empty() {
        let suggestions = VISION_MODEL_SUGGESTIONS.join("`, `");
        tracing::warn!("[local_ai] vision request with no vision model configured");
        return Err(format!(
            "no local vision model is configured. Set `local_ai.vision_model_id` to a \
             vision-capable model (for example `{suggestions}`) and pull it with \
             `ollama pull <model>`, or route the vision workload to a cloud provider \
             with `vision_provider`."
        ));
    }
    enforce_vision_capability(apply_vision_alias(raw))
}

pub(crate) fn effective_embedding_model_id(config: &Config) -> String {
    let raw = config.local_ai.embedding_model_id.trim();

    // LM Studio serves embeddings under user-managed names (e.g.
    // `text-embedding-bge-m3`) that are deliberately outside the
    // OpenHuman-managed Ollama MVP allowlist. Mirror `effective_chat_model_id`
    // and pass a configured id through unchanged so the user can target the
    // exact served model instead of having it rewritten back to `bge-m3`
    // (#3920). The allowlist remains in force for the managed Ollama path
    // below, where the ids are OpenHuman-pulled assets.
    if provider_from_config(config) == LocalAiProvider::LmStudio {
        if raw.is_empty() {
            // No configured id — fall back to the canonical default so the
            // memory tree still has an embedder to request, rather than
            // sending an empty model name to the LM Studio server.
            tracing::debug!(
                provider = LocalAiProvider::LmStudio.as_str(),
                "[local_ai] effective_embedding_model_id: no LM Studio embedding model configured, using default"
            );
            return DEFAULT_OLLAMA_EMBED_MODEL.to_string();
        }
        tracing::debug!(
            provider = LocalAiProvider::LmStudio.as_str(),
            "[local_ai] effective_embedding_model_id: using provider-managed embedding id"
        );
        return raw.to_string();
    }

    if raw.is_empty() {
        return enforce_mvp_embedding_allowlist(DEFAULT_OLLAMA_EMBED_MODEL);
    }
    enforce_mvp_embedding_allowlist(raw)
}

pub(crate) fn effective_stt_model_id(config: &Config) -> String {
    let raw = config.local_ai.stt_model_id.trim();
    if raw.is_empty() {
        "ggml-base-q5_1.bin".to_string()
    } else {
        raw.to_string()
    }
}

pub(crate) fn effective_tts_voice_id(config: &Config) -> String {
    let raw = config.local_ai.tts_voice_id.trim();
    if raw.is_empty() {
        "en_US-lessac-medium".to_string()
    } else {
        raw.to_string()
    }
}

pub(crate) fn effective_quantization(config: &Config) -> String {
    let raw = config.local_ai.quantization.trim();
    if raw.is_empty() {
        "q4".to_string()
    } else {
        raw.to_ascii_lowercase()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> Config {
        Config::default()
    }

    #[test]
    fn chat_model_falls_back_for_empty_and_unsupported_ids() {
        let mut config = test_config();

        config.local_ai.chat_model_id = String::new();
        config.local_ai.model_id = String::new();
        assert_eq!(effective_chat_model_id(&config), MVP_DEFAULT_CHAT_MODEL);

        config.local_ai.chat_model_id = "custom.gguf".to_string();
        assert_eq!(effective_chat_model_id(&config), MVP_DEFAULT_CHAT_MODEL);

        config.local_ai.chat_model_id = "qwen3-1.7b".to_string();
        assert_eq!(effective_chat_model_id(&config), MVP_DEFAULT_CHAT_MODEL);
    }

    #[test]
    fn chat_model_allows_mvp_model() {
        let mut config = test_config();
        config.local_ai.chat_model_id = "gemma3:1b-it-qat".to_string();
        assert_eq!(effective_chat_model_id(&config), "gemma3:1b-it-qat");
    }

    #[test]
    fn chat_model_allows_requested_ollama_gemma3n_q8() {
        let mut config = test_config();
        config.local_ai.chat_model_id = "gemma3n:e4b-it-q8_0".to_string();
        assert_eq!(effective_chat_model_id(&config), "gemma3n:e4b-it-q8_0");
    }

    #[test]
    fn chat_model_allows_custom_ids_for_lm_studio() {
        let mut config = test_config();
        config.local_ai.provider = "lm_studio".to_string();
        config.local_ai.chat_model_id = "publisher/custom-model-7b".to_string();
        assert_eq!(
            effective_chat_model_id(&config),
            "publisher/custom-model-7b"
        );
    }

    #[test]
    fn lm_studio_chat_model_returns_empty_when_no_model_configured() {
        // LM Studio has no sensible Ollama-branded default — an empty model ID
        // surfaces the missing-model warning in diagnostics / status rather than
        // silently sending "gemma3:1b-it-qat" to an LM Studio server.
        let mut config = test_config();
        config.local_ai.provider = "lm_studio".to_string();
        config.local_ai.chat_model_id = String::new();
        config.local_ai.model_id = String::new();
        assert_eq!(effective_chat_model_id(&config), "");
    }

    #[test]
    fn chat_model_rejects_non_mvp_models() {
        let mut config = test_config();

        // Bare `gemma3n:e4b` is a real Ollama tag but is NOT the allowlisted
        // quantization, so it still redirects to the default.
        config.local_ai.chat_model_id = "gemma3n:e4b".to_string();
        assert_eq!(effective_chat_model_id(&config), MVP_DEFAULT_CHAT_MODEL);

        // Arbitrary non-preset models stay rejected.
        config.local_ai.chat_model_id = "llama3.1:8b".to_string();
        assert_eq!(effective_chat_model_id(&config), MVP_DEFAULT_CHAT_MODEL);

        config.local_ai.chat_model_id = "totally-made-up-model:v0".to_string();
        assert_eq!(effective_chat_model_id(&config), MVP_DEFAULT_CHAT_MODEL);
    }

    /// #5146 §1.3: the allowlist must cover every preset chat model.
    ///
    /// `gemma3:270m-it-qat` (1 GB tier) and `gemma3:4b-it-qat` (8-16 GB tier)
    /// were previously absent, so applying either preset resolved straight
    /// back to the 1B default — the user picked a tier and silently got a
    /// different model than the one the preset advertised.
    #[test]
    fn preset_chat_models_are_allowlisted_and_resolve_unchanged() {
        let mut config = test_config();
        for preset in crate::openhuman::inference::presets::all_presets() {
            config.local_ai.chat_model_id = preset.chat_model_id.to_string();
            assert_eq!(
                effective_chat_model_id(&config),
                preset.chat_model_id,
                "preset {:?} chat model `{}` is not allowlisted and was redirected",
                preset.tier,
                preset.chat_model_id
            );
        }
    }

    /// GH #5055 / #5146 §1.3: every allowlisted chat model must be a real,
    /// fully-qualified Ollama id.
    ///
    /// The #5055 form of this test asserted "no entry may start with
    /// `gemma4:`", because no `gemma4` namespace existed at the time. Gemma 4
    /// has since been published and `gemma4:e4b-it-q8_0` resolves against
    /// `registry.ollama.ai`, so that assertion was pinning an expired fact.
    /// The durable invariant is the `<model>:<tag>` shape plus the
    /// preset cross-check above.
    #[test]
    fn mvp_chat_allowlist_entries_are_fully_qualified() {
        for model in MVP_ALLOWED_CHAT_MODELS {
            assert!(
                model.contains(':'),
                "`{model}` must be a fully-qualified `<model>:<tag>` id"
            );
        }
    }

    #[test]
    fn vision_model_normalizes_legacy_moondream_values() {
        let mut config = test_config();

        // Empty stays empty: "vision not configured" is a real state.
        config.local_ai.vision_model_id = String::new();
        assert_eq!(effective_vision_model_id(&config), "");

        // Legacy shorthands normalize to the pinned Moondream build. Before
        // #5146 these resolved to "" (vision silently disabled) because the
        // vision allowlist contained only the empty string.
        config.local_ai.vision_model_id = "moondream".to_string();
        assert_eq!(effective_vision_model_id(&config), DEFAULT_LOW_VISION_MODEL);
        config.local_ai.vision_model_id = "moondream:1.8b".to_string();
        assert_eq!(effective_vision_model_id(&config), DEFAULT_LOW_VISION_MODEL);
    }

    /// #5146 §Part 1: a genuinely vision-capable model must survive resolution
    /// unchanged. The previous `MVP_ALLOWED_VISION_MODELS = &[""]` allowlist
    /// rewrote every one of these to `""`.
    #[test]
    fn vision_capable_models_pass_through_unchanged() {
        let mut config = test_config();
        for model in ["llava:7b", "gemma3:4b-it-qat", "gemma4:e4b-it-q8_0"] {
            config.local_ai.vision_model_id = model.to_string();
            assert_eq!(effective_vision_model_id(&config), model);
        }
    }

    /// #5146 §Part 1 / P1: a chat-only model must never be returned as the
    /// vision model — and must not be quietly swapped for one either. Both
    /// resolvers report it as unusable so no pull path can act on it.
    #[test]
    fn chat_only_vision_model_resolves_to_nothing_usable() {
        let mut config = test_config();
        for chat_only in ["gemma3n:e4b-it-q8_0", "gemma3:1b-it-qat", "llama3.1:8b"] {
            config.local_ai.vision_model_id = chat_only.to_string();
            assert_eq!(
                effective_vision_model_id(&config),
                "",
                "{chat_only} must not resolve to a substitute"
            );
            assert!(
                resolve_vision_model_id(&config).is_err(),
                "{chat_only} must be an actionable error at request time"
            );
        }
    }

    /// The pinned default must itself be vision-capable: it is what the
    /// `moondream` alias resolves to, and what the "for example …" suggestions
    /// point users at, so a chat-only default would send them in a circle.
    #[test]
    fn default_vision_model_is_vision_capable() {
        assert!(!DEFAULT_LOW_VISION_MODEL.is_empty());
        assert!(vision_models::is_vision_capable(DEFAULT_LOW_VISION_MODEL));
    }

    /// #5146 §Part 1: an unconfigured vision model must produce an actionable
    /// error, not an empty model id that downstream code sends to Ollama.
    #[test]
    fn resolve_vision_model_id_errors_when_unconfigured() {
        let mut config = test_config();
        config.local_ai.vision_model_id = String::new();

        let err = resolve_vision_model_id(&config)
            .err()
            .expect("expected a vision error");
        assert!(
            err.contains("vision_model_id"),
            "error should name the config key to set: {err}"
        );
        assert!(
            err.contains("ollama pull"),
            "error should say how to install a model: {err}"
        );
        // Whitespace-only is the same "not configured" state.
        config.local_ai.vision_model_id = "   ".to_string();
        assert!(resolve_vision_model_id(&config).is_err());
    }

    #[test]
    fn resolve_vision_model_id_returns_the_configured_model_when_it_can_see() {
        let mut config = test_config();
        config.local_ai.vision_model_id = "llava:7b".to_string();
        assert_eq!(resolve_vision_model_id(&config).unwrap(), "llava:7b");
    }

    /// An alias rewrite resolves to a different string but is the *same* model
    /// the user asked for, so it stays silent and must keep working.
    #[test]
    fn resolve_vision_model_id_still_applies_the_moondream_alias() {
        let mut config = test_config();
        for alias in ["moondream", "moondream:1.8b", "MoonDream"] {
            config.local_ai.vision_model_id = alias.to_string();
            let resolved = resolve_vision_model_id(&config)
                .unwrap_or_else(|e| panic!("alias {alias} must resolve, got: {e}"));
            assert_eq!(resolved, DEFAULT_LOW_VISION_MODEL);
            assert!(vision_models::is_vision_capable(&resolved));
        }
    }

    // ── #5146 P1: a chat-only vision model errors, never substitutes ─────────

    /// The headline P1 regression. A chat-only `vision_model_id` used to be
    /// silently swapped for the default vision model, which was then
    /// auto-pulled (~1.7 GB, no progress) and answered many prompts with an
    /// empty string — surfacing as `ollama vision returned empty content`.
    #[test]
    fn resolve_vision_model_id_errors_on_a_chat_only_model_instead_of_substituting() {
        let mut config = test_config();
        config.local_ai.vision_model_id = "gemma3n:e4b-it-q8_0".to_string();

        let err = resolve_vision_model_id(&config)
            .err()
            .expect("a chat-only vision model must be an error, not a substitution");

        assert!(
            err.contains("gemma3n:e4b-it-q8_0"),
            "the error must name the model the user actually configured: {err}"
        );
        assert!(
            err.contains("not vision-capable"),
            "the error must say what is wrong with it: {err}"
        );
        assert!(
            err.contains("vision_model_id"),
            "the error must name the key to change: {err}"
        );
        // `DEFAULT_LOW_VISION_MODEL` is also `VISION_MODEL_SUGGESTIONS[0]`, so
        // asserting its absence would assert against the suggestion list
        // itself. The contract is the framing: it is offered as one example to
        // pick from, not announced as the model that replaced the user's.
        assert!(
            err.contains("for example"),
            "a vision-capable model must be offered as an example to choose, never as a \
             substitute that was already applied: {err}"
        );
        assert!(
            !err.contains("selected vision model `moondream"),
            "the error must name the user's model as the problem, not a suggestion: {err}"
        );
    }

    /// The auto-pull half of P1: several callers feed
    /// `effective_vision_model_id` straight into `ensure_ollama_model_available`,
    /// so a substituted id here is exactly how an unchosen model got downloaded.
    /// Empty keeps those paths off it (and `ensure_ollama_model_available`
    /// rejects a blank id rather than pulling a nameless model).
    #[test]
    fn effective_vision_model_id_is_empty_for_a_chat_only_model() {
        let mut config = test_config();
        config.local_ai.vision_model_id = "gemma3n:e4b-it-q8_0".to_string();
        assert_eq!(
            effective_vision_model_id(&config),
            "",
            "a chat-only model must not resolve to a substitute that a pull path would download"
        );

        // Unchanged for the two states that already worked.
        config.local_ai.vision_model_id = String::new();
        assert_eq!(effective_vision_model_id(&config), "");
        config.local_ai.vision_model_id = "llava:7b".to_string();
        assert_eq!(effective_vision_model_id(&config), "llava:7b");
    }

    /// `effective_vision_model_id` and `resolve_vision_model_id` must agree on
    /// which models are usable — a non-empty effective id that the resolver
    /// rejects (or vice versa) would put status surfaces and request-time
    /// behaviour out of sync.
    #[test]
    fn effective_and_resolved_vision_ids_agree_on_usability() {
        let mut config = test_config();
        for candidate in [
            "llava:7b",
            "gemma3n:e4b-it-q8_0",
            "moondream",
            "llama3.2:3b",
            "",
        ] {
            config.local_ai.vision_model_id = candidate.to_string();
            let effective = effective_vision_model_id(&config);
            let resolved = resolve_vision_model_id(&config);
            assert_eq!(
                effective.is_empty(),
                resolved.is_err(),
                "disagreement for {candidate:?}: effective={effective:?} resolved={resolved:?}"
            );
            if let Ok(model) = resolved {
                assert_eq!(effective, model);
            }
        }
    }

    #[test]
    fn embedding_model_empty_falls_back_to_bge_m3() {
        // After the cloud-embeddings unification PR, the default embedder
        // for the local Ollama path is bge-m3 (1024 dim) to match memory
        // tree's fixed on-disk format. Empty / whitespace input must
        // resolve to that default, not the prior all-minilm:latest.
        let mut config = test_config();
        config.local_ai.embedding_model_id = String::new();
        assert_eq!(effective_embedding_model_id(&config), "bge-m3");

        config.local_ai.embedding_model_id = "   ".to_string();
        assert_eq!(effective_embedding_model_id(&config), "bge-m3");
    }

    #[test]
    fn embedding_model_passes_through_allowlisted_legacy() {
        // all-minilm:latest is kept in MVP_ALLOWED_EMBEDDING_MODELS for
        // back-compat with users who already pulled it under the prior
        // default. It is NOT 1024-dim — memory tree's post-call validator
        // will surface that mismatch at embed time — but the allowlist
        // enforcer itself must let the value pass through unchanged.
        let mut config = test_config();
        config.local_ai.embedding_model_id = "all-minilm:latest".to_string();
        assert_eq!(effective_embedding_model_id(&config), "all-minilm:latest");
    }

    #[test]
    fn embedding_model_rejects_non_allowlisted_and_redirects_to_default() {
        // Any non-allowlisted value (including legacy nomic-embed-text:latest
        // and arbitrary user input) is silently redirected to the canonical
        // default. This is the path that fired the "embedding model not in
        // MVP allowlist, redirecting to default" warning on every embed
        // resolution before bge-m3 was added to the allowlist.
        let mut config = test_config();
        config.local_ai.embedding_model_id = "nomic-embed-text:latest".to_string();
        assert_eq!(effective_embedding_model_id(&config), "bge-m3");

        config.local_ai.embedding_model_id = "totally-made-up-model:v0".to_string();
        assert_eq!(effective_embedding_model_id(&config), "bge-m3");
    }

    #[test]
    fn lm_studio_embedding_model_passes_through_served_name() {
        // The native local-runtime fix for #3920: LM Studio serves embeddings
        // under user-managed names that are not in the MVP allowlist. A
        // configured id must reach the runtime unchanged rather than being
        // rewritten back to bge-m3 (which the LM Studio server would not have
        // under that exact name).
        let mut config = test_config();
        config.local_ai.provider = "lm_studio".to_string();
        config.local_ai.embedding_model_id = "text-embedding-bge-m3".to_string();
        assert_eq!(
            effective_embedding_model_id(&config),
            "text-embedding-bge-m3"
        );
    }

    #[test]
    fn lm_studio_embedding_model_passes_through_arbitrary_id() {
        // Contrast with `embedding_model_rejects_non_allowlisted_and_redirects_to_default`:
        // the SAME non-allowlisted id is rewritten to bge-m3 on the managed
        // Ollama path but passes through unchanged on the LM Studio path.
        let mut config = test_config();
        config.local_ai.provider = "lm_studio".to_string();
        config.local_ai.embedding_model_id = "nomic-embed-text:latest".to_string();
        assert_eq!(
            effective_embedding_model_id(&config),
            "nomic-embed-text:latest"
        );
    }

    #[test]
    fn lm_studio_embedding_model_empty_falls_back_to_default() {
        // With no configured embedding id, fall back to the canonical default
        // so the memory tree still has an embedder to request.
        let mut config = test_config();
        config.local_ai.provider = "lm_studio".to_string();
        config.local_ai.embedding_model_id = String::new();
        assert_eq!(effective_embedding_model_id(&config), "bge-m3");

        config.local_ai.embedding_model_id = "   ".to_string();
        assert_eq!(effective_embedding_model_id(&config), "bge-m3");
    }

    #[test]
    fn ollama_embedding_path_still_enforces_allowlist_after_lm_studio_bypass() {
        // Guard: the LM Studio bypass must not weaken the managed Ollama path.
        // Default provider (Ollama) still rewrites a non-allowlisted id.
        let mut config = test_config();
        config.local_ai.embedding_model_id = "text-embedding-bge-m3".to_string();
        assert_eq!(effective_embedding_model_id(&config), "bge-m3");
    }

    #[test]
    fn stt_tts_and_quantization_defaults_are_applied() {
        let mut config = test_config();
        config.local_ai.stt_model_id.clear();
        config.local_ai.tts_voice_id.clear();
        config.local_ai.quantization = "Q5_K_M".to_string();

        assert_eq!(effective_stt_model_id(&config), "ggml-base-q5_1.bin");
        assert_eq!(effective_tts_voice_id(&config), "en_US-lessac-medium");
        assert_eq!(effective_quantization(&config), "q5_k_m");
    }
}
