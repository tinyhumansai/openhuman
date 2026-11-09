use super::apply_module_scrubber;
use super::scrub_secrets;

fn module_redaction(_input: &str) -> Option<String> {
    Some("module-redacted".to_owned())
}

#[test]
fn module_scrubber_result_replaces_boot_floor_after_load() {
    assert_eq!(
        apply_module_scrubber("boot-floor", Some(module_redaction)),
        "module-redacted"
    );
}

#[test]
fn unavailable_module_keeps_only_the_boot_floor_result() {
    let floor = scrub_secrets("Bearer secret-value-that-must-not-leak");
    let result = apply_module_scrubber(&floor, None);
    assert!(!result.contains("secret-value-that-must-not-leak"));
}

#[test]
fn scrubs_bearer_token() {
    assert_eq!(
        scrub_secrets("Authorization: Bearer abc123xyz"),
        "Authorization: Bearer [REDACTED]"
    );
}

#[test]
fn scrubs_api_key_assignment() {
    assert_eq!(scrub_secrets("api_key=sk-abc123"), "api_key=[REDACTED]");
}

#[test]
fn scrubs_anthropic_key() {
    assert_eq!(
        scrub_secrets("key: sk-ant-api03-abcdefghijklmnop"),
        "key: [REDACTED]"
    );
}

#[test]
fn scrubs_bare_generic_sk_key() {
    assert_eq!(scrub_secrets("sk-abcdefghijklmnopqrstuvwx"), "[REDACTED]");
}

#[test]
fn scrubs_generic_sk_key_with_separators() {
    // A `_` or `-` mid-suffix must not leave a trailing fragment unredacted.
    assert_eq!(scrub_secrets("sk-abcdefghijklmnopqrst_uv"), "[REDACTED]");
    assert_eq!(scrub_secrets("sk-abcdefghij-klmnopqrst_uv"), "[REDACTED]");
}

#[test]
fn leaves_plain_diagnostics_intact() {
    let msg = "profile 42: derived rate clamp exceeded (max_iterations=8)";
    assert_eq!(scrub_secrets(msg), msg);
}

#[test]
fn scrubs_snake_case_and_json_quoted_tokens() {
    let out = scrub_secrets("cb?access_token=abc123&refresh_token=def456");
    assert!(!out.contains("abc123") && !out.contains("def456"), "{out}");
    let out = scrub_secrets(r#"{"token":"ghi789","api_key":"jkl012"}"#);
    assert!(!out.contains("ghi789") && !out.contains("jkl012"), "{out}");
    assert_eq!(
        scrub_secrets("Authorization: Basic dXNlcjpwYXNzd29yZA=="),
        "Authorization: Basic [REDACTED]"
    );
    // Only named credential keys: a cancellation token is not a secret.
    assert_eq!(
        scrub_secrets("cancellation_token=abc"),
        "cancellation_token=abc"
    );
}

#[test]
fn scrubs_a_quoted_value_through_its_closing_quote() {
    let out = scrub_secrets(r#"{"password":"correct horse, battery; p&ss \"x\" y","user":"a"}"#);
    for leaked in ["horse", "battery", "p&ss", " y\""] {
        assert!(!out.contains(leaked), "{leaked} in {out}");
    }
    assert!(out.ends_with(r#","user":"a"}"#), "{out}");
    // An unterminated quoted value is still redacted.
    let out = scrub_secrets(r#""token":"abc123"#);
    assert!(!out.contains("abc123"), "{out}");
}

#[test]
fn scrubs_a_short_basic_authorization_header() {
    assert_eq!(
        scrub_secrets("Authorization: Basic dTpw"),
        "Authorization: Basic [REDACTED]"
    );
}

#[test]
fn scrubs_any_snake_case_token_key_and_escaped_json() {
    assert_eq!(scrub_secrets("bot_token=b0t12345"), "bot_token=[REDACTED]");
    assert_eq!(
        scrub_secrets(r#"body {\"token\":\"ghi 789\"} sent"#),
        r#"body {\"token\":\"[REDACTED]\"} sent"#
    );
    assert_eq!(
        scrub_secrets(r#"{\"api_key\": \"k1\", \"x\": 1}"#),
        r#"{\"api_key\": \"[REDACTED]\", \"x\": 1}"#
    );
    // Still not a secret.
    assert_eq!(
        scrub_secrets("cancellation_token=abc"),
        "cancellation_token=abc"
    );
}
