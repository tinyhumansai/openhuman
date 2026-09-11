use super::*;
use serde_json::json;

#[test]
fn backtick_run_reports_longest_sequence() {
    assert_eq!(longest_backtick_run("a``b````c"), 4);
    assert_eq!(longest_backtick_run("plain"), 0);
}

#[test]
fn fenced_json_can_be_embedded_in_prose() {
    assert_eq!(
        extract_fenced_json_block("before ```json\n{\"ok\":true}\n``` after"),
        Some(json!({"ok": true}))
    );
}

#[test]
fn balanced_json_ignores_delimiters_and_escapes_inside_strings() {
    assert_eq!(
        extract_balanced_json(r#"before {"text":"} and \"quoted\"","ok":true} after"#),
        Some(json!({"text": "} and \"quoted\"", "ok": true}))
    );
    assert_eq!(extract_balanced_json("no structured value"), None);
}
