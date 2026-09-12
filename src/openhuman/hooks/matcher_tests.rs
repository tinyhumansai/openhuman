use super::*;

#[test]
fn absent_matcher_selects_everything() {
    assert!(matches(None, Some("anything")));
    assert!(matches(None, None));
    assert!(matches(Some("  "), Some("anything")));
}

#[test]
fn wildcard_selects_subjectless_events() {
    assert!(matches(Some("*"), None));
}

#[test]
fn named_matcher_skips_subjectless_events() {
    assert!(!matches(Some("shell"), None));
}

#[test]
fn literal_name_is_case_insensitive_and_exact() {
    assert!(matches(Some("Shell"), Some("shell")));
    assert!(!matches(Some("Shell"), Some("shell_extra")));
}

#[test]
fn alternation_matches_any_branch() {
    assert!(matches(Some("Read|Write|Shell"), Some("write")));
    assert!(!matches(Some("Read|Write"), Some("shell")));
}

#[test]
fn mcp_prefix_is_stripped() {
    assert!(matches(Some("MCP:search_docs"), Some("search_docs")));
}

#[test]
fn regex_matcher_applies_to_command_lines() {
    assert!(matches(Some("^rm "), Some("rm -rf /tmp/x")));
    assert!(!matches(Some("^rm "), Some("echo rm ")));
}

#[test]
fn invalid_regex_matches_nothing() {
    assert!(!matches(Some("["), Some("anything")));
}
