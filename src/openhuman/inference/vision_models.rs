//! Which local model IDs can actually accept image input.
//!
//! Every vision path resolves its model through this registry first, so a
//! vision tool-call either reaches a genuinely vision-capable model or surfaces
//! a clear error. See [`crate::openhuman::inference::model_ids`].
//!
//! ## Why resolve up front rather than let Ollama reject it
//!
//! This module originally claimed that routing a vision request at a chat-only
//! model is *not* a loud failure — that Ollama accepts the `images` array
//! against any model, silently discards it, and answers from the prompt text
//! alone, yielding a confident hallucination.
//!
//! **That was measured against Ollama 0.32.5 and does not hold** (#5146 P5).
//! All three endpoints — `/api/generate`, `/api/chat`, and the OpenAI-compatible
//! `/v1/chat/completions` — reject the request cleanly:
//!
//! ```text
//! HTTP 400 {"error":{"code":400,
//!   "message":"Multimodal data provided, but model does not support multimodal requests.",
//!   "type":"invalid_request_error"}}
//! ```
//!
//! The registry is still worth having, for reasons that do not depend on the
//! old claim:
//!
//! - a raw upstream 400 is not an actionable message — resolving first lets us
//!   name the model, the capability, and the remedy;
//! - it lets the caller fail *before* spending a model load or a pull;
//! - older Ollama builds, and other OpenAI-compatible local servers reached
//!   through the same code path, make no such guarantee.
//!
//! Keep this note accurate: a future reader deciding whether the registry can
//! be deleted must not rely on a silent-discard premise that no current Ollama
//! exhibits.
//!
//! The families below were verified against the live Ollama library registry
//! (`GET https://registry.ollama.ai/v2/library/<name>/manifests/<tag>` plus the
//! published capability badges on `ollama.com/library/<name>`).

/// Model families where every published tag accepts image input.
///
/// Kept as whole-family entries (the segment before `:`) rather than loose
/// substrings so a near-miss like `gemma3n` can never match a `gemma3` rule.
const VISION_CAPABLE_FAMILIES: &[&str] = &[
    "moondream",
    "llava",
    "llava-llama3",
    "llava-phi3",
    "bakllava",
    "llama3.2-vision",
    "llama4",
    "minicpm-v",
    "granite3.2-vision",
    "qwen2-vl",
    "qwen2.5vl",
    "mistral-small3.1",
    "mistral-small3.2",
    // Gemma 4 is multimodal at every published size, including the `e2b` /
    // `e4b` edge builds. Contrast `gemma3n` below.
    "gemma4",
];

/// Families that look vision-capable by name but are text-only.
///
/// `gemma3n` is the load-bearing entry: it is a *separate* model from
/// `gemma3`, shares its prefix, and ships **text input only** on Ollama. It
/// was the 16 GB+ preset's vision model before #5146.
const TEXT_ONLY_FAMILIES: &[&str] = &["gemma3n"];

/// Substrings that identify a repackaged upstream vision model, e.g.
/// `hf.co/user/llava-v1.6-mistral-7b` or a locally re-tagged `my-moondream`.
/// Only consulted after the exact-family rules above.
const VISION_MARKERS: &[&str] = &["llava", "moondream", "bakllava", "vision"];

/// Vision models suggested to the user when none is configured. Every entry is
/// pullable from the Ollama library with no extra setup.
pub(crate) const VISION_MODEL_SUGGESTIONS: &[&str] =
    &["moondream:1.8b-v2-q4_K_S", "llava:7b", "gemma3:4b-it-qat"];

/// Gemma 3 is split by size: `270m` and `1b` are text-only, while `4b`, `12b`
/// and `27b` are multimodal. `gemma3:latest` resolves to the 4B build.
fn gemma3_tag_is_multimodal(tag: &str) -> bool {
    if tag.is_empty() || tag == "latest" {
        return true;
    }
    !(tag.starts_with("270m") || tag.starts_with("1b"))
}

