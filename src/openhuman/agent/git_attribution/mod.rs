//! Git commit attribution for changes made by the OpenHuman agent.
//!
//! Agent shell commands inherit a temporary `prepare-commit-msg` hook which
//! appends OpenHuman's co-author trailer. The hook directory contains shims for
//! every client-side hook, so a repository's validation hooks continue to run.

mod hook;

pub use hook::hook_env;

#[cfg(test)]
#[path = "git_attribution_tests.rs"]
mod tests;
