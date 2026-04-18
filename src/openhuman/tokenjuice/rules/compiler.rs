//! Rule compilation: converts a `JsonRule` descriptor into a `CompiledRule`
//! with pre-built `regex::Regex` instances.
//!
//! Invalid regex patterns produce a non-fatal diagnostic log and are silently
//! dropped so a bad user rule does not crash the engine.

use crate::openhuman::tokenjuice::types::{
    CompiledCounter, CompiledOutputMatch, CompiledParts, CompiledRule, JsonRule, RuleOrigin,
};

// ---------------------------------------------------------------------------
// Regex helpers
// ---------------------------------------------------------------------------

/// Build regex flags ensuring `u` (Unicode) is always present.
///
/// Upstream uses `new RegExp(pattern, mergeRegexFlags(flags))` where `u` is
/// always prepended.  In Rust's `regex` crate there is no separate `u` flag —
/// Unicode is on by default — so we translate only `i` (case-insensitive) and
/// `m` (multiline).
fn build_regex(pattern: &str, flags: Option<&str>) -> Option<regex::Regex> {
    let case_insensitive = flags.map(|f| f.contains('i')).unwrap_or(false);
    let multiline = flags.map(|f| f.contains('m')).unwrap_or(false);

    // Build pattern with inline flags
    let prefix = match (case_insensitive, multiline) {
        (true, true) => "(?im)",
        (true, false) => "(?i)",
        (false, true) => "(?m)",
        (false, false) => "",
    };
    let full = format!("{}{}", prefix, pattern);

    match regex::Regex::new(&full) {
        Ok(re) => Some(re),
        Err(err) => {
            log::debug!(
                "[tokenjuice] rule compiler: invalid regex '{}' (flags={:?}): {}",
                pattern,
                flags,
                err
            );
            None
        }
    }
}

// ---------------------------------------------------------------------------
// compile_rule
// ---------------------------------------------------------------------------

/// Compile a `JsonRule` into a `CompiledRule`.
///
/// `path` is either a filesystem path or `"builtin:<id>"` for embedded rules.
pub fn compile_rule(rule: JsonRule, source: RuleOrigin, path: String) -> CompiledRule {
    log::debug!(
        "[tokenjuice] compiling rule '{}' from {:?} path={}",
        rule.id,
        source,
        path
    );

    let skip_patterns: Vec<regex::Regex> = rule
        .filters
        .as_ref()
        .and_then(|f| f.skip_patterns.as_ref())
        .map(|pats| pats.iter().filter_map(|p| build_regex(p, None)).collect())
        .unwrap_or_default();

    let keep_patterns: Vec<regex::Regex> = rule
        .filters
        .as_ref()
        .and_then(|f| f.keep_patterns.as_ref())
        .map(|pats| pats.iter().filter_map(|p| build_regex(p, None)).collect())
        .unwrap_or_default();

    let counters: Vec<CompiledCounter> = rule
        .counters
        .as_ref()
        .map(|counters| {
            counters
                .iter()
                .filter_map(|c| {
                    build_regex(&c.pattern, c.flags.as_deref()).map(|re| CompiledCounter {
                        name: c.name.clone(),
                        pattern: re,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let output_matches: Vec<CompiledOutputMatch> = rule
        .match_output
        .as_ref()
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| {
                    build_regex(&entry.pattern, entry.flags.as_deref()).map(|re| {
                        CompiledOutputMatch {
                            pattern: re,
                            message: entry.message.clone(),
                        }
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    CompiledRule {
        compiled: CompiledParts {
            skip_patterns,
            keep_patterns,
            counters,
            output_matches,
        },
        rule,
        source,
        path,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::tokenjuice::types::{JsonRule, RuleMatch};

    fn minimal_rule(id: &str) -> JsonRule {
        JsonRule {
            id: id.to_owned(),
            family: "test".to_owned(),
            description: None,
            priority: None,
            on_empty: None,
            match_output: None,
            counter_source: None,
            r#match: RuleMatch::default(),
            filters: None,
            transforms: None,
            summarize: None,
            counters: None,
            failure: None,
        }
    }

    #[test]
    fn compiles_minimal_rule() {
        let rule = minimal_rule("test/rule");
        let compiled = compile_rule(rule, RuleOrigin::Builtin, "builtin:test/rule".to_owned());
        assert_eq!(compiled.rule.id, "test/rule");
        assert!(compiled.compiled.skip_patterns.is_empty());
    }

    #[test]
    fn invalid_regex_is_dropped_not_panicked() {
        use crate::openhuman::tokenjuice::types::{RuleCounter, RuleFilters};
        let mut rule = minimal_rule("test/bad");
        rule.filters = Some(RuleFilters {
            skip_patterns: Some(vec!["[invalid".to_owned()]),
            keep_patterns: None,
        });
        rule.counters = Some(vec![RuleCounter {
            name: "bad counter".to_owned(),
            pattern: "(unclosed".to_owned(),
            flags: None,
        }]);
        let compiled = compile_rule(rule, RuleOrigin::Builtin, "builtin:test/bad".to_owned());
        // Both should be silently dropped
        assert!(compiled.compiled.skip_patterns.is_empty());
        assert!(compiled.compiled.counters.is_empty());
    }

    #[test]
    fn case_insensitive_flag() {
        use crate::openhuman::tokenjuice::types::RuleCounter;
        let mut rule = minimal_rule("test/ci");
        rule.counters = Some(vec![RuleCounter {
            name: "error".to_owned(),
            pattern: "error".to_owned(),
            flags: Some("i".to_owned()),
        }]);
        let compiled = compile_rule(rule, RuleOrigin::Builtin, "builtin:test/ci".to_owned());
        assert_eq!(compiled.compiled.counters.len(), 1);
        assert!(compiled.compiled.counters[0].pattern.is_match("ERROR"));
        assert!(compiled.compiled.counters[0].pattern.is_match("error"));
    }
}
