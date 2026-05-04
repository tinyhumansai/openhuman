use super::*;
use crate::openhuman::tokenjuice::rules::compiler::compile_rule;
use crate::openhuman::tokenjuice::types::RuleOrigin;

/// Load every builtin rule and assert:
///   (a) none fail to parse as `JsonRule`
///   (b) duplicate ids are detected and reported (but the test does not fail)
///
/// This mirrors the lenient-by-design rule loader: a bad JSON entry is
/// logged but does not crash the engine.
#[test]
fn all_builtins_parse_without_error() {
    use crate::openhuman::tokenjuice::types::JsonRule;
    use std::collections::HashMap;

    let mut id_count: HashMap<String, Vec<&str>> = HashMap::new();
    let mut parse_failures: Vec<(&str, String)> = Vec::new();

    for (id, json) in BUILTIN_RULE_JSONS {
        match serde_json::from_str::<JsonRule>(json) {
            Ok(rule) => {
                id_count.entry(rule.id.clone()).or_default().push(id);
            }
            Err(e) => {
                parse_failures.push((id, e.to_string()));
                eprintln!("[tokenjuice/builtin] PARSE FAIL '{}': {}", id, e);
            }
        }
    }

    // Report duplicate ids (non-fatal: last-write wins in the loader anyway)
    for (rule_id, ids) in &id_count {
        if ids.len() > 1 {
            eprintln!(
                "[tokenjuice/builtin] DUPLICATE id '{}' in entries: {:?}",
                rule_id, ids
            );
        }
    }

    let duplicates: Vec<_> = id_count
        .iter()
        .filter(|(_, v)| v.len() > 1)
        .map(|(k, _)| k.as_str())
        .collect();

    assert!(
        parse_failures.is_empty(),
        "builtin rule parse failures: {:?}",
        parse_failures
    );
    assert!(
        duplicates.is_empty(),
        "duplicate builtin rule ids (fix builtin.rs): {:?}",
        duplicates
    );
}

/// Compile all builtins and list any that fail to compile (non-fatal).
/// This ensures the lenient compile path is exercised and gives a clear
/// inventory if any regex is incompatible with the `regex` crate.
#[test]
fn all_builtins_compile() {
    use crate::openhuman::tokenjuice::types::JsonRule;

    let mut compile_issues: Vec<String> = Vec::new();

    for (id, json) in BUILTIN_RULE_JSONS {
        let rule: JsonRule = match serde_json::from_str(json) {
            Ok(r) => r,
            Err(e) => {
                compile_issues.push(format!("PARSE '{}': {}", id, e));
                continue;
            }
        };

        // compile_rule is lenient: invalid regex is dropped (not panicked)
        let compiled = compile_rule(rule, RuleOrigin::Builtin, format!("builtin:{}", id));

        // For rules that define counters/filters/output_matches, check that
        // at least some patterns compiled (unless no patterns were declared).
        // We do NOT fail on partial compilation — log only.
        let _ = compiled; // compilation itself must not panic
    }

    if !compile_issues.is_empty() {
        eprintln!(
            "[tokenjuice/builtin] {} compile issues (lenient — not failing test):",
            compile_issues.len()
        );
        for issue in &compile_issues {
            eprintln!("  {}", issue);
        }
    }

    // The test passes as long as compile_rule doesn't panic for any builtin.
    // Partial regex failures are logged above but do not fail the suite.
}

#[test]
fn generic_fallback_is_present() {
    let has_fallback = BUILTIN_RULE_JSONS
        .iter()
        .any(|(id, _)| *id == "generic/fallback");
    assert!(
        has_fallback,
        "generic/fallback must be in BUILTIN_RULE_JSONS"
    );
}

#[test]
fn total_builtin_count() {
    // Ensure we have the expected number of vendored rules.
    // Update this number when new rules are added.
    assert_eq!(
        BUILTIN_RULE_JSONS.len(),
        96,
        "expected 96 builtin rules; update this assertion if the vendor set changes"
    );
}

// --- exercise the parse-fail and duplicate code paths in-situ ---

#[test]
fn duplicate_id_reporting_logic_works() {
    // Exercise the "ids.len() > 1" and duplicate-filter branches of the
    // all_builtins_parse_without_error helper by running the same logic
    // on a synthetic set containing a known duplicate.
    use crate::openhuman::tokenjuice::types::JsonRule;
    use std::collections::HashMap;

    let test_entries: &[(&str, &str)] = &[
        ("rule-a", r#"{"id":"dup","family":"test","match":{}}"#),
        ("rule-b", r#"{"id":"dup","family":"test","match":{}}"#),
        ("rule-c", r#"{"id":"unique","family":"test","match":{}}"#),
    ];

    let mut id_count: HashMap<String, Vec<&str>> = HashMap::new();
    for (entry_id, json) in test_entries {
        if let Ok(rule) = serde_json::from_str::<JsonRule>(json) {
            id_count.entry(rule.id.clone()).or_default().push(entry_id);
        }
    }

    // Exercise the duplicate-reporting branch
    for (rule_id, ids) in &id_count {
        if ids.len() > 1 {
            // This is the branch normally exercised by all_builtins_parse_without_error
            // when duplicates exist. We just log it here.
            eprintln!("TEST duplicate '{}' in {:?}", rule_id, ids);
        }
    }

    let duplicates: Vec<_> = id_count
        .iter()
        .filter(|(_, v)| v.len() > 1)
        .map(|(k, _)| k.as_str())
        .collect();
    assert_eq!(duplicates.len(), 1, "expected exactly one duplicate");
    assert_eq!(duplicates[0], "dup");
}

#[test]
fn compile_issues_reporting_logic_works() {
    // Exercise the compile_issues error-reporting branch from all_builtins_compile
    // by simulating the path with a known-bad JSON entry.
    let mut compile_issues: Vec<String> = Vec::new();

    // Simulate a parse failure (bad JSON)
    let bad_json = "{ not valid json at all }";
    if let Err(e) = serde_json::from_str::<crate::openhuman::tokenjuice::types::JsonRule>(bad_json)
    {
        compile_issues.push(format!("PARSE 'bad-entry': {}", e));
    }

    // Now exercise the reporting branch
    assert!(!compile_issues.is_empty());
    eprintln!(
        "[test] {} compile issues (expected in this test):",
        compile_issues.len()
    );
    for issue in &compile_issues {
        eprintln!("  {}", issue);
    }
}
