//! Known model context-window sizes for pre-inference budgeting.
//!
//! Provider `/models` responses may include `context_length` / `context_window`,
//! but the agent harness must enforce limits **before** the first dispatch —
//! otherwise long histories produce upstream `400 Bad Request` errors when usage
//! metadata is not yet available.

use crate::openhuman::config::{
    MODEL_AGENTIC_V1, MODEL_CHAT_V1, MODEL_CODING_V1, MODEL_REASONING_QUICK_V1, MODEL_REASONING_V1,
};

/// Conservative default for OpenHuman abstract tier models (tokens).
const TIER_LARGE_CONTEXT: u64 = 200_000;
const TIER_STANDARD_CONTEXT: u64 = 128_000;
const TIER_LOCAL_CONTEXT: u64 = 8_192;

/// Last-resort context window (tokens) for a **local** provider when neither the
/// static model table nor the provider profile declares one.
///
/// Local runtimes (llama.cpp / vLLM via `LocalProviderKind::LocalOpenai`, whose
/// profile default is `None`) enforce the model's *loaded* `n_ctx` and reject an
/// over-budget prompt with a hard `400` (`n_keep >= n_ctx`) — issue #3550 /
/// Sentry TAURI-RUST-6V0. Returning `None` for those would **disable**
/// pre-dispatch trimming and let the overflow through. So instead of skipping,
/// we trim against this conservative floor: a slightly-too-aggressive trim is
/// strictly better than a guaranteed 400. The value is the smallest real local
/// profile default (MLX = 4096), chosen because any local runtime can hold at
/// least this much, while keeping the floor low enough to actually bound the
/// prompt. Only applied to local providers — cloud providers with an unknown
/// model keep `None` (their windows are large; a tiny floor would needlessly
/// truncate legitimate large-context requests).
///
/// This floor is a **guess**, used for *trimming only*. It is deliberately not
/// fed into the engine's hard un-evictable-prefix abort ("reload with a larger
/// context length") — that abort consults the provider's *authoritative*
/// [`crate::openhuman::inference::provider::Provider::loaded_context_window`]
/// instead, so we never reject a request against a guessed window the real
/// loaded `n_ctx` would have accepted (Codex P1 review on PR #3771).
const CONSERVATIVE_LOCAL_CONTEXT_FLOOR: u64 = 4_096;
/// Summarization tier. `summarization-v1` resolves to a long-context flash
/// model (currently DeepSeek v4 flash, ~1M tokens). `extract_from_result`
/// uses this window to single-shot whole oversized payloads instead of
/// chunking, so it must reflect the real backing model's capacity.
const TIER_SUMMARIZATION_CONTEXT: u64 = 1_000_000;

/// How a pattern in [`MODEL_CONTEXT_PATTERNS`] is matched against a model id.
#[derive(Copy, Clone)]
enum PatternMatch {
    /// Pattern must appear anywhere as a substring (after lowercasing).
    Substring,
    /// Pattern must appear as a full `-`/`_`/`/`/`:`-delimited segment.
    /// Prevents false positives like `"solo1-7b"` matching the `"o1"` pattern
    /// or `"proto3-chat"` matching the `"o3"` pattern.
    Segment,
}

