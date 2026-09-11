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
#[path = "vision_models_tests.rs"]
mod tests;
