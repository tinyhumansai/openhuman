// Sub-modules split by concern from the original ollama_admin.rs (1586 lines).
mod binary;
mod diagnostics;
mod health;
mod model_pull;
mod server;
mod util;

// Re-export free functions that form the public/crate API of this module.
pub(crate) use util::test_ollama_connection;

// Test-facing re-export: `ollama_admin_tests.rs` resolves `super::…` against
// this module. Dropped during the sub-module split; restored so libtest compiles.
#[cfg(test)]
pub(crate) use util::interrupted_pull_settle_window_secs;

#[cfg(test)]
#[path = "../ollama_admin_tests.rs"]
mod tests;
