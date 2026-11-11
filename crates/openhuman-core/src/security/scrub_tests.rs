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

/// Host refusals and harness artifact pointers open with a bracketed label
/// (`[policy-blocked] …`, `[artifact] kind=…`). That is free text, not JSON:
/// it must come back redacted, not withheld as a whole.
#[cfg(feature = "security-module")]
#[test]
fn free_text_opening_with_a_bracketed_label_is_redacted_not_withheld() {
    let token = format!("sk-ant-api03-{}", "0123456789abcdef".repeat(3));
    for label in ["[policy-blocked]", "[artifact]", "  [step-2]"] {
        let input = format!("{label} the command printed {token} and stopped");
        let output = sanitize_text(&input);
        assert!(!output.value.contains(&token), "{label}: {}", output.value);
        assert!(output.report.changed(), "{label}");
        assert_eq!(
            output.value,
            format!("{label} the command printed [REDACTED] and stopped"),
            "{label}"
        );
        // A second pass is a fixed point, as TinySecurity's own Text mode is.
        assert_eq!(sanitize_text(&output.value).value, output.value, "{label}");
    }
    let clean = "[artifact] kind=output path=notes.txt bytes=5\nhello";
    assert_eq!(sanitize_text(clean).value, clean);
    assert!(!sanitize_text(clean).report.changed());
}

/// Text that opens like JSON but is not JSON and carries no label stays
/// withheld: TinySecurity refuses it, and the host must not guess.
#[cfg(feature = "security-module")]
#[test]
fn malformed_json_without_a_label_is_still_withheld() {
    for input in [
        "{\"api_key\": \"\\u0073k-live-truncated",
        "[\"partial\", ",
        "\"unterminated",
        "[sk-live-abcdef0123456789",
        "[ label with space] text",
    ] {
        let output = sanitize_text(input);
        assert_eq!(output.value, "[REDACTED]", "{input}");
        assert!(output.report.changed(), "{input}");
    }
}
