//! Known model context-window sizes for pre-inference budgeting.
//!
//! Provider `/models` responses may include `context_length` / `context_window`,
//! but the agent harness must enforce limits **before** the first dispatch —
//! otherwise long histories produce upstream `400 Bad Request` errors when usage
//! metadata is not yet available.

use crate::openhuman::config::{
    MODEL_AGENTIC_V1, MODEL_BURST_V1, MODEL_CHAT_V1, MODEL_CODING_V1, MODEL_REASONING_QUICK_V1,
    MODEL_REASONING_V1,
};

/// Conservative default for OpenHuman abstract tier models (tokens).
const TIER_LARGE_CONTEXT: u64 = 200_000;
/// Reasoning tier — backed by a 1M-context model.
const TIER_REASONING_CONTEXT: u64 = 1_000_000;
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
/// DeepSeek v4 Flash window (~1M tokens) — the shared backing for every managed
/// "flash" tier: `chat-v1`, its legacy alias `reasoning-quick-v1`, and
/// `summarization-v1`. Kept as a single constant so these tiers can't drift
/// apart again (issue #4706: `chat-v1` was pinned at `TIER_STANDARD_CONTEXT`
/// (128K) while `summarization-v1` was 1M, even though both resolve to DeepSeek
/// v4 Flash in the backend model registry). `extract_from_result` also relies on
/// this window to single-shot whole oversized payloads instead of chunking, so
/// it must reflect the real backing model's capacity.
const TIER_FLASH_CONTEXT: u64 = 1_000_000;

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

    if let Some(price) = crate::openhuman::platform::cost::catalog::lookup(normalized) {
        tracing::debug!(
            model = normalized,
            catalog_model = price.model_id,
            context_window = price.context_window,
            "[model_context] matched cost catalog row"
        );
        return Some(u64::from(price.context_window));
    }

    if let Some(window) = tinyinference::model::context_window_for_model_id(normalized) {
        tracing::debug!(
            model = normalized,
            context_window = window,
            "[model_context] matched tinyagents model context hint"
        );
        return Some(window);
    }

    // The crate's `context_window_for_model_id` resolves the canonical o1/o3
    // ids (`o1`, `o1-mini`, `openai/o1-preview`, …) but does not match an
    // `o1`/`o3` token embedded mid-name (e.g. `ollama/mistral-for-o1-benchmark`).
    // OpenHuman keeps that segment heuristic host-side to preserve the
    // pre-port behavior (regression guard from PR #2100) until the crate
    // matcher covers internal segments.
    if let Some(window) = o1_o3_segment_context(normalized) {
        tracing::debug!(
            model = normalized,
            context_window = window,
            "[model_context] matched host o1/o3 segment heuristic"
        );
        return Some(window);
    }

    None
}

/// Match a bounded `o1`/`o3` segment anywhere in the model id and map it to the
/// OpenAI reasoning-model 200K window. The token must be delimited by a
/// non-alphanumeric boundary (or string start/end) on both sides so substrings
/// like `solo1-7b`, `proto3-chat`, or `octo3thing` do **not** over-match.
fn o1_o3_segment_context(model: &str) -> Option<u64> {
    const O_SERIES_CONTEXT: u64 = 200_000;
    let bytes = model.as_bytes();
    let n = bytes.len();
    let mut i = 0;
    while i + 1 < n {
        let is_o = bytes[i] == b'o' || bytes[i] == b'O';
        if is_o && (bytes[i + 1] == b'1' || bytes[i + 1] == b'3') {
            let before_ok = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();
            let after = i + 2;
            let after_ok = after >= n || !bytes[after].is_ascii_alphanumeric();
            if before_ok && after_ok {
                return Some(O_SERIES_CONTEXT);
            }
        }
        i += 1;
    }
    None
}

fn tier_context_window(model: &str) -> Option<u64> {
    match model {
        MODEL_REASONING_V1 => Some(TIER_REASONING_CONTEXT),
        MODEL_AGENTIC_V1 | MODEL_CODING_V1 => Some(TIER_LARGE_CONTEXT),
        "summarization-v1" => Some(TIER_FLASH_CONTEXT),
        // Burst tier advertises a 128k window on the managed backend. Matched on
        // the `burst-v1` alias before any substring fallbacks below.
        MODEL_BURST_V1 => Some(TIER_STANDARD_CONTEXT),
        // `chat-v1` (and its legacy alias `reasoning-quick-v1`) are backed by
        // DeepSeek v4 Flash — the same ~1M model as `summarization-v1`, not a
        // 128K model (issue #4706). Share `TIER_FLASH_CONTEXT` so the three
        // flash tiers stay in lockstep.
        MODEL_CHAT_V1 | MODEL_REASONING_QUICK_V1 | "chat" => Some(TIER_FLASH_CONTEXT),
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
///   it. Currently `reasoning-v1` and `vision-v1` — plus their `hint:reasoning`
///   / `hint:vision` aliases — are vision-capable; every other tier is not.
/// - **Custom/BYOK models** consult the user-set `model_registry.vision` flag
///   ([`model_vision_enabled`]).
pub fn model_supports_vision(model: &str, config: &crate::openhuman::config::Config) -> bool {
    crate::openhuman::inference::provider::factory::oh_tier_supports_vision(model)
        || model_vision_enabled(model, config)
}

#[cfg(test)]
#[path = "model_context_tests.rs"]
mod tests;
