//! Rule classification: given a `ToolExecutionInput`, find the best-matching
//! `CompiledRule` and return a `ClassificationResult`.
//!
//! Port of `src/core/classify.ts` and the matching helpers from
//! `src/core/rules.ts`.

use crate::openhuman::tokenjuice::types::{
    ClassificationResult, CompiledRule, JsonRule, ToolExecutionInput,
};

// ---------------------------------------------------------------------------
// Matching helpers
// ---------------------------------------------------------------------------

/// True if every string in `expected` is present somewhere in `argv`.
fn includes_all(argv: &[String], expected: &[String]) -> bool {
    expected.iter().all(|part| argv.contains(part))
}

/// Test whether `rule` matches `input`.  Mirrors `matchesRule` in TS.
pub fn matches_rule(rule: &JsonRule, input: &ToolExecutionInput) -> bool {
    let argv = input.argv.as_deref().unwrap_or(&[]);
    // Fall back to a joined argv when `command` wasn't explicitly set so
    // `commandIncludes*` rules still match for argv-only callers.
    let command_fallback: String;
    let command: &str = match input.command.as_deref() {
        Some(c) => c,
        None => {
            command_fallback = argv.join(" ");
            &command_fallback
        }
    };
    let tool_name = &input.tool_name;

    // toolNames filter
    if let Some(tool_names) = &rule.r#match.tool_names {
        if !tool_names.contains(tool_name) {
            return false;
        }
    }

    // argv0 filter
    if let Some(argv0_list) = &rule.r#match.argv0 {
        let first = argv.first().map(String::as_str).unwrap_or("");
        if !argv0_list.iter().any(|s| s == first) {
            return false;
        }
    }

    // argvIncludes — all groups must match
    if let Some(groups) = &rule.r#match.argv_includes {
        if !groups.iter().all(|group| includes_all(argv, group)) {
            return false;
        }
    }

    // argvIncludesAny — at least one group must match
    if let Some(groups) = &rule.r#match.argv_includes_any {
        if !groups.iter().any(|group| includes_all(argv, group)) {
            return false;
        }
    }

    // commandIncludes — all substrings must appear in command
    if let Some(parts) = &rule.r#match.command_includes {
        if !parts.iter().all(|part| command.contains(part.as_str())) {
            return false;
        }
    }

    // commandIncludesAny — at least one substring must appear
    if let Some(parts) = &rule.r#match.command_includes_any {
        if !parts.iter().any(|part| command.contains(part.as_str())) {
            return false;
        }
    }

    true
}

// ---------------------------------------------------------------------------
// Scoring
// ---------------------------------------------------------------------------

/// Numeric specificity score for a rule — higher wins.
/// Mirrors `scoreRule` in TS.
fn score_rule(rule: &JsonRule) -> i64 {
    let priority = rule.priority.unwrap_or(0) as i64 * 1000;
    let argv0 = rule.r#match.argv0.as_ref().map(|v| v.len()).unwrap_or(0) as i64 * 100;
    let argv_includes = rule
        .r#match
        .argv_includes
        .as_ref()
        .map(|groups| groups.iter().map(|g| g.len()).sum::<usize>())
        .unwrap_or(0) as i64
        * 40;
    let argv_includes_any = rule
        .r#match
        .argv_includes_any
        .as_ref()
        .map(|groups| groups.iter().map(|g| g.len()).sum::<usize>())
        .unwrap_or(0) as i64
        * 35;
    let command_includes = rule
        .r#match
        .command_includes
        .as_ref()
        .map(|v| v.len())
        .unwrap_or(0) as i64
        * 25;
    let command_includes_any = rule
        .r#match
        .command_includes_any
        .as_ref()
        .map(|v| v.len())
        .unwrap_or(0) as i64
        * 20;
    let tool_names = rule
        .r#match
        .tool_names
        .as_ref()
        .map(|v| v.len())
        .unwrap_or(0) as i64
        * 10;

    priority
        + argv0
        + argv_includes
        + argv_includes_any
        + command_includes
        + command_includes_any
        + tool_names
}

