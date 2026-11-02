use super::{sanitize_json, sanitize_text};

#[test]
fn oversized_text_is_withheld_with_no_secret_fallback() {
    let secret = "unique-secret-value";
    let input = format!("{secret}{}", "x".repeat(64 * 1024));
    let output = sanitize_text(&input);
    assert!(output.report.changed());
    assert!(!output.value.contains(secret));
}

#[test]
fn oversized_json_is_replaced_by_a_shape_only_marker() {
    let value = serde_json::json!({"password": format!("{}", "s".repeat(64 * 1024))});
    let output = sanitize_json(&value);
    assert!(output.report.changed());
    assert!(output.value.to_string().contains("arguments unavailable"));
    assert!(!output.value.to_string().contains(&"s".repeat(64)));
}

#[cfg(not(feature = "security-module"))]
#[test]
fn disabled_module_build_never_returns_raw_text() {
    let secret = "credential-that-must-not-leak";
    let output = sanitize_text(secret);
    assert!(output.report.changed());
    assert!(!output.value.contains(secret));
}