/// Returns `true` when `model_id` names a model that can accept image input.
///
/// Errs toward `false`: an unknown id is treated as chat-only so the caller
/// reports "no vision model available" instead of shipping images to a model
/// that will quietly ignore them.
pub(crate) fn is_vision_capable(model_id: &str) -> bool {
    let normalized = model_id.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return false;
    }

    let (family, tag) = match normalized.split_once(':') {
        Some((family, tag)) => (family, tag),
        None => (normalized.as_str(), ""),
    };

    if TEXT_ONLY_FAMILIES.contains(&family) {
        return false;
    }
    if family == "gemma3" {
        return gemma3_tag_is_multimodal(tag);
    }
    if VISION_CAPABLE_FAMILIES.contains(&family) {
        return true;
    }

    VISION_MARKERS
        .iter()
        .any(|marker| normalized.contains(marker))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedicated_vision_families_are_capable() {
        assert!(is_vision_capable("moondream:1.8b-v2-q4_K_S"));
        assert!(is_vision_capable("moondream"));
        assert!(is_vision_capable("llava:7b"));
        assert!(is_vision_capable("llava:13b"));
        assert!(is_vision_capable("bakllava:latest"));
        assert!(is_vision_capable("llama3.2-vision:11b"));
        assert!(is_vision_capable("minicpm-v:8b"));
        assert!(is_vision_capable("qwen2.5vl:7b"));
    }

    #[test]
    fn gemma3_is_multimodal_only_at_4b_and_above() {
        // 270m / 1b ship without a vision encoder.
        assert!(!is_vision_capable("gemma3:270m-it-qat"));
        assert!(!is_vision_capable("gemma3:1b-it-qat"));
        assert!(!is_vision_capable("gemma3:1b"));

        assert!(is_vision_capable("gemma3:4b-it-qat"));
        assert!(is_vision_capable("gemma3:12b-it-qat"));
        assert!(is_vision_capable("gemma3:27b"));
        // `latest` is the 4B multimodal build.
        assert!(is_vision_capable("gemma3:latest"));
        assert!(is_vision_capable("gemma3"));
    }

    #[test]
    fn gemma3n_is_never_treated_as_vision_capable() {
        // Regression guard for #5146: `gemma3n` shares a prefix with `gemma3`
        // but is text-only, and it was previously wired in as the 16 GB+
        // tier's vision model.
        assert!(!is_vision_capable("gemma3n:e4b-it-q8_0"));
        assert!(!is_vision_capable("gemma3n:e2b"));
        assert!(!is_vision_capable("gemma3n"));
        assert!(!is_vision_capable("GEMMA3N:E4B-IT-Q8_0"));
    }

    #[test]
    fn gemma4_is_multimodal_at_every_size() {
        assert!(is_vision_capable("gemma4:e4b-it-q8_0"));
        assert!(is_vision_capable("gemma4:e2b-it-qat"));
        assert!(is_vision_capable("gemma4:12b"));
        assert!(is_vision_capable("gemma4"));
    }

    #[test]
    fn chat_only_models_are_rejected() {
        assert!(!is_vision_capable("llama3.1:8b"));
        assert!(!is_vision_capable("qwen2.5:14b"));
        assert!(!is_vision_capable("deepseek-r1:7b"));
        assert!(!is_vision_capable("phi4:latest"));
    }

    #[test]
    fn embedding_models_are_rejected() {
        assert!(!is_vision_capable("bge-m3"));
        assert!(!is_vision_capable("all-minilm:latest"));
        assert!(!is_vision_capable("nomic-embed-text:latest"));
    }

    #[test]
    fn empty_and_whitespace_are_rejected() {
        assert!(!is_vision_capable(""));
        assert!(!is_vision_capable("   "));
    }

    #[test]
    fn repackaged_upstream_vision_models_are_detected() {
        assert!(is_vision_capable("hf.co/user/llava-v1.6-mistral-7b"));
        assert!(is_vision_capable("my-moondream:custom"));
        assert!(is_vision_capable("someone/llama3.2-vision-abliterated"));
    }

    #[test]
    fn detection_is_case_insensitive() {
        assert!(is_vision_capable("LLaVA:7B"));
        assert!(is_vision_capable("MoonDream"));
        assert!(!is_vision_capable("Gemma3:1B-IT-QAT"));
    }

    #[test]
    fn every_suggested_vision_model_is_vision_capable() {
        // The suggestions are quoted verbatim in the user-facing errors from
        // `model_ids::resolve_vision_model_id` — both the "nothing configured"
        // and the "not vision-capable" arms. A chat-only entry here would send
        // a user straight back into the error they are trying to escape.
        for suggestion in VISION_MODEL_SUGGESTIONS {
            assert!(
                is_vision_capable(suggestion),
                "suggested vision model `{suggestion}` is not vision-capable"
            );
        }
    }
}
