use super::*;

// ── Guard clause: empty / whitespace input short-circuits ────
//
// The post-guard code (clipboard / enigo / AppleScript) needs a
// display and a real system event loop, so coverage of those paths
// below `insert_text`'s trim-guard is only achievable in an
// end-to-end integration environment. Units here pin the logic
// that IS deterministic in a headless test process.

#[test]
fn empty_text_is_noop_and_succeeds() {
    assert!(insert_text("", None).is_ok());
}

#[test]
fn whitespace_only_skips_insertion_and_succeeds() {
    assert!(insert_text("   ", None).is_ok());
}

#[test]
fn newlines_and_tabs_only_also_treated_as_empty() {
    // `trim()` strips any Unicode whitespace — the skip branch must
    // fire for pure `\t` and `\n` buffers too, not just spaces.
    assert!(insert_text("\n\n", None).is_ok());
    assert!(insert_text("\t  \n", Some("any-app")).is_ok());
}

#[test]
fn paste_modifier_is_platform_correct() {
    let key = paste_modifier_key();
    if cfg!(target_os = "macos") {
        assert!(matches!(key, Key::Meta));
    } else {
        assert!(matches!(key, Key::Control));
    }
}

#[test]
fn constants_match_openwhispr_timings() {
    // Lock in the OpenWhispr-derived delays so nobody silently
    // shortens them (would race the target app's paste handler).
    assert_eq!(PASTE_DELAY, Duration::from_millis(120));
    assert_eq!(CLIPBOARD_RESTORE_DELAY, Duration::from_millis(450));
}

// ── AppleScript string escaping (macOS-only) ─────────────────

#[cfg(target_os = "macos")]
#[test]
fn escape_applescript_string_escapes_backslash_and_quote() {
    assert_eq!(escape_applescript_string("plain"), "plain");
    assert_eq!(escape_applescript_string(r#"a"b"#), r#"a\"b"#);
    assert_eq!(escape_applescript_string(r"a\b"), r"a\\b");
    // Backslash must be escaped BEFORE quotes so the order of
    // substitutions doesn't double-escape already-escaped quotes.
    assert_eq!(escape_applescript_string(r#"\"mix"#), r#"\\\"mix"#);
}

#[cfg(target_os = "macos")]
#[test]
fn escape_applescript_string_is_idempotent_on_benign_input() {
    for s in ["", "App Name", "Safari", "Sub-App 2", "123"] {
        assert_eq!(escape_applescript_string(s), s);
    }
}

// ── Focus-restore error path (macOS-only) ────────────────────

#[cfg(target_os = "macos")]
#[test]
fn restore_focus_to_app_errors_on_bogus_app_name() {
    // `osascript` returns a non-zero exit when the target app
    // cannot be activated, so we expect the helper to surface
    // that as an Err. This exercises the error-formatting branch.
    let err = restore_focus_to_app("__definitely_no_such_app_abcxyz__")
        .expect_err("bogus app should not activate");
    assert!(
        err.contains("failed to restore focus"),
        "expected focus-restore prefix in error, got: {err}"
    );
}
