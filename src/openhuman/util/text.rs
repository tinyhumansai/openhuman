//! Text and string-boundary helpers.
//!
//! UTF-8-safe truncation, char-boundary rounding, and the non-leaky
//! provenance tag used by the cross-chat context block.

/// Render a short, non-leaky provenance tag for a session/thread id.
///
/// The channel-side `session_id` is typically a JSON blob
/// (`{"client_id": "...", "thread_id": "..."}`); rendering it verbatim
/// in a model prompt or log line would leak the raw `client_id` /
/// socket UUID. Hash the input with `DefaultHasher` and emit only the
/// low 32 bits as `chat:xxxxxxxx` — short, stable per id, and not
/// reversible to the original blob.
///
/// Used by the cross-chat context block (issue #1505) so the prompt
/// can attribute hits without surfacing raw identifiers.
pub fn provenance_tag(session_id: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    session_id.hash(&mut hasher);
    let h = hasher.finish();
    format!("chat:{:08x}", (h & 0xFFFF_FFFF) as u32)
}

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

/// Return a prefix of `s` whose byte length is at most `max_bytes`, backing up
/// to the nearest UTF-8 character boundary when `max_bytes` falls in the middle
/// of a multi-byte character.
pub fn utf8_safe_prefix_at_byte_boundary(s: &str, max_bytes: usize) -> &str {
    &s[..floor_char_boundary(s, max_bytes)]
}

/// Round a byte index UP to the nearest UTF-8 character boundary.
pub fn ceil_char_boundary(s: &str, index: usize) -> usize {
    if index >= s.len() {
        return s.len();
    }
    let mut start = index;
    while start < s.len() && !s.is_char_boundary(start) {
        start += 1;
    }
    start
}

#[cfg(test)]
#[path = "text_tests.rs"]
mod tests;
