use super::*;

#[test]
fn redact_masks_short_values_entirely() {
    assert_eq!(redact("abcd"), "***");
    assert_eq!(redact(""), "***");
}

#[test]
fn redact_keeps_only_four_leading_chars_of_long_values() {
    let out = redact("sk-live-1234567890");
    assert!(out.starts_with("sk-l"));
    assert!(out.ends_with("***"));
    assert!(!out.contains("567890"));
}
