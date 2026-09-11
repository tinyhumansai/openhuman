//! Host-owned per-model temperature suppression helpers.
//!
//! Some models (OpenAI o-series, GPT-5 reasoning variants) reject the
//! `temperature` field in the request body and return an error when it is
//! present. `temperature_for_model` consults the config's
//! `temperature_unsupported_models` list (which accepts shell-style `*`
//! globs) and returns `None` when the model matches, causing the
//! serialisation layer to omit the field via `skip_serializing_if`.

use crate::openhuman::config::Config;

/// Returns the effective temperature for `model`, or `None` if the model
/// is listed in `config.temperature_unsupported_models`.
///
/// The list entries support shell-style `*` wildcard matching (no `?` or
/// `[]`). Matching is case-sensitive and done against the full model ID.
///
/// # Examples
///
/// ```
/// // model "o1-preview" matches pattern "o1*" → None
/// // model "gpt-4o-mini" matches no pattern   → Some(0.7)
/// ```
pub fn temperature_for_model(model: &str, default: f64, config: &Config) -> Option<f64> {
    if config
        .temperature_unsupported_models
        .iter()
        .any(|pat| glob_match(pat, model))
    {
        tracing::debug!(
            "[inference][temperature] model='{}' matched unsupported-temperature list — omitting temperature field",
            model
        );
        None
    } else {
        Some(default)
    }
}

/// Minimal shell-style glob matcher supporting only `*` (match any sequence
/// of characters, including empty). Does not support `?` or `[...]`.
///
/// This avoids pulling in the `glob` crate for what is effectively a
/// starts-with / ends-with / contains check.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    // Split on `*` and consume the text segment by segment.
    let parts: Vec<&str> = pattern.split('*').collect();

    if parts.is_empty() {
        // Pattern is purely `*` — matches everything.
        return true;
    }

    let mut remaining = text;

    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            // Consecutive stars or leading/trailing star — skip.
            continue;
        }

        if i == 0 {
            // First segment: must match the start of `text`.
            if !remaining.starts_with(part) {
                return false;
            }
            remaining = &remaining[part.len()..];
        } else {
            // Middle or last segment: find first occurrence in `remaining`.
            match remaining.find(part) {
                Some(pos) => {
                    remaining = &remaining[pos + part.len()..];
                }
                None => return false,
            }
        }
    }

    // If the pattern did NOT end with `*`, the remaining text must be empty.
    if !pattern.ends_with('*') && !remaining.is_empty() {
        return false;
    }

    true
}

#[cfg(test)]
#[path = "temperature_tests.rs"]
mod tests;
