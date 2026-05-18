//! Per-model temperature suppression helpers.
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
mod tests {
    use super::*;
    use crate::openhuman::config::Config;

    // ── glob_match unit tests ─────────────────────────────────────────────────

    #[test]
    fn glob_exact_match() {
        assert!(glob_match("o1-preview", "o1-preview"));
    }

    #[test]
    fn glob_prefix_star() {
        assert!(glob_match("o1*", "o1-preview"));
        assert!(glob_match("o1*", "o1-mini"));
        assert!(glob_match("o1*", "o1"));
        assert!(!glob_match("o1*", "gpt-4o"));
    }

    #[test]
    fn glob_suffix_star() {
        assert!(glob_match("*mini", "gpt-4o-mini"));
        assert!(!glob_match("*mini", "gpt-4o-large"));
    }

    #[test]
    fn glob_contains_star() {
        assert!(glob_match("gpt*mini", "gpt-4o-mini"));
        assert!(!glob_match("gpt*mini", "gpt-4o-large"));
    }

    #[test]
    fn glob_pure_star() {
        assert!(glob_match("*", "anything"));
        assert!(glob_match("*", ""));
    }

    #[test]
    fn glob_no_star_mismatch() {
        assert!(!glob_match("o1", "o1-preview"));
        assert!(glob_match("o1", "o1"));
    }

    #[test]
    fn glob_gpt5_pattern() {
        assert!(glob_match("gpt-5*", "gpt-5"));
        assert!(glob_match("gpt-5*", "gpt-5-turbo"));
        assert!(!glob_match("gpt-5*", "gpt-4o"));
    }

    // ── temperature_for_model tests ───────────────────────────────────────────

    fn config_with_unsupported(patterns: Vec<String>) -> Config {
        let mut config = Config::default();
        config.temperature_unsupported_models = patterns;
        config
    }

    #[test]
    fn temperature_returned_for_normal_model() {
        let config = Config::default(); // has ["o1*","o3*","o4*","gpt-5*"] by default
        assert_eq!(
            temperature_for_model("gpt-4o-mini", 0.7, &config),
            Some(0.7)
        );
        assert_eq!(
            temperature_for_model("claude-3-opus", 0.5, &config),
            Some(0.5)
        );
    }

    #[test]
    fn temperature_suppressed_for_o1_model() {
        let config = Config::default();
        assert_eq!(temperature_for_model("o1-preview", 0.7, &config), None);
        assert_eq!(temperature_for_model("o1-mini", 0.7, &config), None);
        assert_eq!(temperature_for_model("o1", 0.7, &config), None);
    }

    #[test]
    fn temperature_suppressed_for_o3_o4() {
        let config = Config::default();
        assert_eq!(temperature_for_model("o3", 0.7, &config), None);
        assert_eq!(temperature_for_model("o3-mini", 0.7, &config), None);
        assert_eq!(temperature_for_model("o4-mini", 0.7, &config), None);
    }

    #[test]
    fn temperature_suppressed_for_gpt5() {
        let config = Config::default();
        assert_eq!(temperature_for_model("gpt-5", 0.7, &config), None);
        assert_eq!(temperature_for_model("gpt-5-turbo", 0.7, &config), None);
    }

    #[test]
    fn temperature_uses_custom_unsupported_list() {
        let config = config_with_unsupported(vec!["custom-*".to_string()]);
        assert_eq!(temperature_for_model("custom-model", 0.7, &config), None);
        assert_eq!(
            temperature_for_model("gpt-4o-mini", 0.7, &config),
            Some(0.7)
        );
        // Default patterns no longer apply when list is replaced.
        assert_eq!(temperature_for_model("o1-preview", 0.7, &config), Some(0.7));
    }

    #[test]
    fn temperature_empty_list_always_returns_some() {
        let config = config_with_unsupported(vec![]);
        assert_eq!(temperature_for_model("o1-preview", 0.7, &config), Some(0.7));
        assert_eq!(temperature_for_model("gpt-5", 0.3, &config), Some(0.3));
    }
}