/// `(pattern, match mode, context window in tokens)` — first match wins.
const MODEL_CONTEXT_PATTERNS: &[(&str, PatternMatch, u64)] = &[
    ("claude-haiku-4.5", PatternMatch::Substring, 200_000),
    ("claude-haiku-4", PatternMatch::Substring, 200_000),
    ("claude-haiku", PatternMatch::Substring, 200_000),
    ("claude-sonnet-4", PatternMatch::Substring, 200_000),
    ("claude-opus-4", PatternMatch::Substring, 200_000),
    ("claude-3-5-sonnet", PatternMatch::Substring, 200_000),
    ("claude-3-5-haiku", PatternMatch::Substring, 200_000),
    ("claude-3-opus", PatternMatch::Substring, 200_000),
    ("gpt-4.1", PatternMatch::Substring, 1_047_576),
    ("gpt-4o", PatternMatch::Substring, 128_000),
    ("gpt-4-turbo", PatternMatch::Substring, 128_000),
    ("gpt-4", PatternMatch::Substring, 128_000),
    ("gpt-3.5", PatternMatch::Substring, 16_385),
    // `o1`/`o3` are short and collide with substrings of unrelated model ids
    // (e.g. `solo1-7b`, `proto3-chat`). Require a segment-boundary match.
    ("o1", PatternMatch::Segment, 200_000),
    ("o3", PatternMatch::Segment, 200_000),
    ("deepseek", PatternMatch::Substring, 128_000),
    ("gemma3", PatternMatch::Substring, 8_192),
    ("gemma", PatternMatch::Substring, 8_192),
    ("llama-3", PatternMatch::Substring, 128_000),
    ("llama3", PatternMatch::Substring, 128_000),
];

fn matches_pattern(lower: &str, pattern: &str, mode: PatternMatch) -> bool {
    match mode {
        PatternMatch::Substring => lower.contains(pattern),
        PatternMatch::Segment => lower
            .split(|c: char| matches!(c, '/' | '-' | '_' | ':' | '.'))
            .any(|seg| seg == pattern),
    }
}

/// Resolve the context window (in tokens) for a model id or OpenHuman tier alias.
///
/// Returns `None` when the model is unknown — callers should skip pre-dispatch
/// trimming rather than guess.
pub fn context_window_for_model(model: &str) -> Option<u64> {
    let normalized = model.trim();
    if normalized.is_empty() {
        return None;
    }

    if let Some(window) = tier_context_window(normalized) {
        return Some(window);
    }

    let lower = normalized.to_ascii_lowercase();
    for (pattern, mode, window) in MODEL_CONTEXT_PATTERNS {
        if matches_pattern(&lower, pattern, *mode) {
            tracing::debug!(
                model = normalized,
                pattern,
                context_window = window,
                "[model_context] matched known model pattern"
            );
            return Some(*window);
        }
    }

    None
}

fn tier_context_window(model: &str) -> Option<u64> {
    match model {
        MODEL_REASONING_V1 | MODEL_AGENTIC_V1 | MODEL_CODING_V1 => Some(TIER_LARGE_CONTEXT),
        "summarization-v1" => Some(TIER_SUMMARIZATION_CONTEXT),
        MODEL_CHAT_V1 | MODEL_REASONING_QUICK_V1 | "chat" => Some(TIER_STANDARD_CONTEXT),
        m if m.starts_with("gemma") || m.contains(":1b") || m.contains("270m") => {
            Some(TIER_LOCAL_CONTEXT)
        }
        _ => None,
    }
}

/// Resolve context window with local provider profile fallback.
///
/// When `context_window_for_model` returns `None` (unknown model name —
/// common for local models like `qwen3:14b`, `phi3:mini`, etc.) this
/// function falls back to the provider profile's declared default context
/// window. This ensures preflight trimming still works for local models
/// even when the exact model name isn't in the static pattern table.
///
/// For a **local** provider this never returns `None`: if the profile itself
/// declares no default (e.g. `LocalProviderKind::LocalOpenai` = llama.cpp /
/// vLLM), it falls back once more to [`CONSERVATIVE_LOCAL_CONTEXT_FLOOR`] so
/// trimming still engages instead of being silently skipped and overflowing
/// the runtime `n_ctx` (issue #3550 / Sentry TAURI-RUST-6V0). `None` is only
/// returned when `local_kind` is `None` (cloud provider, unknown model) —
/// where over-trimming a large window is worse than skipping the trim.
pub fn context_window_for_model_with_local_fallback(
    model: &str,
    local_kind: Option<crate::openhuman::inference::local::profile::LocalProviderKind>,
) -> Option<u64> {
    if let Some(window) = context_window_for_model(model) {
        return Some(window);
    }
    // Fall back to the local provider profile's default context window.
    if let Some(kind) = local_kind {
        let profile = crate::openhuman::inference::local::profile::profile_for_kind(kind);
        if let Some(default_ctx) = profile.default_context_window {
            tracing::debug!(
                model,
                provider = kind.as_str(),
                context_window = default_ctx,
                "[model_context] using local provider profile default context window"
            );
            return Some(default_ctx);
        }
        // Local provider with no declared default (llama.cpp / vLLM). Never
        // return `None` here — that would disable pre-dispatch trimming and let
        // the prompt overflow the runtime `n_ctx` (the TAURI-RUST-6V0 400). Trim
        // against a conservative floor instead.
        tracing::debug!(
            model,
            provider = kind.as_str(),
            context_window = CONSERVATIVE_LOCAL_CONTEXT_FLOOR,
            "[model_context] local provider has no profile default; using conservative context floor"
        );
        return Some(CONSERVATIVE_LOCAL_CONTEXT_FLOOR);
    }
    None
}

