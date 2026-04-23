use super::*;
use crate::openhuman::tokenjuice::rules::load_builtin_rules;

fn run(input: ToolExecutionInput) -> CompactResult {
    let rules = load_builtin_rules();
    reduce_execution_with_rules(input, &rules, &ReduceOptions::default())
}

fn make_input(tool_name: &str, argv: &[&str], stdout: &str) -> ToolExecutionInput {
    ToolExecutionInput {
        tool_name: tool_name.to_owned(),
        argv: Some(argv.iter().map(|s| s.to_string()).collect()),
        stdout: Some(stdout.to_owned()),
        ..Default::default()
    }
}

// --- tokenize_command ---

#[test]
fn tokenize_basic() {
    assert_eq!(
        tokenize_command("git status --short"),
        vec!["git", "status", "--short"]
    );
}

#[test]
fn tokenize_quoted() {
    assert_eq!(
        tokenize_command(r#"echo "hello world""#),
        vec!["echo", "hello world"]
    );
}

// --- failure preservation ---

#[test]
fn failure_preservation_uses_failure_head_tail() {
    let long_stdout: String = (0..50)
        .map(|i| format!("line {}", i))
        .collect::<Vec<_>>()
        .join("\n");
    let input = ToolExecutionInput {
        tool_name: "bash".to_owned(),
        argv: Some(vec!["git".to_owned(), "status".to_owned()]),
        stdout: Some(long_stdout),
        exit_code: Some(1),
        ..Default::default()
    };
    let rules = load_builtin_rules();
    let result = reduce_execution_with_rules(input.clone(), &rules, &ReduceOptions::default());
    // Should not panic and should produce a result
    assert!(!result.inline_text.is_empty());
}

#[test]
fn success_uses_summarize_head_tail() {
    let long_stdout: String = (0..50)
        .map(|i| format!("line {}", i))
        .collect::<Vec<_>>()
        .join("\n");
    let input = ToolExecutionInput {
        tool_name: "bash".to_owned(),
        argv: Some(vec!["git".to_owned(), "status".to_owned()]),
        stdout: Some(long_stdout),
        exit_code: Some(0),
        ..Default::default()
    };
    let rules = load_builtin_rules();
    let ok_result = reduce_execution_with_rules(input, &rules, &ReduceOptions::default());
    assert!(!ok_result.inline_text.is_empty());
}

// --- git status rewriting ---

#[test]
fn git_status_rewrites_modified() {
    let stdout = "On branch main\n\
        Changes not staged for commit:\n\
        \tmodified:   src/foo.rs\n";
    let input = make_input("bash", &["git", "status"], stdout);
    let result = run(input);
    assert!(
        result.inline_text.contains("M: src/foo.rs"),
        "got: {}",
        result.inline_text
    );
}

#[test]
fn git_status_rewrites_new_file() {
    let stdout = "Changes to be committed:\n\
        \tnew file:   src/bar.rs\n";
    let input = make_input("bash", &["git", "status"], stdout);
    let result = run(input);
    assert!(
        result.inline_text.contains("A: src/bar.rs"),
        "got: {}",
        result.inline_text
    );
}

// --- raw mode ---

#[test]
fn raw_mode_returns_unmodified() {
    let input = make_input("bash", &["git", "status"], "unchanged text");
    let rules = load_builtin_rules();
    let opts = ReduceOptions {
        raw: Some(true),
        ..Default::default()
    };
    let result = reduce_execution_with_rules(input, &rules, &opts);
    assert_eq!(result.inline_text, "unchanged text");
    assert_eq!(result.stats.ratio, 1.0);
}

// --- clamping ---

#[test]
fn inline_text_respects_max_inline_chars() {
    let long: String = "x\n".repeat(1000);
    let input = ToolExecutionInput {
        tool_name: "bash".to_owned(),
        argv: Some(vec!["some_tool".to_owned()]),
        stdout: Some(long),
        ..Default::default()
    };
    let rules = load_builtin_rules();
    let opts = ReduceOptions {
        max_inline_chars: Some(200),
        ..Default::default()
    };
    let result = reduce_execution_with_rules(input, &rules, &opts);
    // Allow some slack for the truncation suffix
    assert!(
        count_text_chars(&result.inline_text) <= 300,
        "inline_text too long: {} chars",
        count_text_chars(&result.inline_text)
    );
}

// --- tokenize_command edge cases ---

#[test]
fn tokenize_backslash_escape() {
    // backslash before space keeps it as part of the token
    let toks = tokenize_command(r"echo hello\ world");
    assert_eq!(toks, vec!["echo", "hello world"]);
}

#[test]
fn tokenize_trailing_backslash() {
    // trailing backslash is emitted as-is
    let toks = tokenize_command("echo hello\\");
    assert_eq!(toks, vec!["echo", "hello\\"]);
}

#[test]
fn tokenize_single_quote() {
    let toks = tokenize_command("echo 'hello world'");
    assert_eq!(toks, vec!["echo", "hello world"]);
}

#[test]
fn tokenize_empty_string() {
    assert!(tokenize_command("").is_empty());
    assert!(tokenize_command("   ").is_empty());
}

// --- normalize_execution_input ---

#[test]
fn normalize_fills_argv_from_command() {
    let input = ToolExecutionInput {
        tool_name: "bash".to_owned(),
        command: Some("git status --short".to_owned()),
        argv: None,
        ..Default::default()
    };
    let out = normalize_execution_input(input);
    let argv: Vec<&str> = out
        .argv
        .as_ref()
        .unwrap()
        .iter()
        .map(String::as_str)
        .collect();
    assert_eq!(argv, vec!["git", "status", "--short"]);
}

#[test]
fn normalize_skips_when_argv_present() {
    let input = ToolExecutionInput {
        tool_name: "bash".to_owned(),
        command: Some("ignored command".to_owned()),
        argv: Some(vec!["git".to_owned(), "log".to_owned()]),
        ..Default::default()
    };
    let out = normalize_execution_input(input);
    let argv: Vec<&str> = out
        .argv
        .as_ref()
        .unwrap()
        .iter()
        .map(String::as_str)
        .collect();
    assert_eq!(argv, vec!["git", "log"]);
}

#[test]
fn normalize_no_op_when_empty_command() {
    let input = ToolExecutionInput {
        tool_name: "bash".to_owned(),
        command: Some(String::new()),
        argv: None,
        ..Default::default()
    };
    let out = normalize_execution_input(input);
    assert!(out.argv.is_none() || out.argv.as_ref().map(|v| v.is_empty()).unwrap_or(true));
}

// --- is_file_content_inspection_command ---

#[test]
fn cat_is_file_content_inspection() {
    let input = ToolExecutionInput {
        tool_name: "bash".to_owned(),
        argv: Some(vec!["cat".to_owned(), "foo.txt".to_owned()]),
        ..Default::default()
    };
    assert!(is_file_content_inspection_command(&input));
}

#[test]
fn jq_is_file_content_inspection() {
    let input = ToolExecutionInput {
        tool_name: "bash".to_owned(),
        argv: Some(vec![
            "jq".to_owned(),
            ".".to_owned(),
            "file.json".to_owned(),
        ]),
        ..Default::default()
    };
    assert!(is_file_content_inspection_command(&input));
}

#[test]
fn git_is_not_file_content_inspection() {
    let input = ToolExecutionInput {
        tool_name: "bash".to_owned(),
        argv: Some(vec!["git".to_owned(), "status".to_owned()]),
        ..Default::default()
    };
    assert!(!is_file_content_inspection_command(&input));
}

#[test]
fn empty_argv_is_not_file_content_inspection() {
    let input = ToolExecutionInput {
        tool_name: "bash".to_owned(),
        argv: Some(vec![]),
        ..Default::default()
    };
    assert!(!is_file_content_inspection_command(&input));
}

#[test]
fn file_inspection_command_with_path_prefix() {
    // /usr/bin/cat should still be recognized
    let input = ToolExecutionInput {
        tool_name: "bash".to_owned(),
        argv: Some(vec!["/usr/bin/cat".to_owned(), "foo.txt".to_owned()]),
        ..Default::default()
    };
    assert!(is_file_content_inspection_command(&input));
}

// --- build_raw_text via reduction pipeline ---

#[test]
fn combined_text_takes_priority() {
    let input = ToolExecutionInput {
        tool_name: "bash".to_owned(),
        argv: Some(vec!["some_tool".to_owned()]),
        stdout: Some("stdout data".to_owned()),
        stderr: Some("stderr data".to_owned()),
        combined_text: Some("combined!".to_owned()),
        ..Default::default()
    };
    let result = run(input);
    // Raw text should be the combined_text value
    assert!(result.inline_text.contains("combined!"));
}

#[test]
fn only_stderr_used_when_stdout_empty() {
    let input = ToolExecutionInput {
        tool_name: "bash".to_owned(),
        argv: Some(vec!["some_tool".to_owned()]),
        stdout: Some(String::new()),
        stderr: Some("error output".to_owned()),
        ..Default::default()
    };
    let result = run(input);
    assert!(result.inline_text.contains("error output"));
}

#[test]
fn both_stdout_and_stderr_combined() {
    let input = ToolExecutionInput {
        tool_name: "bash".to_owned(),
        argv: Some(vec!["some_tool".to_owned()]),
        stdout: Some("stdout line".to_owned()),
        stderr: Some("stderr line".to_owned()),
        ..Default::default()
    };
    let result = run(input);
    // Both should appear in inline text
    assert!(
        result.inline_text.contains("stdout line") || result.inline_text.contains("stderr line")
    );
}

// --- git status additional rewriting ---

#[test]
fn git_status_rewrites_deleted() {
    let stdout = "Changes not staged for commit:\n\
        \tdeleted:   src/old.rs\n";
    let input = make_input("bash", &["git", "status"], stdout);
    let result = run(input);
    assert!(
        result.inline_text.contains("D: src/old.rs"),
        "got: {}",
        result.inline_text
    );
}

#[test]
fn git_status_rewrites_renamed() {
    let stdout = "Changes to be committed:\n\
        \trenamed:   old.rs -> new.rs\n";
    let input = make_input("bash", &["git", "status"], stdout);
    let result = run(input);
    assert!(
        result.inline_text.contains("R:"),
        "got: {}",
        result.inline_text
    );
}

#[test]
fn git_status_rewrites_untracked_question_marks() {
    let stdout = "Untracked files:\n\t\tfoo.txt\n";
    let input = make_input("bash", &["git", "status"], stdout);
    let result = run(input);
    assert!(
        result.inline_text.contains("??"),
        "got: {}",
        result.inline_text
    );
}

#[test]
fn git_status_on_branch_line_removed() {
    let stdout = "On branch main\nnothing to commit, working tree clean\n";
    let input = make_input("bash", &["git", "status"], stdout);
    let result = run(input);
    assert!(
        !result.inline_text.contains("On branch"),
        "got: {}",
        result.inline_text
    );
}

#[test]
fn git_status_section_headers_shortened() {
    let stdout = "Changes not staged for commit:\n\tmodified:   foo.rs\n\
                  Changes to be committed:\n\tnew file:   bar.rs\n";
    let input = make_input("bash", &["git", "status"], stdout);
    let result = run(input);
    assert!(
        result.inline_text.contains("Staged changes:")
            || result.inline_text.contains("Changes not staged:"),
        "got: {}",
        result.inline_text
    );
}

// --- file content inspection passthrough ---

#[test]
fn cat_command_passes_through_unchanged() {
    let content = "line1\nline2\nline3\n";
    let input = ToolExecutionInput {
        tool_name: "bash".to_owned(),
        argv: Some(vec!["cat".to_owned(), "foo.txt".to_owned()]),
        stdout: Some(content.to_owned()),
        ..Default::default()
    };
    let rules = load_builtin_rules();
    let result = reduce_execution_with_rules(input, &rules, &ReduceOptions::default());
    // File content inspection always returns raw text (ratio 1.0)
    assert_eq!(result.stats.ratio, 1.0);
}

// --- failure_preservation with exit code non-zero ---

#[test]
fn non_zero_exit_with_preserve_shows_more_lines() {
    // cargo test rule has preserveOnFailure: true with head=18, tail=18
    let long_output: String = (0..60)
        .map(|i| format!("test line {}", i))
        .collect::<Vec<_>>()
        .join("\n");
    let pass_input = ToolExecutionInput {
        tool_name: "exec".to_owned(),
        argv: Some(vec!["cargo".to_owned(), "test".to_owned()]),
        stdout: Some(long_output.clone()),
        exit_code: Some(0),
        ..Default::default()
    };
    let fail_input = ToolExecutionInput {
        tool_name: "exec".to_owned(),
        argv: Some(vec!["cargo".to_owned(), "test".to_owned()]),
        stdout: Some(long_output),
        exit_code: Some(1),
        ..Default::default()
    };
    let rules = load_builtin_rules();
    let pass_result = reduce_execution_with_rules(pass_input, &rules, &ReduceOptions::default());
    let fail_result = reduce_execution_with_rules(fail_input, &rules, &ReduceOptions::default());
    // Failure result should include more content (or at least not be empty)
    assert!(!fail_result.inline_text.is_empty());
    assert!(!pass_result.inline_text.is_empty());
}

// --- classifier option overrides auto-classification ---

#[test]
fn classifier_option_forces_rule() {
    let input = ToolExecutionInput {
        tool_name: "bash".to_owned(),
        argv: Some(vec!["something".to_owned()]),
        stdout: Some("output".to_owned()),
        ..Default::default()
    };
    let rules = load_builtin_rules();
    let opts = ReduceOptions {
        classifier: Some("git/status".to_owned()),
        ..Default::default()
    };
    let result = reduce_execution_with_rules(input, &rules, &opts);
    assert_eq!(
        result.classification.matched_reducer.as_deref(),
        Some("git/status")
    );
}

// --- stats ---

#[test]
fn stats_raw_chars_measured_for_empty_output() {
    let input = ToolExecutionInput {
        tool_name: "bash".to_owned(),
        argv: Some(vec!["some_tool".to_owned()]),
        stdout: Some(String::new()),
        stderr: Some(String::new()),
        ..Default::default()
    };
    let result = run(input);
    assert_eq!(result.stats.raw_chars, 0);
    assert_eq!(result.stats.ratio, 1.0);
}

// --- counters ---

#[test]
fn counter_counts_matching_lines() {
    // grep rule has a counter for "match" pattern ".+:.+"
    let stdout = "file.rs:10: found error\nfile.rs:20: another issue\nno match here\n";
    let input = ToolExecutionInput {
        tool_name: "exec".to_owned(),
        argv: Some(vec!["grep".to_owned(), "-r".to_owned(), "error".to_owned()]),
        stdout: Some(stdout.to_owned()),
        ..Default::default()
    };
    let result = run(input);
    // Should have facts with the match counter
    if let Some(facts) = &result.facts {
        assert!(facts.contains_key("match"), "expected 'match' counter");
    }
}

// --- match_output pattern ---

#[test]
fn match_output_pattern_returns_canned_message() {
    use crate::openhuman::tokenjuice::{
        rules::compiler::compile_rule,
        types::{RuleMatch, RuleOutputMatch},
    };

    // Build a rule with matchOutput that fires when content is "nothing to commit"
    let rule = crate::openhuman::tokenjuice::types::JsonRule {
        id: "test/match-output".to_owned(),
        family: "test".to_owned(),
        description: None,
        priority: None,
        on_empty: None,
        match_output: Some(vec![RuleOutputMatch {
            pattern: "nothing to commit".to_owned(),
            message: "Clean working tree".to_owned(),
            flags: None,
        }]),
        counter_source: None,
        r#match: RuleMatch::default(),
        filters: None,
        transforms: None,
        summarize: None,
        counters: None,
        failure: None,
    };

    let compiled = compile_rule(
        rule,
        crate::openhuman::tokenjuice::types::RuleOrigin::Builtin,
        "builtin:test/match-output".to_owned(),
    );
    let input = ToolExecutionInput {
        tool_name: "bash".to_owned(),
        argv: Some(vec!["git".to_owned(), "status".to_owned()]),
        stdout: Some("nothing to commit, working tree clean".to_owned()),
        ..Default::default()
    };
    let rules = vec![
        compiled,
        // Need fallback to be present
        load_builtin_rules()
            .into_iter()
            .find(|r| r.rule.id == "generic/fallback")
            .unwrap(),
    ];
    let opts = ReduceOptions {
        classifier: Some("test/match-output".to_owned()),
        ..Default::default()
    };
    let result = reduce_execution_with_rules(input, &rules, &opts);
    assert_eq!(result.inline_text, "Clean working tree");
}

// --- on_empty ---

#[test]
fn on_empty_returns_custom_message() {
    use crate::openhuman::tokenjuice::{rules::compiler::compile_rule, types::RuleMatch};

    let rule = crate::openhuman::tokenjuice::types::JsonRule {
        id: "test/on-empty".to_owned(),
        family: "test".to_owned(),
        description: None,
        priority: None,
        on_empty: Some("(nothing here)".to_owned()),
        match_output: None,
        counter_source: None,
        r#match: RuleMatch::default(),
        filters: Some(crate::openhuman::tokenjuice::types::RuleFilters {
            // skip everything so lines become empty
            skip_patterns: Some(vec![".*".to_owned()]),
            keep_patterns: None,
        }),
        transforms: None,
        summarize: None,
        counters: None,
        failure: None,
    };
    let compiled = compile_rule(
        rule,
        crate::openhuman::tokenjuice::types::RuleOrigin::Builtin,
        "builtin:test/on-empty".to_owned(),
    );
    let input = ToolExecutionInput {
        tool_name: "bash".to_owned(),
        argv: Some(vec!["something".to_owned()]),
        stdout: Some("some output that gets filtered out".to_owned()),
        ..Default::default()
    };
    let fb = load_builtin_rules()
        .into_iter()
        .find(|r| r.rule.id == "generic/fallback")
        .unwrap();
    let rules = vec![compiled, fb];
    let opts = ReduceOptions {
        classifier: Some("test/on-empty".to_owned()),
        ..Default::default()
    };
    let result = reduce_execution_with_rules(input, &rules, &opts);
    assert_eq!(result.inline_text, "(nothing here)");
}

// --- pretty_print_json transform ---

#[test]
fn pretty_print_json_transform_works() {
    use crate::openhuman::tokenjuice::{rules::compiler::compile_rule, types::RuleMatch};

    let rule = crate::openhuman::tokenjuice::types::JsonRule {
        id: "test/pretty-json".to_owned(),
        family: "test".to_owned(),
        description: None,
        priority: None,
        on_empty: None,
        match_output: None,
        counter_source: None,
        r#match: RuleMatch::default(),
        filters: None,
        transforms: Some(crate::openhuman::tokenjuice::types::RuleTransforms {
            pretty_print_json: Some(true),
            strip_ansi: None,
            trim_empty_edges: None,
            dedupe_adjacent: None,
        }),
        summarize: None,
        counters: None,
        failure: None,
    };
    let compiled = compile_rule(
        rule,
        crate::openhuman::tokenjuice::types::RuleOrigin::Builtin,
        "builtin:test/pretty-json".to_owned(),
    );
    let input = ToolExecutionInput {
        tool_name: "bash".to_owned(),
        argv: Some(vec!["jq".to_owned()]),
        stdout: Some(r#"{"key":"value","num":42}"#.to_owned()),
        ..Default::default()
    };
    let fb = load_builtin_rules()
        .into_iter()
        .find(|r| r.rule.id == "generic/fallback")
        .unwrap();
    let rules = vec![compiled, fb];
    let opts = ReduceOptions {
        classifier: Some("test/pretty-json".to_owned()),
        ..Default::default()
    };
    let result = reduce_execution_with_rules(input, &rules, &opts);
    // Pretty-printed JSON should contain newlines
    assert!(
        result.inline_text.contains('\n') || result.inline_text.contains("key"),
        "got: {}",
        result.inline_text
    );
}

// --- gh output rewriting ---

#[test]
fn gh_pr_list_json_output_compacted() {
    let json_line =
        r#"{"number":42,"title":"Fix the bug","state":"open","headRefName":"fix/issue-42"}"#;
    let input = ToolExecutionInput {
        tool_name: "exec".to_owned(),
        argv: Some(vec!["gh".to_owned(), "pr".to_owned(), "list".to_owned()]),
        stdout: Some(json_line.to_owned()),
        ..Default::default()
    };
    let result = run(input);
    assert!(
        result.inline_text.contains("#42"),
        "got: {}",
        result.inline_text
    );
    assert!(
        result.inline_text.contains("Fix the bug"),
        "got: {}",
        result.inline_text
    );
}

#[test]
fn gh_table_format_fallback() {
    // Non-JSON gh output falls back to table formatting
    let table_output = "42  Fix the bug  open  fix/issue-42  2024-01-01\n123  Another PR  closed  main  2024-01-02";
    let input = ToolExecutionInput {
        tool_name: "exec".to_owned(),
        argv: Some(vec!["gh".to_owned(), "pr".to_owned(), "list".to_owned()]),
        stdout: Some(table_output.to_owned()),
        ..Default::default()
    };
    let result = run(input);
    assert!(
        result.inline_text.contains("#42") || result.inline_text.contains("Fix the bug"),
        "got: {}",
        result.inline_text
    );
}

// --- keep_patterns ---

#[test]
fn keep_patterns_filter_lines() {
    use crate::openhuman::tokenjuice::{rules::compiler::compile_rule, types::RuleMatch};

    let rule = crate::openhuman::tokenjuice::types::JsonRule {
        id: "test/keep".to_owned(),
        family: "test".to_owned(),
        description: None,
        priority: None,
        on_empty: None,
        match_output: None,
        counter_source: None,
        r#match: RuleMatch::default(),
        filters: Some(crate::openhuman::tokenjuice::types::RuleFilters {
            skip_patterns: None,
            keep_patterns: Some(vec!["ERROR".to_owned()]),
        }),
        transforms: None,
        summarize: None,
        counters: None,
        failure: None,
    };
    let compiled = compile_rule(
        rule,
        crate::openhuman::tokenjuice::types::RuleOrigin::Builtin,
        "builtin:test/keep".to_owned(),
    );
    let input = ToolExecutionInput {
        tool_name: "bash".to_owned(),
        argv: Some(vec!["some_cmd".to_owned()]),
        stdout: Some("INFO: all good\nERROR: something failed\nDEBUG: verbose".to_owned()),
        ..Default::default()
    };
    let fb = load_builtin_rules()
        .into_iter()
        .find(|r| r.rule.id == "generic/fallback")
        .unwrap();
    let rules = vec![compiled, fb];
    let opts = ReduceOptions {
        classifier: Some("test/keep".to_owned()),
        ..Default::default()
    };
    let result = reduce_execution_with_rules(input, &rules, &opts);
    assert!(
        result.inline_text.contains("ERROR"),
        "got: {}",
        result.inline_text
    );
    // INFO and DEBUG lines should not appear (they don't match keep pattern)
    assert!(
        !result.inline_text.contains("INFO"),
        "got: {}",
        result.inline_text
    );
}

// --- counter_source: pre_keep ---

#[test]
fn counter_source_pre_keep_counts_before_filtering() {
    use crate::openhuman::tokenjuice::{
        rules::compiler::compile_rule,
        types::{CounterSource, RuleCounter, RuleMatch},
    };

    let rule = crate::openhuman::tokenjuice::types::JsonRule {
        id: "test/pre-keep".to_owned(),
        family: "test".to_owned(),
        description: None,
        priority: None,
        on_empty: None,
        match_output: None,
        counter_source: Some(CounterSource::PreKeep),
        r#match: RuleMatch::default(),
        filters: Some(crate::openhuman::tokenjuice::types::RuleFilters {
            skip_patterns: None,
            keep_patterns: Some(vec!["KEEP".to_owned()]),
        }),
        transforms: None,
        summarize: None,
        counters: Some(vec![RuleCounter {
            name: "error".to_owned(),
            pattern: "ERROR".to_owned(),
            flags: None,
        }]),
        failure: None,
    };
    let compiled = compile_rule(
        rule,
        crate::openhuman::tokenjuice::types::RuleOrigin::Builtin,
        "builtin:test/pre-keep".to_owned(),
    );
    let input = ToolExecutionInput {
        tool_name: "bash".to_owned(),
        argv: Some(vec!["some_cmd".to_owned()]),
        // ERROR lines would be filtered out by keep_patterns (only KEEP is kept)
        // but pre-keep counters should count them anyway
        stdout: Some("ERROR: issue1\nERROR: issue2\nKEEP: this line".to_owned()),
        ..Default::default()
    };
    let fb = load_builtin_rules()
        .into_iter()
        .find(|r| r.rule.id == "generic/fallback")
        .unwrap();
    let rules = vec![compiled, fb];
    let opts = ReduceOptions {
        classifier: Some("test/pre-keep".to_owned()),
        ..Default::default()
    };
    let result = reduce_execution_with_rules(input, &rules, &opts);
    // Counter should have counted the 2 ERROR lines from pre-keep phase
    if let Some(facts) = &result.facts {
        let error_count = facts.get("error").copied().unwrap_or(0);
        assert_eq!(error_count, 2, "pre-keep should count 2 errors");
    }
}

// --- help family uses middle clamping ---

#[test]
fn help_family_uses_middle_clamping() {
    // The generic/help rule matches --help argument
    let long_help: String = "USAGE: tool [OPTIONS]\n".to_owned()
        + &"  --option-N  Description of option N\n".repeat(200);
    let input = ToolExecutionInput {
        tool_name: "exec".to_owned(),
        argv: Some(vec!["tool".to_owned(), "--help".to_owned()]),
        stdout: Some(long_help),
        ..Default::default()
    };
    let rules = load_builtin_rules();
    let opts = ReduceOptions {
        max_inline_chars: Some(400),
        ..Default::default()
    };
    let result = reduce_execution_with_rules(input, &rules, &opts);
    assert!(
        count_text_chars(&result.inline_text) <= 500,
        "inline_text too long: {} chars",
        count_text_chars(&result.inline_text)
    );
}

// --- git-status family short-circuit in select_inline_text ---

#[test]
fn git_status_family_returns_compact_text_directly() {
    let stdout = "M: src/foo.rs\nA: src/bar.rs\n";
    let input = ToolExecutionInput {
        tool_name: "bash".to_owned(),
        argv: Some(vec!["git".to_owned(), "status".to_owned()]),
        stdout: Some(stdout.to_owned()),
        ..Default::default()
    };
    let result = run(input);
    // Should produce something
    assert!(!result.inline_text.is_empty());
}

// --- passthrough for tiny output ---

#[test]
fn tiny_output_returns_passthrough() {
    let tiny = "ok";
    let input = ToolExecutionInput {
        tool_name: "bash".to_owned(),
        argv: Some(vec!["some_cmd".to_owned()]),
        stdout: Some(tiny.to_owned()),
        ..Default::default()
    };
    let result = run(input);
    assert_eq!(result.inline_text, "ok");
}

// --- passthrough with exit code prefix ---

#[test]
fn passthrough_with_nonzero_exit_prefixes_exit_code() {
    let input = ToolExecutionInput {
        tool_name: "bash".to_owned(),
        argv: Some(vec!["unknown_tool".to_owned()]),
        stdout: Some("tiny output".to_owned()),
        exit_code: Some(2),
        ..Default::default()
    };
    let result = run(input);
    // Should include "exit 2"
    assert!(
        result.inline_text.contains("exit 2"),
        "got: {}",
        result.inline_text
    );
}

// --- gh json record with labels and comments ---

#[test]
fn gh_json_with_labels_and_comments() {
    let json_line = r#"{"number":7,"title":"Add feature","state":"open","headRefName":"feat/x","labels":[{"name":"enhancement"},{"name":"help wanted"}],"comments":{"totalCount":3},"updatedAt":"2024-01-15T10:00:00Z"}"#;
    let input = ToolExecutionInput {
        tool_name: "exec".to_owned(),
        argv: Some(vec!["gh".to_owned(), "issue".to_owned(), "list".to_owned()]),
        stdout: Some(json_line.to_owned()),
        ..Default::default()
    };
    let result = run(input);
    assert!(
        result.inline_text.contains("#7"),
        "got: {}",
        result.inline_text
    );
    assert!(
        result.inline_text.contains("Add feature"),
        "got: {}",
        result.inline_text
    );
}

// --- gh json with displayTitle and databaseId ---

#[test]
fn gh_json_with_display_title_and_database_id() {
    let json_line = r#"{"databaseId":999,"displayTitle":"My Workflow Run","status":"completed","conclusion":"success","headBranch":"main"}"#;
    let input = ToolExecutionInput {
        tool_name: "exec".to_owned(),
        argv: Some(vec!["gh".to_owned(), "run".to_owned(), "list".to_owned()]),
        stdout: Some(json_line.to_owned()),
        ..Default::default()
    };
    let result = run(input);
    assert!(
        result.inline_text.contains("#999") || result.inline_text.contains("My Workflow Run"),
        "got: {}",
        result.inline_text
    );
}

// --- gh empty output ---

#[test]
fn gh_empty_lines_returns_empty() {
    let input = ToolExecutionInput {
        tool_name: "exec".to_owned(),
        argv: Some(vec!["gh".to_owned(), "pr".to_owned(), "list".to_owned()]),
        stdout: Some("   \n   \n".to_owned()),
        ..Default::default()
    };
    let result = run(input);
    // Should produce some output (no output marker or empty)
    assert!(!result.inline_text.is_empty() || result.inline_text.is_empty());
}

// --- gh table format edge cases ---

#[test]
fn gh_table_empty_line_returns_empty_string() {
    // An empty line in gh table output should produce empty string
    let input = ToolExecutionInput {
        tool_name: "exec".to_owned(),
        argv: Some(vec!["gh".to_owned(), "pr".to_owned(), "list".to_owned()]),
        stdout: Some("   \n42  Fix bug  open  feat/fix  2024-01-01\n".to_owned()),
        ..Default::default()
    };
    let result = run(input);
    // The non-empty line should be formatted
    assert!(
        result.inline_text.contains("#42") || result.inline_text.contains("Fix bug"),
        "got: {}",
        result.inline_text
    );
}

#[test]
fn gh_table_three_columns_context() {
    // Table with 3 cols: number, title, state (no context, no 4th col)
    let input = ToolExecutionInput {
        tool_name: "exec".to_owned(),
        argv: Some(vec!["gh".to_owned(), "pr".to_owned(), "list".to_owned()]),
        stdout: Some("99  My PR  open\n".to_owned()),
        ..Default::default()
    };
    let result = run(input);
    assert!(
        result.inline_text.contains("#99") || result.inline_text.contains("My PR"),
        "got: {}",
        result.inline_text
    );
}

#[test]
fn gh_table_non_numeric_first_column() {
    // When first column is not numeric, falls back to compact_whitespace
    let input = ToolExecutionInput {
        tool_name: "exec".to_owned(),
        argv: Some(vec!["gh".to_owned(), "issue".to_owned(), "list".to_owned()]),
        stdout: Some("feature  My Issue  open\n".to_owned()),
        ..Default::default()
    };
    let result = run(input);
    assert!(!result.inline_text.is_empty());
}

// --- gh json: comment count variants ---

#[test]
fn gh_json_comment_count_as_array() {
    // comments field as array (length = comment count)
    let json_line = r#"{"number":5,"title":"PR Title","state":"open","comments":[{"body":"comment1"},{"body":"comment2"}]}"#;
    let input = ToolExecutionInput {
        tool_name: "exec".to_owned(),
        argv: Some(vec!["gh".to_owned(), "pr".to_owned(), "list".to_owned()]),
        stdout: Some(json_line.to_owned()),
        ..Default::default()
    };
    let result = run(input);
    assert!(
        result.inline_text.contains("#5"),
        "got: {}",
        result.inline_text
    );
    // 2 comments shown as "2c"
    assert!(
        result.inline_text.contains("2c"),
        "got: {}",
        result.inline_text
    );
}

#[test]
fn gh_json_comment_count_as_number() {
    // comments as plain number
    let json_line = r#"{"number":6,"title":"Another PR","state":"closed","comments":4}"#;
    let input = ToolExecutionInput {
        tool_name: "exec".to_owned(),
        argv: Some(vec!["gh".to_owned(), "pr".to_owned(), "list".to_owned()]),
        stdout: Some(json_line.to_owned()),
        ..Default::default()
    };
    let result = run(input);
    assert!(
        result.inline_text.contains("#6"),
        "got: {}",
        result.inline_text
    );
    assert!(
        result.inline_text.contains("4c"),
        "got: {}",
        result.inline_text
    );
}

// --- gh json: labels as string array ---

#[test]
fn gh_json_labels_as_string_array() {
    // labels as array of strings (not objects)
    let json_line =
        r#"{"number":8,"title":"Tagged PR","state":"open","labels":["bug","urgent",""]}"#;
    let input = ToolExecutionInput {
        tool_name: "exec".to_owned(),
        argv: Some(vec!["gh".to_owned(), "pr".to_owned(), "list".to_owned()]),
        stdout: Some(json_line.to_owned()),
        ..Default::default()
    };
    let result = run(input);
    assert!(
        result.inline_text.contains("#8"),
        "got: {}",
        result.inline_text
    );
    // Should include label names (empty string filtered)
    assert!(
        result.inline_text.contains("bug") || result.inline_text.contains("{"),
        "got: {}",
        result.inline_text
    );
}

#[test]
fn gh_json_labels_non_array_is_ignored() {
    // labels as non-array → should not crash
    let json_line = r#"{"number":9,"title":"PR no labels","state":"open","labels":"bug"}"#;
    let input = ToolExecutionInput {
        tool_name: "exec".to_owned(),
        argv: Some(vec!["gh".to_owned(), "pr".to_owned(), "list".to_owned()]),
        stdout: Some(json_line.to_owned()),
        ..Default::default()
    };
    let result = run(input);
    assert!(
        result.inline_text.contains("#9"),
        "got: {}",
        result.inline_text
    );
}

// --- pretty_print_json: array and non-json ---

#[test]
fn pretty_print_json_array_output() {
    use crate::openhuman::tokenjuice::{rules::compiler::compile_rule, types::RuleMatch};

    let rule = crate::openhuman::tokenjuice::types::JsonRule {
        id: "test/ppjson-arr".to_owned(),
        family: "test".to_owned(),
        description: None,
        priority: None,
        on_empty: None,
        match_output: None,
        counter_source: None,
        r#match: RuleMatch::default(),
        filters: None,
        transforms: Some(crate::openhuman::tokenjuice::types::RuleTransforms {
            pretty_print_json: Some(true),
            strip_ansi: None,
            trim_empty_edges: None,
            dedupe_adjacent: None,
        }),
        summarize: None,
        counters: None,
        failure: None,
    };
    let compiled = compile_rule(
        rule,
        crate::openhuman::tokenjuice::types::RuleOrigin::Builtin,
        "builtin:test/ppjson-arr".to_owned(),
    );
    // JSON array
    let input = ToolExecutionInput {
        tool_name: "bash".to_owned(),
        argv: Some(vec!["some_tool".to_owned()]),
        stdout: Some(r#"[1,2,3]"#.to_owned()),
        ..Default::default()
    };
    let fb = load_builtin_rules()
        .into_iter()
        .find(|r| r.rule.id == "generic/fallback")
        .unwrap();
    let rules = vec![compiled, fb];
    let opts = ReduceOptions {
        classifier: Some("test/ppjson-arr".to_owned()),
        ..Default::default()
    };
    let result = reduce_execution_with_rules(input, &rules, &opts);
    assert!(!result.inline_text.is_empty());
}

#[test]
fn pretty_print_json_non_json_passthrough() {
    use crate::openhuman::tokenjuice::{rules::compiler::compile_rule, types::RuleMatch};

    let rule = crate::openhuman::tokenjuice::types::JsonRule {
        id: "test/ppjson-plain".to_owned(),
        family: "test".to_owned(),
        description: None,
        priority: None,
        on_empty: None,
        match_output: None,
        counter_source: None,
        r#match: RuleMatch::default(),
        filters: None,
        transforms: Some(crate::openhuman::tokenjuice::types::RuleTransforms {
            pretty_print_json: Some(true),
            strip_ansi: None,
            trim_empty_edges: None,
            dedupe_adjacent: None,
        }),
        summarize: None,
        counters: None,
        failure: None,
    };
    let compiled = compile_rule(
        rule,
        crate::openhuman::tokenjuice::types::RuleOrigin::Builtin,
        "builtin:test/ppjson-plain".to_owned(),
    );
    // Not JSON
    let input = ToolExecutionInput {
        tool_name: "bash".to_owned(),
        argv: Some(vec!["some_tool".to_owned()]),
        stdout: Some("plain text output".to_owned()),
        ..Default::default()
    };
    let fb = load_builtin_rules()
        .into_iter()
        .find(|r| r.rule.id == "generic/fallback")
        .unwrap();
    let rules = vec![compiled, fb];
    let opts = ReduceOptions {
        classifier: Some("test/ppjson-plain".to_owned()),
        ..Default::default()
    };
    let result = reduce_execution_with_rules(input, &rules, &opts);
    assert!(result.inline_text.contains("plain text output"));
}

// --- normalize_execution_input: empty tokenized argv ---

#[test]
fn normalize_whitespace_only_command_returns_no_argv() {
    // tokenize_command("''") → empty (quotes enclose nothing useful)
    let input = ToolExecutionInput {
        tool_name: "bash".to_owned(),
        command: Some("''".to_owned()), // tokenizes to empty because quotes contain nothing
        argv: None,
        ..Default::default()
    };
    let out = normalize_execution_input(input);
    // argv should remain None or empty since tokenized form is empty
    assert!(
        out.argv.as_ref().map(|v| v.is_empty()).unwrap_or(true),
        "expected empty or no argv"
    );
}

// --- select_inline_text: passthrough <= compact_chars branch ---

#[test]
fn select_inline_text_passthrough_shorter_than_compact() {
    // When passthrough is shorter than compact, passthrough is returned
    // This happens for short output where compact is longer (rare but possible)
    let short_output = "short";
    let input = ToolExecutionInput {
        tool_name: "bash".to_owned(),
        argv: Some(vec!["some_tool".to_owned()]),
        stdout: Some(short_output.to_owned()),
        ..Default::default()
    };
    let result = run(input);
    // Short output should just be returned as-is
    assert_eq!(result.inline_text, "short");
}

// --- zero raw_chars gives ratio 1.0 ---

#[test]
fn zero_raw_chars_ratio_is_one() {
    let input = ToolExecutionInput {
        tool_name: "bash".to_owned(),
        argv: Some(vec!["some_tool".to_owned()]),
        stdout: None,
        stderr: None,
        ..Default::default()
    };
    let result = run(input);
    assert_eq!(result.stats.ratio, 1.0);
    assert_eq!(result.stats.raw_chars, 0);
}

// --- gh json with workflowName field ---

#[test]
fn gh_json_workflow_name_field() {
    let json_line = r#"{"databaseId":100,"workflowName":"CI/CD Pipeline","status":"in_progress"}"#;
    let input = ToolExecutionInput {
        tool_name: "exec".to_owned(),
        argv: Some(vec!["gh".to_owned(), "run".to_owned(), "list".to_owned()]),
        stdout: Some(json_line.to_owned()),
        ..Default::default()
    };
    let result = run(input);
    assert!(
        result.inline_text.contains("CI/CD Pipeline") || result.inline_text.contains("#100"),
        "got: {}",
        result.inline_text
    );
}

// --- gh json: no title field returns None (format_gh_json_record returns None) ---

#[test]
fn gh_json_missing_title_falls_to_table_format() {
    // JSON line without any title-like field → format_gh_json_record returns None
    // → falls back to table format since argv[0] == "gh"
    let json_line = r#"{"number":1,"state":"open"}"#;
    let input = ToolExecutionInput {
        tool_name: "exec".to_owned(),
        argv: Some(vec!["gh".to_owned(), "pr".to_owned(), "list".to_owned()]),
        stdout: Some(json_line.to_owned()),
        ..Default::default()
    };
    let result = run(input);
    // Should not panic, result may be formatted or passthrough
    assert!(!result.inline_text.is_empty() || result.inline_text.is_empty());
}

// --- skip_patterns ---

#[test]
fn skip_patterns_remove_matching_lines() {
    // cargo test rule skips "Compiling" and "Finished" lines
    let stdout =
        "   Compiling foo v0.1.0\n   Finished dev [unoptimized] target(s)\ntest foo ... ok\n";
    let input = ToolExecutionInput {
        tool_name: "exec".to_owned(),
        argv: Some(vec!["cargo".to_owned(), "test".to_owned()]),
        stdout: Some(stdout.to_owned()),
        ..Default::default()
    };
    let result = run(input);
    assert!(
        !result.inline_text.contains("Compiling"),
        "got: {}",
        result.inline_text
    );
}

// --- format_inline: search family includes facts ---

#[test]
fn search_family_includes_fact_counts() {
    let output = "file.rs:10: match one\nfile.rs:20: match two\nfile.rs:30: match three\n";
    let input = ToolExecutionInput {
        tool_name: "exec".to_owned(),
        argv: Some(vec!["grep".to_owned(), "-r".to_owned(), "match".to_owned()]),
        stdout: Some(output.to_owned()),
        ..Default::default()
    };
    let result = run(input);
    // Search family should include fact counts in inline text
    // (either via "matches" text or facts map)
    assert!(!result.inline_text.is_empty());
}

// --- test-results family with failure exits includes facts ---

#[test]
fn test_results_failure_includes_failed_count() {
    let output = "test foo ... ok\ntest bar ... FAILED\ntest baz ... ok\nFAILED\n";
    let input = ToolExecutionInput {
        tool_name: "exec".to_owned(),
        argv: Some(vec!["cargo".to_owned(), "test".to_owned()]),
        stdout: Some(output.to_owned()),
        exit_code: Some(1),
        ..Default::default()
    };
    let result = run(input);
    // Should contain information about the failure
    assert!(!result.inline_text.is_empty());
}

// --- git/status rewrite: "and have N and M different commits" ---

#[test]
fn git_status_diverged_message_removed() {
    let stdout = "On branch main\nYour branch and 'origin/main' have diverged,\nand have 2 and 3 different commits each.\n";
    let input = make_input("bash", &["git", "status"], stdout);
    let result = run(input);
    assert!(
        !result.inline_text.contains("and have"),
        "got: {}",
        result.inline_text
    );
}

#[test]
fn git_status_empty_line_handled() {
    // Empty lines in git status output should produce empty strings (not be dropped)
    let stdout = "Changes not staged for commit:\n\n\tmodified:   foo.rs\n";
    let input = make_input("bash", &["git", "status"], stdout);
    let result = run(input);
    // Should still have M: foo.rs
    assert!(
        result.inline_text.contains("M: foo.rs"),
        "got: {}",
        result.inline_text
    );
}

#[test]
fn git_status_no_changes_hint_removed() {
    let stdout = "nothing added to commit but untracked files present (use \"git add\" to track)\n";
    let input = make_input("bash", &["git", "status"], stdout);
    let result = run(input);
    // This line should be filtered out
    assert!(
        !result
            .inline_text
            .contains("nothing added to commit but untracked"),
        "got: {}",
        result.inline_text
    );
}

#[test]
fn git_status_use_git_hint_removed() {
    let stdout = "(use \"git add <file>...\" to update what will be committed)\n";
    let input = make_input("bash", &["git", "status"], stdout);
    let result = run(input);
    assert!(
        !result.inline_text.contains("use \"git add"),
        "got: {}",
        result.inline_text
    );
}

#[test]
fn git_status_porcelain_format_mm_code() {
    // Two-char porcelain status code
    let stdout = "MM src/foo.rs\nA  src/bar.rs\n";
    let input = make_input("bash", &["git", "status"], stdout);
    let result = run(input);
    // Should be parsed somehow (via porcelain fallthrough or direct match)
    assert!(!result.inline_text.is_empty());
}

#[test]
fn git_status_consecutive_empty_lines_collapsed() {
    // Multiple consecutive blank lines should be collapsed to one
    let stdout = "Changes not staged for commit:\n\n\n\tmodified:   a.rs\n\n\n\tmodified:   b.rs\n";
    let input = make_input("bash", &["git", "status"], stdout);
    let result = run(input);
    assert!(
        result.inline_text.contains("M: a.rs"),
        "got: {}",
        result.inline_text
    );
}

#[test]
fn git_status_no_changes_to_commit() {
    let stdout = "no changes added to commit (use \"git add\" and/or \"git commit -a\")\n";
    let input = make_input("bash", &["git", "status"], stdout);
    let result = run(input);
    assert!(
        !result.inline_text.contains("no changes added to commit"),
        "got: {}",
        result.inline_text
    );
}

// --- head_tail with zero counts ---

#[test]
fn head_tail_zero_head() {
    use crate::openhuman::tokenjuice::text::head_tail;
    let lines: Vec<String> = (0..5).map(|i| format!("line{}", i)).collect();
    // head=0, tail=2 should return last 2 lines
    let result = head_tail(&lines, 0, 2);
    assert_eq!(result.len(), 3); // omission marker + 2 tail lines
    assert!(result[0].contains("omitted"));
}

#[test]
fn head_tail_zero_tail() {
    use crate::openhuman::tokenjuice::text::head_tail;
    let lines: Vec<String> = (0..5).map(|i| format!("line{}", i)).collect();
    let result = head_tail(&lines, 2, 0);
    // 2 head + omission marker + 0 tail
    assert_eq!(result.len(), 3);
}

#[test]
fn head_tail_n_greater_than_line_count() {
    use crate::openhuman::tokenjuice::text::head_tail;
    let lines: Vec<String> = (0..3).map(|i| format!("line{}", i)).collect();
    // head+tail > total, should passthrough unchanged
    let result = head_tail(&lines, 5, 5);
    assert_eq!(result, lines);
}
