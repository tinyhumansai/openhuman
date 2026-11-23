//! Stub.

use serde_json::Value;
use std::path::Path;

pub(super) fn view_only_refusal(_action: &str, _args: &Value) -> Option<String> {
    None
}

pub(super) fn pool_key(scope: &Path, thread_id: &str, _view_only: bool) -> String {
    format!("{}:{thread_id}", scope.display())
}

#[cfg(test)]
#[path = "browser_view_only_tests.rs"]
mod tests;