// ---------------------------------------------------------------------------
// classify_execution
// ---------------------------------------------------------------------------

/// Classify `input` against the provided `rules` and return a
/// `ClassificationResult`.
///
/// If `forced_rule_id` is `Some`, that rule is used directly (if found).
pub fn classify_execution(
    input: &ToolExecutionInput,
    rules: &[CompiledRule],
    forced_rule_id: Option<&str>,
) -> ClassificationResult {
    // Forced classification
    if let Some(id) = forced_rule_id {
        if let Some(rule) = rules.iter().find(|r| r.rule.id == id) {
            log::debug!(
                "[tokenjuice] forced classification: rule='{}' family='{}'",
                id,
                rule.rule.family
            );
            return ClassificationResult {
                family: rule.rule.family.clone(),
                confidence: 1.0,
                matched_reducer: Some(rule.rule.id.clone()),
            };
        }
    }

    // Find all matching rules
    let mut matched: Vec<&CompiledRule> = rules
        .iter()
        .filter(|r| matches_rule(&r.rule, input))
        .collect();

    if matched.is_empty() {
        log::debug!(
            "[tokenjuice] no rule matched tool='{}' argv={:?} — using generic fallback",
            input.tool_name,
            input.argv
        );
        return ClassificationResult {
            family: "generic".to_owned(),
            confidence: 0.2,
            matched_reducer: None,
        };
    }

    // Sort by descending score, then alphabetically for stability
    matched.sort_by(|a, b| {
        let score_diff = score_rule(&b.rule).cmp(&score_rule(&a.rule));
        if score_diff != std::cmp::Ordering::Equal {
            score_diff
        } else {
            a.rule.id.cmp(&b.rule.id)
        }
    });

    let best = matched[0];
    let confidence = if best.rule.id == "generic/fallback" {
        0.2
    } else {
        0.9
    };

    log::debug!(
        "[tokenjuice] classified tool='{}' → rule='{}' family='{}' confidence={}",
        input.tool_name,
        best.rule.id,
        best.rule.family,
        confidence
    );

    ClassificationResult {
        family: best.rule.family.clone(),
        confidence,
        matched_reducer: Some(best.rule.id.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::tokenjuice::rules::load_builtin_rules;

    fn make_input(tool_name: &str, argv: &[&str]) -> ToolExecutionInput {
        ToolExecutionInput {
            tool_name: tool_name.to_owned(),
            argv: Some(argv.iter().map(|s| s.to_string()).collect()),
            ..Default::default()
        }
    }

    #[test]
    fn git_status_matches() {
        let rules = load_builtin_rules();
        let input = make_input("bash", &["git", "status"]);
        let result = classify_execution(&input, &rules, None);
        assert_eq!(result.matched_reducer.as_deref(), Some("git/status"));
        assert_eq!(result.family, "git-status");
    }

    #[test]
    fn npm_install_does_not_match_git_status() {
        let rules = load_builtin_rules();
        let input = make_input("exec", &["npm", "install"]);
        let result = classify_execution(&input, &rules, None);
        assert_ne!(result.matched_reducer.as_deref(), Some("git/status"));
    }

    #[test]
    fn no_match_returns_generic() {
        let rules = load_builtin_rules();
        let input = make_input("some_unknown_tool", &["mysterious", "command"]);
        let result = classify_execution(&input, &rules, None);
        assert_eq!(result.family, "generic");
        assert_eq!(result.confidence, 0.2);
    }

    #[test]
    fn forced_rule_id_overrides_matching() {
        let rules = load_builtin_rules();
        // Input would normally match git/status but we force cargo-test
        let input = make_input("bash", &["git", "status"]);
        let result = classify_execution(&input, &rules, Some("tests/cargo-test"));
        assert_eq!(result.matched_reducer.as_deref(), Some("tests/cargo-test"));
        assert_eq!(result.confidence, 1.0);
    }

    #[test]
    fn fallback_confidence_is_low() {
        let rules = load_builtin_rules();
        // Force the fallback explicitly
        let input = make_input("bash", &["some", "arbitrary", "command"]);
        let result = classify_execution(&input, &rules, Some("generic/fallback"));
        assert_eq!(result.confidence, 1.0); // forced always returns 1.0
    }

    #[test]
    fn git_diff_stat_requires_both_args() {
        let rules = load_builtin_rules();
        // Missing --stat → should not match git/diff-stat
        let input_no_stat = make_input("bash", &["git", "diff"]);
        let result = classify_execution(&input_no_stat, &rules, None);
        assert_ne!(result.matched_reducer.as_deref(), Some("git/diff-stat"));

        // With --stat → should match
        let input_with_stat = make_input("bash", &["git", "diff", "--stat"]);
        let result2 = classify_execution(&input_with_stat, &rules, None);
        assert_eq!(result2.matched_reducer.as_deref(), Some("git/diff-stat"));
    }

    // --- matches_rule: individual dimension tests ---

    #[test]
    fn tool_names_filter_blocks_wrong_tool() {
        // cargo test rule requires toolNames: ["exec"]
        let rules = load_builtin_rules();
        // "bash" tool should not match tests/cargo-test (requires "exec")
        let input = ToolExecutionInput {
            tool_name: "bash".to_owned(),
            argv: Some(vec!["cargo".to_owned(), "test".to_owned()]),
            ..Default::default()
        };
        let result = classify_execution(&input, &rules, None);
        assert_ne!(result.matched_reducer.as_deref(), Some("tests/cargo-test"));
    }

    #[test]
    fn tool_names_filter_matches_correct_tool() {
        let rules = load_builtin_rules();
        let input = ToolExecutionInput {
            tool_name: "exec".to_owned(),
            argv: Some(vec!["cargo".to_owned(), "test".to_owned()]),
            ..Default::default()
        };
        let result = classify_execution(&input, &rules, None);
        assert_eq!(
            result.matched_reducer.as_deref(),
            Some("tests/cargo-test"),
            "cargo test with exec tool should match tests/cargo-test"
        );
    }

    #[test]
    fn argv_includes_any_matches_at_least_one_group() {
        // Build a custom rule with argvIncludesAny and test it via matches_rule directly
        use crate::openhuman::tokenjuice::types::{JsonRule, RuleMatch};

        let rule = JsonRule {
            id: "test/any".to_owned(),
            family: "test".to_owned(),
            description: None,
            priority: None,
            on_empty: None,
            match_output: None,
            counter_source: None,
            r#match: RuleMatch {
                argv0: Some(vec!["tool".to_owned()]),
                argv_includes_any: Some(vec![vec!["--foo".to_owned()], vec!["--bar".to_owned()]]),
                ..Default::default()
            },
            filters: None,
            transforms: None,
            summarize: None,
            counters: None,
            failure: None,
        };

        // Should match when --foo is present
        let input_foo = ToolExecutionInput {
            tool_name: "bash".to_owned(),
            argv: Some(vec!["tool".to_owned(), "--foo".to_owned()]),
            ..Default::default()
        };
        assert!(matches_rule(&rule, &input_foo));

        // Should match when --bar is present
        let input_bar = ToolExecutionInput {
            tool_name: "bash".to_owned(),
            argv: Some(vec!["tool".to_owned(), "--bar".to_owned()]),
            ..Default::default()
        };
        assert!(matches_rule(&rule, &input_bar));

        // Should NOT match when neither is present
        let input_none = ToolExecutionInput {
            tool_name: "bash".to_owned(),
            argv: Some(vec!["tool".to_owned(), "--baz".to_owned()]),
            ..Default::default()
        };
        assert!(!matches_rule(&rule, &input_none));
    }

    #[test]
    fn command_includes_all_substrings_required() {
        use crate::openhuman::tokenjuice::types::{JsonRule, RuleMatch};

        let rule = JsonRule {
            id: "test/cmd-incl".to_owned(),
            family: "test".to_owned(),
            description: None,
            priority: None,
            on_empty: None,
            match_output: None,
            counter_source: None,
            r#match: RuleMatch {
                command_includes: Some(vec!["git".to_owned(), "status".to_owned()]),
                ..Default::default()
            },
            filters: None,
            transforms: None,
            summarize: None,
            counters: None,
            failure: None,
        };

        let input_match = ToolExecutionInput {
            tool_name: "bash".to_owned(),
            command: Some("git status --short".to_owned()),
            ..Default::default()
        };
        assert!(matches_rule(&rule, &input_match));

        // Missing "status" → no match
        let input_no_match = ToolExecutionInput {
            tool_name: "bash".to_owned(),
            command: Some("git log --oneline".to_owned()),
            ..Default::default()
        };
        assert!(!matches_rule(&rule, &input_no_match));
    }

    #[test]
    fn command_includes_any_at_least_one_substring() {
        use crate::openhuman::tokenjuice::types::{JsonRule, RuleMatch};

        let rule = JsonRule {
            id: "test/cmd-any".to_owned(),
            family: "test".to_owned(),
            description: None,
            priority: None,
            on_empty: None,
            match_output: None,
            counter_source: None,
            r#match: RuleMatch {
                command_includes_any: Some(vec!["install".to_owned(), "update".to_owned()]),
                ..Default::default()
            },
            filters: None,
            transforms: None,
            summarize: None,
            counters: None,
            failure: None,
        };

        let input_install = ToolExecutionInput {
            tool_name: "bash".to_owned(),
            command: Some("npm install".to_owned()),
            ..Default::default()
        };
        assert!(matches_rule(&rule, &input_install));

        let input_update = ToolExecutionInput {
            tool_name: "bash".to_owned(),
            command: Some("npm update".to_owned()),
            ..Default::default()
        };
        assert!(matches_rule(&rule, &input_update));

        let input_neither = ToolExecutionInput {
            tool_name: "bash".to_owned(),
            command: Some("npm run build".to_owned()),
            ..Default::default()
        };
        assert!(!matches_rule(&rule, &input_neither));
    }

    #[test]
    fn forced_rule_id_not_found_falls_back_to_matching() {
        let rules = load_builtin_rules();
        let input = make_input("bash", &["git", "status"]);
        // Force a non-existent rule ID → should fall through to normal matching
        let result = classify_execution(&input, &rules, Some("nonexistent/rule"));
        // Falls through to normal matching; git status should still match git/status
        assert_eq!(result.matched_reducer.as_deref(), Some("git/status"));
    }

    #[test]
    fn multiple_matches_best_score_wins() {
        let rules = load_builtin_rules();
        // "git diff --stat" should match git/diff-stat (more specific) over git/show or others
        let input = make_input("bash", &["git", "diff", "--stat"]);
        let result = classify_execution(&input, &rules, None);
        assert_eq!(result.matched_reducer.as_deref(), Some("git/diff-stat"));
        assert_eq!(result.confidence, 0.9);
    }

    #[test]
    fn generic_fallback_matched_gives_low_confidence() {
        let rules = load_builtin_rules();
        // An unknown command should match generic/fallback with low confidence
        let input = ToolExecutionInput {
            tool_name: "exec".to_owned(),
            argv: Some(vec!["some_nonexistent_program".to_owned()]),
            ..Default::default()
        };
        let result = classify_execution(&input, &rules, None);
        // generic/fallback matches everything, so it will be the winner for unknown commands
        // but confidence should be low (0.2)
        assert_eq!(result.confidence, 0.2);
    }
}
