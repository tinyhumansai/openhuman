use super::detect_tab_artifact_suffix;
use super::is_low_quality_suggestion;

#[test]
fn low_quality_rejects_too_short() {
    assert!(is_low_quality_suggestion("", ""));
    assert!(is_low_quality_suggestion("a", "hello "));
}

#[test]
fn low_quality_rejects_pure_punct() {
    assert!(is_low_quality_suggestion("...", "hello"));
    assert!(is_low_quality_suggestion("  -- ", "hello"));
}

#[test]
fn low_quality_rejects_echo_of_tail() {
    assert!(is_low_quality_suggestion("world", "hello world"));
}

#[test]
fn low_quality_accepts_new_content() {
    assert!(!is_low_quality_suggestion(" world", "hello"));
    assert!(!is_low_quality_suggestion("tomorrow", "see you "));
}

#[test]
fn detects_literal_tab_suffix() {
    assert_eq!(
        detect_tab_artifact_suffix("hello world", "hello world\t"),
        1
    );
}

#[test]
fn detects_space_indentation_suffix() {
    assert_eq!(
        detect_tab_artifact_suffix("hello world", "hello world    "),
        4
    );
}

#[test]
fn returns_zero_when_context_does_not_match_expected_tail() {
    assert_eq!(
        detect_tab_artifact_suffix("hello world", "different    "),
        0
    );
}

#[test]
fn returns_zero_when_no_tab_like_suffix_present() {
    assert_eq!(detect_tab_artifact_suffix("hello world", "hello worldx"), 0);
}
