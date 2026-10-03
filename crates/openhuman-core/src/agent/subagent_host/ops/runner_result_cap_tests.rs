use super::apply_max_result_chars;

#[test]
fn max_result_chars_none_is_noop() {
    // No cap → output untouched (the `agent_memory` default).
    let mut out = "hello world".to_string();
    apply_max_result_chars(&mut out, None, "worker");
    assert_eq!(out, "hello world");
}

#[test]
fn max_result_chars_under_cap_is_noop() {
    let mut out = "short".to_string();
    apply_max_result_chars(&mut out, Some(100), "worker");
    assert_eq!(out, "short");
}

#[test]
fn max_result_chars_over_cap_truncates_with_marker() {
    let mut out = "x".repeat(50);
    apply_max_result_chars(&mut out, Some(10), "worker");
    assert!(out.starts_with(&"x".repeat(10)), "{out}");
    assert!(out.ends_with("[...truncated]"), "{out}");
    // 10 kept chars + the marker, and shorter than the 50-char original.
    assert!(out.chars().count() < 50, "{out}");
}

#[test]
fn max_result_chars_truncates_on_char_boundary_for_multibyte() {
    // Cap lands mid-run of multi-byte chars; must not panic or split a char.
    let mut out = "é".repeat(20); // each 'é' is 2 bytes
    apply_max_result_chars(&mut out, Some(5), "worker");
    assert!(out.starts_with(&"é".repeat(5)), "{out}");
    assert!(out.ends_with("[...truncated]"), "{out}");
}
