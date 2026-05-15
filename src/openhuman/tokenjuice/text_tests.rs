//! Additional unit tests for the `tokenjuice::text` sub-modules.
//!
//! Focuses on coverage gaps identified in test-map.md Pick 5
//! ("TokenJuice Rust port for tool-output compaction"):
//! - `strip_ansi` with multi-byte / emoji text (grapheme safety).
//! - `dedupe_adjacent` additional edge cases.
//! - `clamp_text_middle` grapheme-safe split — never breaks inside a
//!   multi-byte codepoint or multi-scalar grapheme cluster.
//! - 3-layer overlay precedence: project > user > builtin.
//! - Rule loader gracefully handles invalid regex (diagnostic, no panic).

use crate::openhuman::tokenjuice::rules::loader::{load_rules, LoadRuleOptions};
use crate::openhuman::tokenjuice::text::width::{count_text_chars, graphemes};
use crate::openhuman::tokenjuice::text::{clamp_text_middle, dedupe_adjacent, strip_ansi};
use crate::openhuman::tokenjuice::types::RuleOrigin;

// ── strip_ansi — multi-byte / emoji safety ───────────────────────────────────

#[test]
fn strip_ansi_leaves_multibyte_cjk_intact() {
    // CJK characters must pass through completely even when preceded by ANSI.
    let input = "\x1b[32m中文\x1b[0m";
    assert_eq!(strip_ansi(input), "中文");
}

#[test]
fn strip_ansi_leaves_emoji_intact() {
    // Emoji must survive stripping.
    let input = "\x1b[1m😀 hello\x1b[0m";
    assert_eq!(strip_ansi(input), "😀 hello");
}

#[test]
fn strip_ansi_multi_byte_only_no_escapes() {
    // When there are no ANSI codes, multi-byte text is returned unchanged.
    let text = "こんにちは";
    assert_eq!(strip_ansi(text), text);
}

#[test]
fn strip_ansi_zwj_emoji_sequence_preserved() {
    // ZWJ sequences (family emoji) must not be mangled.
    let fam = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}"; // family emoji
    let colored = format!("\x1b[31m{fam}\x1b[0m");
    let stripped = strip_ansi(&colored);
    assert_eq!(stripped, fam, "ZWJ sequence must survive ANSI stripping");
}

#[test]
fn strip_ansi_mixed_scripts_preserved() {
    // Arabic, CJK, Latin, emoji all in one string with ANSI wrappers.
    let input = "\x1b[33mعربي 中文 hello 🌍\x1b[0m";
    let stripped = strip_ansi(input);
    assert_eq!(stripped, "عربي 中文 hello 🌍");
}

#[test]
fn strip_ansi_empty_string() {
    assert_eq!(strip_ansi(""), "");
}

#[test]
fn strip_ansi_only_escape_sequences() {
    // If the entire string is escape sequences, the result should be empty.
    let all_ansi = "\x1b[0m\x1b[1m\x1b[31m";
    assert_eq!(strip_ansi(all_ansi), "");
}

// ── dedupe_adjacent ───────────────────────────────────────────────────────────

