//! ANSI / VT escape-sequence stripping.
//!
//! Port of `src/core/text.ts` strip logic.

use once_cell::sync::Lazy;
use regex::Regex;

// CSI: ESC [ … final-byte
static ANSI_CSI: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\x1b\[[0-?]*[ -/]*[@-~]").expect("ansi csi regex"));

// OSC: ESC ] … BEL or ESC backslash
static ANSI_OSC: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)").expect("ansi osc regex"));

// Incomplete CSI at end of string
static ANSI_CSI_INCOMPLETE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\x1b\[[0-?]*[ -/]*$").expect("ansi csi incomplete regex"));

// Incomplete OSC at end of string
static ANSI_OSC_INCOMPLETE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\x1b\][^\x07\x1b]*$").expect("ansi osc incomplete regex"));

// Single-char escapes: ESC followed by @-_
static ANSI_SINGLE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\x1b[@-_]").expect("ansi single regex"));

/// Strip all ANSI/VT escape sequences from `text`.
pub fn strip_ansi(text: &str) -> String {
    let input_len = text.len();
    let s = ANSI_OSC.replace_all(text, "");
    let s = ANSI_CSI.replace_all(&s, "");
    let s = ANSI_OSC_INCOMPLETE.replace_all(&s, "");
    let s = ANSI_CSI_INCOMPLETE.replace_all(&s, "");
    let s = ANSI_SINGLE.replace_all(&s, "");
    // Remove any lone ESC bytes that slipped through
    let out = s.replace('\x1b', "");
    log::trace!(
        "[tokenjuice] strip_ansi in_len={} out_len={}",
        input_len,
        out.len()
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_csi_colour() {
        assert_eq!(strip_ansi("\x1b[31mred\x1b[0m"), "red");
    }

    #[test]
    fn strips_osc() {
        // OSC 8 hyperlink terminated with BEL
        assert_eq!(strip_ansi("\x1b]8;;http://x\x07link\x1b]8;;\x07"), "link");
    }

    #[test]
    fn strips_incomplete_csi_at_end() {
        assert_eq!(strip_ansi("hello\x1b[1"), "hello");
    }

    #[test]
    fn strips_csi_with_letter_terminator() {
        // ESC [ b — `[` starts a CSI sequence, `b` is the final byte → stripped
        assert_eq!(strip_ansi("a\x1b[b"), "a");
    }

    #[test]
    fn strips_single_escape_fe_range() {
        // ESC N — falls in the @-_ range used by single-char escape sequences
        assert_eq!(strip_ansi("a\x1bNb"), "ab");
    }

    #[test]
    fn passthrough_plain() {
        assert_eq!(strip_ansi("plain text"), "plain text");
    }

    #[test]
    fn strips_lone_esc() {
        assert_eq!(strip_ansi("a\x1bb"), "ab");
    }
}
