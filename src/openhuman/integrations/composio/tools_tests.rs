use super::*;
use std::path::Path;
use std::sync::Arc;

struct WorkspaceEnvGuard {
    previous: Option<std::ffi::OsString>,
}

impl WorkspaceEnvGuard {
    fn set(path: &Path) -> Self {
        let previous = std::env::var_os("OPENHUMAN_WORKSPACE");
        unsafe {
            std::env::set_var("OPENHUMAN_WORKSPACE", path);
        }
        Self { previous }
    }
}

struct HomeEnvGuard {
    previous: Option<std::ffi::OsString>,
}

impl HomeEnvGuard {
    fn set(path: &Path) -> Self {
        let previous = std::env::var_os("HOME");
        unsafe {
            std::env::set_var("HOME", path);
        }
        Self { previous }
    }
}

impl Drop for HomeEnvGuard {
    fn drop(&mut self) {
        unsafe {
            match self.previous.take() {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
        }
    }
}

impl Drop for WorkspaceEnvGuard {
    fn drop(&mut self) {
        unsafe {
            match self.previous.take() {
                Some(value) => std::env::set_var("OPENHUMAN_WORKSPACE", value),
                None => std::env::remove_var("OPENHUMAN_WORKSPACE"),
            }
        }
    }
}

/// Minimal `Arc<Config>` for the agent-tool constructors. All five
/// composio agent tools now resolve their client per call through
/// `create_composio_client(&config)` rather than holding a pre-baked
/// handle, so a `Config` is sufficient to instantiate them.
///
/// Config defaults set `composio.mode = "backend"` and stash a
/// throwaway `config_path` under a tempdir. The factory then returns
/// `Err("no backend session")` because no app-session token is stored
/// in the test keychain — that error path is the one we want for the
/// "executes without backend session" failure-mode tests; tests that
/// need a session token override the keychain explicitly.
fn fake_config_arc() -> Arc<crate::openhuman::config::Config> {
    let tmp = tempfile::tempdir().expect("tempdir for fake_config_arc");
    let mut config = crate::openhuman::config::Config::default();
    config.config_path = tmp.path().join("config.toml");
    // Leak the tempdir so the path remains valid for the test's lifetime
    // — `Config::config_path` is just used as a lookup key here, not
    // actually written to.
    std::mem::forget(tmp);
    Arc::new(config)
}

// ── composio_connect (inline approval card, #3993) ──────────────────

fn tool_result_text(result: &crate::openhuman::tools::traits::ToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|c| match c {
            crate::openhuman::tools::traits::ToolContent::Text { text } => Some(text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" ")
}

// ── Sandbox-mode gate (issue #685) ───────────────────────────────
//
// These tests stand alone from the backend client — they only exercise
// the gate added to `ComposioExecuteTool::execute` that keys on the
// `CURRENT_AGENT_SANDBOX_MODE` task-local. The backend is never reached
// when the gate rejects, so `fake_config_arc()` is fine.

fn error_text(result: &ToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|c| match c {
            crate::openhuman::tools::traits::ToolContent::Text { text } => Some(text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" ")
}

// ── Direct-mode routing (#1710) ─────────────────────────────────────
//
// These tests guard the bug-fix where every composio agent tool used
// to hold a pre-baked backend client. After the fix, all five tools
// resolve the client through `create_composio_client` per call so the
// live `composio.mode` toggle is honoured. Read-shaped tools
// (list_toolkits, list_connections, list_tools) short-circuit to an
// empty response in direct mode mirroring the existing ops.rs
// pattern; `composio_authorize` returns an explicit "use
// app.composio.dev" error; `composio_execute` dispatches through the
// direct client.

/// Helper: build a `Config` with `composio.mode = "direct"` plus an
/// inline api_key so the keychain isn't required.
fn direct_mode_config() -> crate::openhuman::config::Config {
    let tmp = tempfile::tempdir().expect("tempdir for direct_mode_config");
    let mut config = crate::openhuman::config::Config::default();
    config.config_path = tmp.path().join("config.toml");
    config.composio.mode = crate::openhuman::config::schema::COMPOSIO_MODE_DIRECT.to_string();
    config.composio.api_key = Some("test-direct-key".to_string());
    std::mem::forget(tmp);
    config
}

#[path = "tools_tests_part_01_tests.rs"]
mod part_01_tests;
#[path = "tools_tests_part_02_tests.rs"]
mod part_02_tests;