fn strs(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

#[test]
fn dedupe_adjacent_collapses_run_of_identical_lines() {
    let lines = strs(&["a", "a", "a", "b"]);
    let out = dedupe_adjacent(&lines);
    assert_eq!(out, strs(&["a", "b"]));
}

#[test]
fn dedupe_adjacent_preserves_non_adjacent_duplicates() {
    // Same value reappearing after a different line must NOT be collapsed.
    let lines = strs(&["a", "b", "a"]);
    let out = dedupe_adjacent(&lines);
    assert_eq!(out, strs(&["a", "b", "a"]));
}

#[test]
fn dedupe_adjacent_single_element_is_unchanged() {
    let lines = strs(&["only"]);
    let out = dedupe_adjacent(&lines);
    assert_eq!(out, strs(&["only"]));
}

#[test]
fn dedupe_adjacent_all_identical_collapses_to_one() {
    let lines = strs(&["x", "x", "x", "x", "x"]);
    let out = dedupe_adjacent(&lines);
    assert_eq!(out, strs(&["x"]));
}

#[test]
fn dedupe_adjacent_empty_lines_are_deduplicated() {
    // Adjacent blank lines must also be collapsed.
    let lines = strs(&["a", "", "", "b", "", "c"]);
    let out = dedupe_adjacent(&lines);
    assert_eq!(out, strs(&["a", "", "b", "", "c"]));
}

#[test]
fn dedupe_adjacent_multibyte_lines_collapsed() {
    let lines = strs(&["日本語", "日本語", "日本語"]);
    let out = dedupe_adjacent(&lines);
    assert_eq!(out, strs(&["日本語"]));
}

// ── clamp_text_middle — grapheme-safe middle truncation ───────────────────────

/// Assert that `clamp_text_middle` never splits inside a multi-byte
/// grapheme: every byte of the output must decode to valid UTF-8, and
/// every character in the output must appear as a whole grapheme in the
/// source string.
#[test]
fn clamp_text_middle_output_is_valid_utf8() {
    // A long string of CJK characters (each is 3 bytes in UTF-8).
    // String is always valid UTF-8, so a tautological from_utf8 check would
    // not actually verify the grapheme-safety contract. Instead, assert that
    // every grapheme in the clamped output also exists as a complete grapheme
    // in the source (i.e. no partial cluster fragments leaked through).
    let cjk: String = "中文字符测试！".repeat(50);
    let clamped = clamp_text_middle(&cjk, 30);
    let source_graphemes: std::collections::HashSet<&str> = graphemes(&cjk).into_iter().collect();
    for g in graphemes(&clamped) {
        // The omission marker is the only legitimate non-source content.
        if g == "·"
            || g.chars().all(|c| {
                c.is_ascii_punctuation() || c.is_ascii_whitespace() || c.is_ascii_alphanumeric()
            })
        {
            continue;
        }
        assert!(
            source_graphemes.contains(g),
            "grapheme {g:?} in clamp output is not a whole grapheme of the source"
        );
    }
}

#[test]
fn clamp_text_middle_does_not_split_emoji_grapheme() {
    // Each emoji is 4 bytes; a naïve byte split could land in the middle.
    // Verify boundary correctness by counting graphemes — the clamp must
    // never produce a partial codepoint or partial grapheme.
    let emojis: String = "😀".repeat(100);
    let clamped = clamp_text_middle(&emojis, 20);
    // Every non-marker grapheme in the output must equal "😀" (source has
    // exactly one distinct grapheme). A partial split would leave a
    // replacement char or a stray surrogate-equivalent sequence.
    for g in graphemes(&clamped) {
        let only_ascii = g.chars().all(|c| c.is_ascii());
        assert!(
            g == "😀" || only_ascii,
            "unexpected grapheme {g:?} — partial emoji split detected"
        );
    }
}

#[test]
fn clamp_text_middle_short_text_is_passthrough() {
    // Strings shorter than max_chars are returned verbatim.
    let text = "hello 世界 🌍";
    let clamped = clamp_text_middle(text, 200);
    assert_eq!(clamped, text);
}

#[test]
fn clamp_text_middle_inserts_omission_marker() {
    let long_text = "line\n".repeat(200);
    let clamped = clamp_text_middle(&long_text, 100);
    assert!(
        clamped.contains("omitted"),
        "middle clamp must contain omission marker, got: {}",
        &clamped[..clamped.len().min(120)]
    );
}

#[test]
fn clamp_text_middle_grapheme_count_respects_limit() {
    // The result should not exceed max_chars + marker length substantially.
    // We use a lenient bound (2× marker overhead) rather than an exact count.
    let long_text = "あいうえお\n".repeat(200); // multi-byte lines
    let max = 100usize;
    let clamped = clamp_text_middle(&long_text, max);
    let grapheme_count = count_text_chars(&clamped);
    // Allow up to 2× max to accommodate the omission marker.
    assert!(
        grapheme_count <= max * 2,
        "clamped grapheme count {grapheme_count} exceeds 2×max ({max})"
    );
}

#[test]
fn clamp_text_middle_zwj_sequence_not_split() {
    // ZWJ family emoji repeated; each base char is 4 bytes, ZWJ is 3 bytes.
    // Assert no partial ZWJ family fragments survive in the output: any
    // grapheme containing the ZWJ codepoint must be the full family unit.
    let zwj_unit = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}"; // family
    let long: String = (zwj_unit.to_owned() + "\n").repeat(100);
    let clamped = clamp_text_middle(&long, 30);
    for g in graphemes(&clamped) {
        if g.contains('\u{200D}')
            || g.chars()
                .any(|c| matches!(c, '\u{1F468}' | '\u{1F469}' | '\u{1F467}'))
        {
            assert_eq!(
                g, zwj_unit,
                "clamp produced partial ZWJ cluster {g:?}; expected the full family unit"
            );
        }
    }
}