/// Whether the model resolved for a chat hint/agent/profile accepts image input
/// according to the **user-configured** vision flag in `config.model_registry`.
///
/// This is the per-model override that lets a user mark a **custom / BYOK** model
/// as vision-capable (Settings → Advanced LLM → custom model → "Supports
/// vision"). Managed-backend models already advertise vision via
/// [`crate::openhuman::inference::provider::Provider::supports_vision`]; this flag
/// covers OpenAI-compatible providers the backend can't introspect per-model.
/// Returns `false` for models the user has not flagged.
pub fn model_vision_enabled(model: &str, config: &crate::openhuman::config::Config) -> bool {
    let normalized = model.trim();
    if normalized.is_empty() {
        return false;
    }
    let enabled = config
        .model_registry
        .iter()
        .any(|entry| entry.id == normalized && entry.vision);
    tracing::debug!(
        model = normalized,
        vision_enabled = enabled,
        "[model_context] resolved user-configured vision flag"
    );
    enabled
}

/// Whether a resolved model accepts image input. The single predicate shared by
/// the chat UI resolve and the server-side session/sub-agent gates.
///
/// - **Managed OpenHuman tiers** consult the hardcoded per-tier map
///   ([`crate::openhuman::inference::provider::factory::oh_tier_supports_vision`]) —
///   the remote backend does not advertise per-tier capability, so the core owns
///   it. Currently only `reasoning-v1` is vision-capable.
/// - **Custom/BYOK models** consult the user-set `model_registry.vision` flag
///   ([`model_vision_enabled`]).
pub fn model_supports_vision(model: &str, config: &crate::openhuman::config::Config) -> bool {
    crate::openhuman::inference::provider::factory::oh_tier_supports_vision(model)
        || model_vision_enabled(model, config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::inference::local::profile::LocalProviderKind;

    #[test]
    fn local_fallback_uses_profile_default() {
        // Unknown model with Ollama profile → 8192 default
        assert_eq!(
            context_window_for_model_with_local_fallback(
                "qwen3:14b",
                Some(LocalProviderKind::Ollama)
            ),
            Some(8_192)
        );
        // Unknown model with MLX profile → 4096 default
        assert_eq!(
            context_window_for_model_with_local_fallback(
                "my-custom-model",
                Some(LocalProviderKind::Mlx)
            ),
            Some(4_096)
        );
        // Unknown model with no local provider → None
        assert_eq!(
            context_window_for_model_with_local_fallback("qwen3:14b", None),
            None
        );
        // Local provider whose profile declares no default (llama.cpp / vLLM via
        // LocalOpenai) → conservative floor, NOT None. None here would disable
        // pre-dispatch trimming and let the prompt overflow the runtime n_ctx
        // (the TAURI-RUST-6V0 400). Must stay bounded.
        assert_eq!(
            context_window_for_model_with_local_fallback(
                "some-unlisted-gguf",
                Some(LocalProviderKind::LocalOpenai)
            ),
            Some(super::CONSERVATIVE_LOCAL_CONTEXT_FLOOR)
        );
        // Known model ignores local fallback
        assert_eq!(
            context_window_for_model_with_local_fallback(
                "llama3:8b",
                Some(LocalProviderKind::Ollama)
            ),
            Some(128_000)
        );
    }

    #[test]
    fn tier_aliases_resolve() {
        assert_eq!(context_window_for_model("reasoning-v1"), Some(200_000));
        assert_eq!(context_window_for_model("agentic-v1"), Some(200_000));
        assert_eq!(context_window_for_model("chat-v1"), Some(128_000));
        assert_eq!(
            context_window_for_model("reasoning-quick-v1"),
            Some(128_000)
        );
        // summarization-v1 maps to a ~1M-token flash model so the extractor can
        // single-shot whole oversized payloads.
        assert_eq!(
            context_window_for_model("summarization-v1"),
            Some(1_000_000)
        );
    }

    #[test]
    fn copilot_haiku_resolves_to_200k() {
        assert_eq!(
            context_window_for_model("github_copilot/claude-haiku-4.5"),
            Some(200_000)
        );
    }

    #[test]
    fn unknown_model_returns_none() {
        assert_eq!(context_window_for_model("totally-unknown-model-xyz"), None);
    }

    #[test]
    fn empty_model_returns_none() {
        assert_eq!(context_window_for_model("   "), None);
    }

    #[test]
    fn model_vision_enabled_reads_registry_only() {
        use crate::openhuman::config::schema::ModelRegistryEntry;
        let mut config = crate::openhuman::config::Config::default();
        config.model_registry = vec![
            ModelRegistryEntry {
                id: "my-llava".into(),
                provider: "openai".into(),
                cost_per_1m_output: 0.0,
                vision: true,
            },
            ModelRegistryEntry {
                id: "text-only".into(),
                provider: "openai".into(),
                cost_per_1m_output: 0.0,
                vision: false,
            },
        ];
        assert!(model_vision_enabled("my-llava", &config));
        assert!(!model_vision_enabled("text-only", &config));
        assert!(!model_vision_enabled("unlisted", &config));
        assert!(!model_vision_enabled("   ", &config));
    }

    #[test]
    fn model_supports_vision_combines_tier_map_and_registry() {
        use crate::openhuman::config::schema::ModelRegistryEntry;
        let mut config = crate::openhuman::config::Config::default();
        config.model_registry = vec![ModelRegistryEntry {
            id: "my-llava".into(),
            provider: "openai".into(),
            cost_per_1m_output: 0.0,
            vision: true,
        }];
        // `reasoning-v1` is the one vision-capable managed tier; the rest are not.
        assert!(model_supports_vision("reasoning-v1", &config));
        assert!(model_supports_vision("hint:reasoning", &config));
        assert!(!model_supports_vision("chat-v1", &config));
        assert!(!model_supports_vision("hint:chat", &config));
        // BYOK model flagged in the registry is vision-capable.
        assert!(model_supports_vision("my-llava", &config));
        // Unlisted custom model is not.
        assert!(!model_supports_vision("gpt-5", &config));
    }

    #[test]
    fn o1_o3_segment_match_does_not_overmatch() {
        // Real OpenAI o1/o3 model ids must still resolve.
        assert_eq!(context_window_for_model("o1"), Some(200_000));
        assert_eq!(context_window_for_model("o1-mini"), Some(200_000));
        assert_eq!(context_window_for_model("o3-mini"), Some(200_000));
        assert_eq!(context_window_for_model("openai/o1-preview"), Some(200_000));

        // Names that merely *contain* the substring "o1" / "o3" must NOT
        // inherit the 200K window (regression guard for PR #2100 review).
        assert_eq!(context_window_for_model("solo1-7b"), None);
        assert_eq!(context_window_for_model("proto3-chat"), None);
        assert_eq!(
            context_window_for_model("ollama/mistral-for-o1-benchmark"),
            Some(200_000),
            "`-o1-` segment should still match"
        );
        assert_eq!(context_window_for_model("octo3thing"), None);
    }
}
