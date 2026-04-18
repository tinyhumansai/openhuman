//! Grapheme-aware terminal-column width calculation.
//!
//! Uses `unicode-segmentation` for grapheme cluster boundaries and
//! `unicode-width` for CJK/emoji double-width detection, mirroring the
//! `Intl.Segmenter`-based logic in the upstream TypeScript.

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthChar;

/// Return the list of user-perceived grapheme clusters in `text`.
pub fn graphemes(text: &str) -> Vec<&str> {
    text.graphemes(true).collect()
}

/// Return the number of grapheme clusters (not bytes or scalar values).
///
/// This is used for character-count limiting (mirrors `countTextChars` in TS).
pub fn count_text_chars(text: &str) -> usize {
    text.graphemes(true).count()
}

/// Return the terminal column width of a single grapheme cluster.
///
/// Emoji are assumed to be 2 columns wide, which matches the upstream TS
/// `graphemeWidth` logic.  The `unicode-width` crate handles most CJK ranges.
fn grapheme_width(segment: &str) -> usize {
    if segment.is_empty() {
        return 0;
    }

    // Emoji: assume width 2 (matches upstream)
    let first_cp = segment.chars().next().unwrap_or('\0');
    if is_emoji(first_cp) {
        return 2;
    }

    // Use unicode-width on the first non-combining code point
    let mut width = 0usize;
    let mut has_visible = false;
    for ch in segment.chars() {
        // Skip zero-width joiners and variation selectors
        if ch == '\u{200D}' || ch == '\u{FE0F}' {
            continue;
        }
        // Skip combining marks (general category M)
        if is_combining_mark(ch) {
            continue;
        }
        let w = UnicodeWidthChar::width(ch).unwrap_or(0);
        width = width.max(w);
        has_visible = true;
    }

    if has_visible {
        width
    } else {
        0
    }
}

/// Return the total terminal column width of `text`.
pub fn count_terminal_cells(text: &str) -> usize {
    text.graphemes(true).map(grapheme_width).sum()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Conservative emoji test covering the main Extended_Pictographic ranges used
/// by the upstream TS code (`/\p{Extended_Pictographic}/u`).
///
/// We use broad ranges to avoid unreachable-pattern warnings in match arms.
fn is_emoji(cp: char) -> bool {
    let c = cp as u32;
    // Misc symbols, dingbats, and the main supplemental emoji blocks
    matches!(c,
        0x2300..=0x27BF |       // Misc technical + arrows + dingbats (broad)
        0x1F300..=0x1FAFF       // All supplemental emoji / symbol blocks
    )
}

/// True for Unicode combining marks (general category M*).
/// We use a simplified range check sufficient for the characters that appear
/// in terminal output.
fn is_combining_mark(ch: char) -> bool {
    let c = ch as u32;
    matches!(c,
        0x0300..=0x036F |   // Combining Diacritical Marks
        0x1AB0..=0x1AFF |   // Combining Diacritical Marks Extended
        0x1DC0..=0x1DFF |   // Combining Diacritical Marks Supplement
        0x20D0..=0x20FF |   // Combining Diacritical Marks for Symbols
        0xFE20..=0xFE2F     // Combining Half Marks
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_char_count() {
        assert_eq!(count_text_chars("hello"), 5);
    }

    #[test]
    fn emoji_char_count_one_grapheme() {
        // U+1F600 GRINNING FACE — 1 grapheme cluster
        assert_eq!(count_text_chars("😀"), 1);
    }

    #[test]
    fn cjk_terminal_width_two_cells() {
        // U+4E2D — one CJK character, should be 2 terminal cells
        assert_eq!(count_terminal_cells("中"), 2);
    }

    #[test]
    fn ascii_terminal_width() {
        assert_eq!(count_terminal_cells("abc"), 3);
    }

    #[test]
    fn graphemes_splits_correctly() {
        let gs = graphemes("abc");
        assert_eq!(gs, vec!["a", "b", "c"]);
    }
}
