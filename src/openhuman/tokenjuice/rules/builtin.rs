//! Embedded built-in rule JSON files.
//!
//! Each rule is embedded at compile time via `include_str!` so the module
//! works with zero external configuration.

/// All vendored rule JSON files embedded as `(id, json)` pairs.
///
/// The `generic/fallback` rule MUST be present; the compiler asserts this via
/// `builtin_rules()`.
pub static BUILTIN_RULE_JSONS: &[(&str, &str)] = &[
    (
        "generic/fallback",
        include_str!("../vendor/rules/generic__fallback.json"),
    ),
    (
        "git/status",
        include_str!("../vendor/rules/git__status.json"),
    ),
    (
        "git/branch",
        include_str!("../vendor/rules/git__branch.json"),
    ),
    (
        "git/diff-stat",
        include_str!("../vendor/rules/git__diff-stat.json"),
    ),
    (
        "git/diff-name-only",
        include_str!("../vendor/rules/git__diff-name-only.json"),
    ),
    (
        "git/log-oneline",
        include_str!("../vendor/rules/git__log-oneline.json"),
    ),
    ("cloud/gh", include_str!("../vendor/rules/cloud__gh.json")),
    (
        "install/npm-install",
        include_str!("../vendor/rules/install__npm-install.json"),
    ),
    (
        "tests/cargo-test",
        include_str!("../vendor/rules/tests__cargo-test.json"),
    ),
    (
        "tests/pytest",
        include_str!("../vendor/rules/tests__pytest.json"),
    ),
];
