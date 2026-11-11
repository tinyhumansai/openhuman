use super::{sanitize_json, sanitize_text};

#[cfg(feature = "security-module")]
#[test]
fn oversized_text_is_withheld_with_no_secret_fallback() {
    let secret = "unique-secret-value";
    let input = format!("{secret}{}", "x".repeat(64 * 1024));
    let output = sanitize_text(&input);
    assert!(output.report.changed());
    assert!(!output.value.contains(secret));
}

#[cfg(feature = "security-module")]
#[test]
fn oversized_json_is_replaced_by_a_shape_only_marker() {
    let value = serde_json::json!({"password": format!("{}", "s".repeat(64 * 1024))});
    let output = sanitize_json(&value);
    assert!(output.report.changed());
    assert!(output.value.to_string().contains("arguments unavailable"));
    assert!(!output.value.to_string().contains(&"s".repeat(64)));
}

/// With security disabled nothing is redacted: text and JSON come back as given,
/// whatever their size, and the report says nothing changed.
fn assert_values_pass_through() {
    let secret = "credential-that-is-not-redacted";
    let output = sanitize_text(secret);
    assert_eq!(output.value, secret);
    assert!(!output.report.changed());

    let large = format!("{secret}{}", "x".repeat(64 * 1024));
    assert_eq!(sanitize_text(&large).value, large);

    let value = serde_json::json!({"password": secret, "n": 1});
    let output = sanitize_json(&value);
    assert_eq!(output.value, value);
    assert!(!output.report.changed());
}

#[cfg(not(feature = "security-module"))]
#[test]
fn a_build_without_the_module_returns_raw_text() {
    assert_values_pass_through();
}

#[cfg(feature = "security-module")]
#[test]
fn security_configured_off_returns_raw_text() {
    let _off = crate::security::switch::off_for_this_thread();
    assert_values_pass_through();
}
