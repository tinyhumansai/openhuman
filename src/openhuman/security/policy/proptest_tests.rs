//! Property-based tests for the autonomy classifier's adversarial-input
//! surfaces (plan.md §6.3 — proptest was previously absent). The command
//! classifier and path checks run on fully untrusted strings from the
//! LLM/user, so the central properties are "never panics on any input" and a
//! handful of fail-closed invariants that must hold for *all* inputs, not just
//! the hand-enumerated cases in `policy_tests.rs`.

use super::types::{CommandClass, SecurityPolicy};
use proptest::prelude::*;

proptest! {
    /// The classifier + allowlist must never panic, regardless of shell
    /// metacharacters, redirects, pipes, or embedded control characters.
    #[test]
    fn classifier_never_panics_on_regex_strings(cmd in ".*") {
        let p = SecurityPolicy::default();
        let _ = p.classify_command(&cmd);
        let _ = p.command_risk_level(&cmd);
        let _ = p.is_command_allowed(&cmd);
        let _ = p.check_gated_command(&cmd);
    }

    /// Same, but over fully arbitrary unicode (includes NUL, newlines, and
    /// multibyte scalars the `.*` strategy skips).
    #[test]
    fn classifier_never_panics_on_arbitrary_unicode(cmd in any::<String>()) {
        let p = SecurityPolicy::default();
        let _ = p.classify_command(&cmd);
        let _ = p.command_risk_level(&cmd);
        let _ = p.is_command_allowed(&cmd);
    }

    /// Fail-closed floor: a quote-free command with an appended unquoted
    /// redirect always classifies as at least `Write` (`cmd > file` writes
    /// `file`), no matter what the base command is.
    #[test]
    fn appended_redirect_forces_at_least_write(prefix in "[a-z][a-z ]{0,40}") {
        let p = SecurityPolicy::default();
        let cmd = format!("{prefix} > out.txt");
        prop_assert!(
            p.classify_command(&cmd) >= CommandClass::Write,
            "an unquoted redirect must lift the class to >= Write: {cmd:?}"
        );
    }

    /// The string-level path check must never panic on arbitrary paths
    /// (traversal, NUL bytes, tilde, URL-encoding, unicode).
    #[test]
    fn path_string_check_never_panics(path in ".*") {
        let p = SecurityPolicy::default();
        let _ = p.is_path_string_allowed(&path);
    }
}