// ── grapheme helper round-trip ────────────────────────────────────────────────

#[test]
fn graphemes_clusters_match_count_text_chars() {
    let mixed = "hello 中文 😀 emoji";
    let gs = graphemes(mixed);
    assert_eq!(gs.len(), count_text_chars(mixed));
}

// ── 3-layer overlay precedence ────────────────────────────────────────────────

#[test]
fn three_layer_overlay_project_beats_user_beats_builtin() {
    // Create temporary dirs for user and project layers.
    let user_dir = tempfile::tempdir().expect("user tempdir");
    let project_dir = tempfile::tempdir().expect("project tempdir");

    // User overrides the builtin `git/status` rule with family "user-family".
    std::fs::write(
        user_dir.path().join("gs.json"),
        r#"{"id":"git/status","family":"user-family","match":{}}"#,
    )
    .unwrap();

    // Project overrides the same rule with family "project-family" (highest priority).
    std::fs::write(
        project_dir.path().join("gs.json"),
        r#"{"id":"git/status","family":"project-family","match":{}}"#,
    )
    .unwrap();

    let opts = LoadRuleOptions {
        user_rules_dir: Some(user_dir.path().to_owned()),
        project_rules_dir: Some(project_dir.path().to_owned()),
        ..Default::default()
    };
    let rules = load_rules(&opts);
    let gs = rules
        .iter()
        .find(|r| r.rule.id == "git/status")
        .expect("git/status rule must be present");

    // Project must win over user and builtin.
    assert_eq!(
        gs.rule.family, "project-family",
        "project layer must override user and builtin layers"
    );
    assert_eq!(
        gs.source,
        RuleOrigin::Project,
        "winning rule must be sourced from Project"
    );
}

#[test]
fn two_layer_overlay_user_beats_builtin() {
    let user_dir = tempfile::tempdir().expect("user tempdir");

    std::fs::write(
        user_dir.path().join("gs.json"),
        r#"{"id":"git/status","family":"user-only","match":{}}"#,
    )
    .unwrap();

    let opts = LoadRuleOptions {
        user_rules_dir: Some(user_dir.path().to_owned()),
        exclude_project: true,
        ..Default::default()
    };
    let rules = load_rules(&opts);
    let gs = rules
        .iter()
        .find(|r| r.rule.id == "git/status")
        .expect("git/status rule");

    assert_eq!(gs.rule.family, "user-only");
    assert_eq!(gs.source, RuleOrigin::User);
}

#[test]
fn builtin_rule_present_when_no_overrides() {
    let opts = LoadRuleOptions {
        exclude_user: true,
        exclude_project: true,
        ..Default::default()
    };
    let rules = load_rules(&opts);
    let gs = rules.iter().find(|r| r.rule.id == "git/status");
    assert!(gs.is_some(), "builtin git/status rule must be present");
    assert_eq!(
        gs.unwrap().source,
        RuleOrigin::Builtin,
        "with no overlay layers, source must be Builtin"
    );
}

// ── rule loader — invalid regex gracefully handled ────────────────────────────

