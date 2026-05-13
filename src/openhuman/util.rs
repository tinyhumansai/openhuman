//! Utility functions for `OpenHuman`.
//!
//! This module contains reusable helper functions used across the codebase.

/// Truncate a string to at most `max_chars` characters, appending "..." if truncated.
///
/// This function safely handles multi-byte UTF-8 characters (emoji, CJK, accented characters)
/// by using character boundaries instead of byte indices.
///
/// # Arguments
/// * `s` - The string to truncate
/// * `max_chars` - Maximum number of characters to keep (excluding "...")
///
/// # Returns
/// * Original string if length <= `max_chars`
/// * Truncated string with "..." appended if length > `max_chars`
///
/// # Examples
/// ```
/// use openhuman_core::openhuman::util::truncate_with_ellipsis;
///
/// // ASCII string - no truncation needed
/// assert_eq!(truncate_with_ellipsis("hello", 10), "hello");
///
/// // ASCII string - truncation needed
/// assert_eq!(truncate_with_ellipsis("hello world", 5), "hello...");
///
/// // Multi-byte UTF-8 (emoji) - safe truncation
/// assert_eq!(truncate_with_ellipsis("Hello 🦀 World", 8), "Hello 🦀...");
/// assert_eq!(truncate_with_ellipsis("😀😀😀😀", 2), "😀😀...");
///
/// // Empty string
/// assert_eq!(truncate_with_ellipsis("", 10), "");
/// ```
pub fn truncate_with_ellipsis(s: &str, max_chars: usize) -> String {
    truncate_with_suffix(s, max_chars, "...")
}

/// Truncate a string to at most `max_chars` characters, appending `suffix` if truncated.
pub fn truncate_with_suffix(s: &str, max_chars: usize, suffix: &str) -> String {
    match s.char_indices().nth(max_chars) {
        Some((idx, _)) => {
            let truncated = &s[..idx];
            // Trim trailing whitespace for cleaner output
            format!("{}{}", truncated.trim_end(), suffix)
        }
        None => s.to_string(),
    }
}

/// Truncate a string to at most `max_bytes` bytes, appending a single-character
/// ellipsis `…` (3 bytes) if truncated. The returned string's total byte
/// length will never exceed `max_bytes`.
pub fn truncate_at_byte_boundary(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let ellipsis = "…";
    let ellipsis_len = ellipsis.len();
    if max_bytes < ellipsis_len {
        return String::new();
    }
    let mut end = max_bytes - ellipsis_len;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{}", &s[..end], ellipsis)
}

/// Round a byte index DOWN to the nearest UTF-8 character boundary.
pub fn floor_char_boundary(s: &str, index: usize) -> usize {
    if index >= s.len() {
        return s.len();
    }
    let mut end = index;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    end
}

/// Utility enum for handling optional values.
pub enum MaybeSet<T> {
    Set(T),
    Unset,
    Null,
}

#[cfg(test)]
mod tests {
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
        let s = "这是一个测试消息用来触发崩溃的中文"; // 21 characters
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
    fn test_truncate_with_suffix() {
        let s = "Hello World";
        assert_eq!(truncate_with_suffix(s, 5, "!!!"), "Hello!!!");
        assert_eq!(truncate_with_suffix(s, 20, "!!!"), "Hello World");
    }
}
