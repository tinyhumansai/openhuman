use super::*;

#[test]
fn test_truncate_ascii_no_truncation() {
    // ASCII string shorter than limit - no change
    assert_eq!(truncate_with_ellipsis("hello", 10), "hello");
    assert_eq!(truncate_with_ellipsis("hello world", 50), "hello world");
}

#[test]
fn test_truncate_ascii_with_truncation() {
    // ASCII string longer than limit - truncates
    assert_eq!(truncate_with_ellipsis("hello world", 5), "hello...");
    assert_eq!(
        truncate_with_ellipsis("This is a long message", 10),
        "This is a..."
    );
}

#[test]
fn test_truncate_empty_string() {
    assert_eq!(truncate_with_ellipsis("", 10), "");
}

#[test]
fn test_truncate_at_exact_boundary() {
    // String exactly at boundary - no truncation
    assert_eq!(truncate_with_ellipsis("hello", 5), "hello");
}

#[test]
fn test_truncate_emoji_single() {
    // Single emoji (4 bytes) - should not panic
    let s = "🦀";
    assert_eq!(truncate_with_ellipsis(s, 10), s);
    assert_eq!(truncate_with_ellipsis(s, 1), s);
}

#[test]
fn test_truncate_emoji_multiple() {
    // Multiple emoji - safe truncation at character boundary
    let s = "😀😀😀😀"; // 4 emoji, each 4 bytes = 16 bytes total
    assert_eq!(truncate_with_ellipsis(s, 2), "😀😀...");
    assert_eq!(truncate_with_ellipsis(s, 3), "😀😀😀...");
}

#[test]
fn test_truncate_mixed_ascii_emoji() {
    // Mixed ASCII and emoji
    assert_eq!(truncate_with_ellipsis("Hello 🦀 World", 8), "Hello 🦀...");
    assert_eq!(truncate_with_ellipsis("Hi 😊", 10), "Hi 😊");
}

#[test]
fn test_truncate_cjk_characters() {
    // CJK characters (Chinese - each is 3 bytes)
    let s = "这是一个测试消息用来触发崩溃 of the 中文"; // 21 characters
    let result = truncate_with_ellipsis(s, 16);
    assert!(result.ends_with("..."));
    assert!(result.is_char_boundary(result.len() - 1));
}

#[test]
fn test_truncate_accented_characters() {
    // Accented characters (2 bytes each in UTF-8)
    let s = "café résumé naïve";
    assert_eq!(truncate_with_ellipsis(s, 10), "café résum...");
}

#[test]
fn test_truncate_unicode_edge_case() {
    // Mix of 1-byte, 2-byte, 3-byte, and 4-byte characters
    let s = "aé你好🦀"; // 1 + 1 + 2 + 2 + 4 bytes = 10 bytes, 5 chars
    assert_eq!(truncate_with_ellipsis(s, 3), "aé你...");
}

#[test]
fn test_truncate_long_string() {
    // Long ASCII string
    let s = "a".repeat(200);
    let result = truncate_with_ellipsis(&s, 50);
    assert_eq!(result.len(), 53); // 50 + "..."
    assert!(result.ends_with("..."));
}

#[test]
fn test_truncate_zero_max_chars() {
    // Edge case: max_chars = 0
    assert_eq!(truncate_with_ellipsis("hello", 0), "...");
}

#[test]
fn test_truncate_at_byte_boundary() {
    let s = "Hello 🦀 World"; // 16 bytes total. "🦀" is 4 bytes at index 6-9.
                              // No truncation
    assert_eq!(truncate_at_byte_boundary(s, 16), s);
    assert_eq!(truncate_at_byte_boundary(s, 20), s);

    // Truncate at index 11 (the space after 🦀)
    // max_bytes = 14, ellipsis = 3 bytes, target end = 11.
    assert_eq!(truncate_at_byte_boundary(s, 14), "Hello 🦀 …");

    // Truncate mid-emoji (byte 8 is mid-🦀)
    // max_bytes = 9, ellipsis = 3 bytes, target end = 6.
    // should back up to index 6, add "…" (3 bytes) -> 9 bytes total
    let truncated = truncate_at_byte_boundary(s, 9);
    assert_eq!(truncated, "Hello …");
    assert!(truncated.len() <= 9);

    // Very small budget
    assert_eq!(truncate_at_byte_boundary("abc", 2), "");
    assert_eq!(truncate_at_byte_boundary("abc", 3), "abc");
}

#[test]
fn test_floor_char_boundary() {
    let s = "A🦀C";
    assert_eq!(floor_char_boundary(s, 0), 0);
    assert_eq!(floor_char_boundary(s, 1), 1); // After 'A'
    assert_eq!(floor_char_boundary(s, 2), 1); // Mid-🦀
    assert_eq!(floor_char_boundary(s, 3), 1); // Mid-🦀
    assert_eq!(floor_char_boundary(s, 4), 1); // Mid-🦀
    assert_eq!(floor_char_boundary(s, 5), 5); // After '🦀'
    assert_eq!(floor_char_boundary(s, 6), 6); // After 'C'
    assert_eq!(floor_char_boundary(s, 100), 6);
}

#[test]
fn test_utf8_safe_prefix_at_byte_boundary() {
    let s = format!("{}{}tail", "a".repeat(79), "魔");
    assert_eq!(utf8_safe_prefix_at_byte_boundary(&s, 80), "a".repeat(79));
    assert_eq!(utf8_safe_prefix_at_byte_boundary(&s, s.len()), s);
    assert_eq!(
        utf8_safe_prefix_at_byte_boundary("ascii preview", 5),
        "ascii"
    );
    assert_eq!(utf8_safe_prefix_at_byte_boundary("short", 80), "short");

    for cap in [30, 40, 80, 200, 500] {
        let preview = format!("{}{}tail", "a".repeat(cap - 1), "界");
        let truncated = utf8_safe_prefix_at_byte_boundary(&preview, cap);
        assert_eq!(truncated, "a".repeat(cap - 1));
        assert!(preview.is_char_boundary(truncated.len()));
    }
}

#[test]
fn test_ceil_char_boundary() {
    let s = "A🦀C";
    assert_eq!(ceil_char_boundary(s, 0), 0);
    assert_eq!(ceil_char_boundary(s, 1), 1); // After 'A'
    assert_eq!(ceil_char_boundary(s, 2), 5); // Mid-🦀
    assert_eq!(ceil_char_boundary(s, 3), 5); // Mid-🦀
    assert_eq!(ceil_char_boundary(s, 4), 5); // Mid-🦀
    assert_eq!(ceil_char_boundary(s, 5), 5); // After '🦀'
    assert_eq!(ceil_char_boundary(s, 6), 6); // After 'C'
    assert_eq!(ceil_char_boundary(s, 100), 6);
}

#[test]
fn test_truncate_with_suffix() {
    let s = "Hello World";
    assert_eq!(truncate_with_suffix(s, 5, "!!!"), "Hello!!!");
    assert_eq!(truncate_with_suffix(s, 20, "!!!"), "Hello World");
}