#[test]
fn invalid_regex_in_skip_patterns_does_not_panic() {
    use crate::openhuman::tokenjuice::rules::compiler::compile_rule;
    use crate::openhuman::tokenjuice::types::{JsonRule, RuleFilters, RuleMatch};

    let rule = JsonRule {
        id: "test/bad-skip".to_owned(),
        family: "test".to_owned(),
        description: None,
        priority: None,
        on_empty: None,
        match_output: None,
        counter_source: None,
        r#match: RuleMatch::default(),
        filters: Some(RuleFilters {
            skip_patterns: Some(vec![
                "[invalid-regex".to_owned(), // deliberately malformed
                "valid_pattern".to_owned(),  // valid one after the bad one
            ]),
            keep_patterns: None,
        }),
        transforms: None,
        summarize: None,
        counters: None,
        failure: None,
    };

    // Must not panic; invalid regex is silently dropped.
    let compiled = compile_rule(
        rule,
        RuleOrigin::Builtin,
        "builtin:test/bad-skip".to_owned(),
    );

    // The invalid pattern is dropped; the valid one should be retained.
    assert_eq!(
        compiled.compiled.skip_patterns.len(),
        1,
        "invalid regex must be dropped; valid pattern must be retained"
    );
}

#[test]
fn all_invalid_regex_in_skip_patterns_leaves_empty_vec() {
    use crate::openhuman::tokenjuice::rules::compiler::compile_rule;
    use crate::openhuman::tokenjuice::types::{JsonRule, RuleFilters, RuleMatch};

    let rule = JsonRule {
        id: "test/all-bad".to_owned(),
        family: "test".to_owned(),
        description: None,
        priority: None,
        on_empty: None,
        match_output: None,
        counter_source: None,
        r#match: RuleMatch::default(),
        filters: Some(RuleFilters {
            skip_patterns: Some(vec![
                "[bad1".to_owned(),
                "(bad2".to_owned(),
                "{bad3".to_owned(),
            ]),
            keep_patterns: None,
        }),
        transforms: None,
        summarize: None,
        counters: None,
        failure: None,
    };

    let compiled = compile_rule(rule, RuleOrigin::Builtin, "builtin:test/all-bad".to_owned());
    assert!(
        compiled.compiled.skip_patterns.is_empty(),
        "all invalid skip patterns must produce an empty vec"
    );
}

#[test]
fn invalid_regex_loaded_from_disk_is_skipped_not_fatal() {
    // Write a rule JSON with an invalid skip_pattern to a temp project dir.
    let dir = tempfile::tempdir().expect("tempdir");
    let bad_rule = r#"{
        "id": "test/disk-bad-regex",
        "family": "test",
        "match": {},
        "filters": {
            "skipPatterns": ["[invalid"]
        }
    }"#;
    std::fs::write(dir.path().join("bad_regex.json"), bad_rule).unwrap();

    // Also add a valid rule to ensure loading continues normally.
    let good_rule = r#"{"id":"test/disk-good","family":"test","match":{}}"#;
    std::fs::write(dir.path().join("good.json"), good_rule).unwrap();

    let opts = LoadRuleOptions {
        project_rules_dir: Some(dir.path().to_owned()),
        exclude_user: true,
        ..Default::default()
    };

    // Must not panic; bad regex → compiled rule with no skip patterns.
    let rules = load_rules(&opts);
    // The valid rule is still present.
    assert!(
        rules.iter().any(|r| r.rule.id == "test/disk-good"),
        "valid rule must still load alongside the bad-regex rule"
    );
    // The bad-regex rule must still load — but with the invalid skip pattern
    // dropped so the rule itself is non-fatal. Asserting presence avoids a
    // false positive where the rule is silently dropped entirely.
    let bad = rules
        .iter()
        .find(|r| r.rule.id == "test/disk-bad-regex")
        .expect("bad-regex rule must still load");
    assert!(
        bad.compiled.skip_patterns.is_empty(),
        "bad-regex rule must have empty compiled skip_patterns"
    );
}
