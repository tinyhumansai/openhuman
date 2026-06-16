// Sub-modules split by concern from the original ollama_admin.rs (1586 lines).
mod binary;
mod diagnostics;
mod health;
mod model_pull;
mod server;
mod util;

// Re-export free functions that form the public/crate API of this module.
pub(crate) use util::interrupted_pull_settle_window_secs;
pub(crate) use util::kill_pid_by_id;
pub(crate) use util::test_ollama_connection;

#[cfg(test)]
#[path = "../ollama_admin_tests.rs"]
mod tests;
