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

// -- #2076: Moonshot Kimi K2 family — only accepts temperature: 1 -------

#[test]
fn temperature_suppressed_for_kimi_k2() {
    // Regression for #2076 (146 Sentry events). The upstream API returns
    //   "invalid temperature: only 1 is allowed for this model"
    // when a non-1 temperature is sent for any Kimi K2 variant. Omitting
    // the field entirely is correct — upstream defaults to 1.0.
    let config = Config::default();
    assert_eq!(temperature_for_model("kimi-k2.6", 0.7, &config), None);
    assert_eq!(
        temperature_for_model("kimi-k2-instruct", 0.5, &config),
        None
    );
    assert_eq!(temperature_for_model("kimi-k2-pro", 1.2, &config), None);
}

#[test]
fn temperature_suppressed_for_moonshot_namespaced_kimi() {
    // OpenRouter and similar gateways namespace Kimi under
    // `moonshot/...` or `moonshotai/...`. The same temperature
    // constraint applies regardless of how the model id is namespaced.
    let config = Config::default();
    assert_eq!(
        temperature_for_model("moonshot/kimi-k2.6", 0.7, &config),
        None
    );
    assert_eq!(
        temperature_for_model("moonshotai/kimi-k2-instruct", 0.5, &config),
        None
    );
    assert_eq!(temperature_for_model("moonshot-v1-8k", 0.3, &config), None);
}

#[test]
fn temperature_still_allowed_for_unrelated_models_after_kimi_additions() {
    // Regression guard: adding the `kimi-k2*` / `moonshot*` patterns
    // must NOT start suppressing temperature for unrelated models.
    // Pin a few common ones that share substrings.
    let config = Config::default();
    // "ki..." prefix that isn't kimi.
    assert_eq!(
        temperature_for_model("kimichat-legacy", 0.7, &config),
        Some(0.7)
    );
    // "moon..." prefix without the rest of the moonshot stem.
    assert_eq!(
        temperature_for_model("moonshine-asr", 0.7, &config),
        Some(0.7)
    );
    // Claude / GPT non-5 / Gemini all stay unaffected.
    assert_eq!(
        temperature_for_model("claude-sonnet-4.6", 0.5, &config),
        Some(0.5)
    );
    assert_eq!(
        temperature_for_model("gemini-2.5-pro", 0.3, &config),
        Some(0.3)
    );
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
