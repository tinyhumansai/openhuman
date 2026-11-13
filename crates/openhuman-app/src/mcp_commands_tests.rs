use super::*;
use crate::core_binary::find_debug_binary_walking_up;

// -------------------------------------------------------------------------
// config_path_for_client — pure path resolution tests
// -------------------------------------------------------------------------

#[test]
fn config_path_claude_desktop_macos() {
    let path = config_path_for_client("claude-desktop", "macos").expect("should resolve on macos");
    let s = path.display().to_string();
    assert!(
        s.contains("Library/Application Support/Claude/claude_desktop_config.json"),
        "unexpected path: {s}"
    );
}

#[test]
fn config_path_claude_desktop_linux() {
    let path = config_path_for_client("claude-desktop", "linux").expect("should resolve on linux");
    let s = path.display().to_string();
    assert!(
        s.contains(".config/Claude/claude_desktop_config.json"),
        "unexpected path: {s}"
    );
}

#[test]
fn config_path_cursor_macos() {
    let path = config_path_for_client("cursor", "macos").expect("should resolve");
    let s = path.display().to_string();
    assert!(s.ends_with(".cursor/mcp.json"), "unexpected path: {s}");
}

#[test]
fn config_path_cursor_linux() {
    let path = config_path_for_client("cursor", "linux").expect("should resolve");
    let s = path.display().to_string();
    assert!(s.ends_with(".cursor/mcp.json"), "unexpected path: {s}");
}

#[test]
fn config_path_codex_all_platforms() {
    for os in &["macos", "windows", "linux"] {
        let path = config_path_for_client("codex", os)
            .unwrap_or_else(|_| panic!("should resolve for os={os}"));
        let s = path.display().to_string();
        assert!(
            s.ends_with(".codex/config.json"),
            "unexpected path for os={os}: {s}"
        );
    }
}

#[test]
fn config_path_zed_macos() {
    let path = config_path_for_client("zed", "macos").expect("should resolve");
    let s = path.display().to_string();
    assert!(
        s.contains("Library/Application Support/Zed/settings.json"),
        "unexpected path: {s}"
    );
}

#[test]
fn config_path_zed_linux() {
    let path = config_path_for_client("zed", "linux").expect("should resolve");
    let s = path.display().to_string();
    assert!(
        s.contains(".config/zed/settings.json"),
        "unexpected path: {s}"
    );
}

#[test]
fn config_path_unknown_client_returns_err() {
    let result = config_path_for_client("unknown-client", "macos");
    assert!(result.is_err(), "unknown client should return Err");
    let err = result.unwrap_err();
    assert!(
        err.contains("Unknown MCP client: unknown-client"),
        "unexpected error message: {err}"
    );
}

// -------------------------------------------------------------------------
// mcp_resolve_binary_path — integration-style test (path must exist in dev)
// -------------------------------------------------------------------------

/// In debug builds (the only mode in which `cargo test` runs), the binary
/// path resolver should either find `OPENHUMAN_CORE_BINARY_PATH` or locate
/// `target/debug/openhuman-core` by walking up from the test executable.
///
/// We only assert the path *contains* `openhuman-core` — the binary may or
/// may not exist on disk in a fresh checkout, so we don't assert `Ok` here;
/// instead we verify the error message is sensible when the file is absent.
#[test]
fn binary_path_result_contains_openhuman_core() {
    match resolve_binary_path() {
        Ok(p) => {
            let s = p.display().to_string();
            assert!(
                s.contains("openhuman-core"),
                "resolved path should contain 'openhuman-core', got: {s}"
            );
        }
        Err(e) => {
            // Acceptable in a clean CI checkout where the binary hasn't
            // been built yet. The error must be descriptive.
            assert!(
                e.contains("openhuman-core") || e.contains("current_exe") || e.contains("target"),
                "error message should reference the binary or path: {e}"
            );
        }
    }
}

#[test]
fn find_debug_binary_returns_none_for_empty_dir() {
    let dir = tempfile::tempdir().expect("tempdir");
    // Walk up from a fresh tempdir in the system temp folder — no ancestor
    // of /tmp (or equivalent) will contain target/debug/openhuman-core.
    let result = find_debug_binary_walking_up(dir.path());
    assert!(
        result.is_none(),
        "expected None walking up from an empty tempdir, got: {result:?}"
    );
}
