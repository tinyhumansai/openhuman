use super::*;
use std::sync::Arc;

use crate::config::test_env::EnvVarGuard;
use tinytools::ToolResult;

/// Minimal `Arc<Config>` for the agent-tool constructors. All five
/// composio agent tools now resolve their client per call through
/// `resolve_composio_route(&config)` rather than holding a pre-baked
/// handle, so a `Config` is sufficient to instantiate them.
///
/// Config defaults set `composio.mode = "backend"` and stash a
/// throwaway `config_path` under a tempdir. The factory then returns
/// `Err("no backend session")` because no app-session token is stored
/// in the test keychain — that error path is the one we want for the
/// "executes without backend session" failure-mode tests; tests that
/// need a session token override the keychain explicitly.
fn fake_config_arc() -> Arc<crate::config::Config> {
    let tmp = tempfile::tempdir().expect("tempdir for fake_config_arc");
    let mut config = crate::config::Config::default();
    config.config_path = tmp.path().join("config.toml");
    // Leak the tempdir so the path remains valid for the test's lifetime
    // — `Config::config_path` is just used as a lookup key here, not
    // actually written to.
    std::mem::forget(tmp);
    Arc::new(config)
}

// ── composio_connect (inline approval card, #3993) ──────────────────

fn tool_result_text(result: &tinytools::ToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|c| match c {
            tinytools::ToolContent::Text { text } => Some(text.clone()),
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
            tinytools::ToolContent::Text { text } => Some(text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" ")
}

// ── Direct-mode routing (#1710) ─────────────────────────────────────
//
// These tests guard the bug-fix where every composio agent tool used
// to hold a pre-baked backend client. After the fix, all five tools
// resolve the client through `resolve_composio_route` per call so the
// live `composio.mode` toggle is honoured. Read-shaped tools
// (list_toolkits, list_connections, list_tools) short-circuit to an
// empty response in direct mode mirroring the existing ops.rs
// pattern; `composio_authorize` returns an explicit "use
// app.composio.dev" error; `composio_execute` dispatches through the
// direct client.

/// Helper: build a `Config` with `composio.mode = "direct"` plus an
/// inline api_key so the keychain isn't required.
fn direct_mode_config() -> crate::config::Config {
    let tmp = tempfile::tempdir().expect("tempdir for direct_mode_config");
    let mut config = crate::config::Config::default();
    config.config_path = tmp.path().join("config.toml");
    config.composio.mode = crate::config::schema::COMPOSIO_MODE_DIRECT.to_string();
    config.composio.api_key = Some("test-direct-key".to_string());
    std::mem::forget(tmp);
    config
}

#[path = "tools_direct_mode_routing_tests.rs"]
mod direct_mode_routing_tests;
#[path = "tools_host_credential_tests.rs"]
mod host_credential_tests;
#[path = "tools_metadata_and_sandbox_tests.rs"]
mod metadata_and_sandbox_tests;
#[path = "tools_redact_tests.rs"]
mod redact_tests;

#[test]
fn shared_security_corpus_pins_configured_secret_redaction() {
    let corpus: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/security-redaction-corpus.json"
    )))
    .unwrap();
    for case in corpus["text"].as_array().unwrap() {
        let input = case["input"].as_str().unwrap();
        let secret = case["composio_secret"].as_str().unwrap().to_owned();
        let output = redact::redact_text(input, &[secret]);
        let expected = if case["case"] == "anthropic_key" {
            "[REDACTED]"
        } else {
            input
        };
        assert_eq!(output, expected, "{}", case["case"]);
    }
}
