use super::*;

#[test]
fn write_mcp_http_config_emits_http_url_with_bearer_header() {
    let dir = tempfile::tempdir().expect("tempdir");
    let addr: std::net::SocketAddr = "127.0.0.1:54321".parse().unwrap();
    let path = write_mcp_http_config(dir.path(), addr, "tok-abc123").expect("write config");
    let raw = std::fs::read_to_string(&path).expect("read config");
    let v: serde_json::Value = serde_json::from_str(&raw).expect("valid json");
    let server = &v["mcpServers"]["openhuman"];
    assert_eq!(
        server["type"], "http",
        "MCP transport must be http (out-of-jail)"
    );
    assert_eq!(server["url"], "http://127.0.0.1:54321/");
    // The loopback server is authenticated — the config must carry the bearer.
    assert_eq!(server["headers"]["Authorization"], "Bearer tok-abc123");
    // It must NOT spawn a stdio child (the old jailed path).
    assert!(server.get("command").is_none());
}

#[test]
fn large_system_prompt_is_written_to_file_instead_of_argv() {
    let dir = tempfile::tempdir().expect("tempdir");
    let prompt = "system instruction\n".repeat(2_500);
    assert!(prompt.len() > 32_767);

    let args = append_system_prompt_args(dir.path(), Some(&prompt)).expect("prompt args");

    assert_eq!(args[0], "--append-system-prompt-file");
    assert_eq!(args.len(), 2);
    assert!(!args.iter().any(|arg| arg.contains(&prompt)));
    assert_eq!(
        std::fs::read_to_string(&args[1]).expect("read prompt file"),
        prompt
    );
}

#[test]
fn empty_system_prompt_does_not_add_an_argument() {
    let dir = tempfile::tempdir().expect("tempdir");
    let args = append_system_prompt_args(dir.path(), Some("  \n ")).expect("prompt args");

    assert!(args.is_empty());
    assert!(!dir.path().join("append-system-prompt.txt").exists());
}

#[test]
fn system_prompt_write_error_is_propagated() {
    let dir = tempfile::tempdir().expect("tempdir");
    let not_a_directory = dir.path().join("file");
    std::fs::write(&not_a_directory, "occupied").expect("write blocking file");

    let error = append_system_prompt_args(&not_a_directory, Some("system prompt"))
        .expect_err("non-directory parent must fail");

    assert!(!error.to_string().is_empty());
}

#[cfg(target_os = "macos")]
#[test]
fn seatbelt_profile_denies_whole_openhuman_root_not_just_subdir() {
    // Driver passes the per-user subdir; the jail must deny the WHOLE
    // `.openhuman-staging` tree (so root-level core.token/credentials are
    // protected), not just the subdir.
    let ws = std::path::Path::new("/Users/test/.openhuman-staging/users/abc/workspace");
    let p = seatbelt_profile(ws);
    assert!(
        p.contains("(allow default)"),
        "CC does everything by default"
    );
    assert!(p.contains("(deny file-write*"), "must deny writes");
    assert!(
        p.contains("(deny file-read*"),
        "must deny reads (no token exfil)"
    );
    // Denied path is the ROOT, not the per-user subdir.
    assert!(
        p.contains("/Users/test/.openhuman-staging\""),
        "deny subpath must be the .openhuman root: {p}"
    );
    assert!(
        !p.contains("users/abc"),
        "deny must NOT be scoped to the narrow subdir: {p}"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn openhuman_internal_root_walks_up_to_dotopenhuman() {
    let r = openhuman_internal_root(std::path::Path::new(
        "/Users/x/.openhuman/users/id/workspace/memory",
    ));
    assert_eq!(r, std::path::Path::new("/Users/x/.openhuman"));
    // Fallback: no `.openhuman*` ancestor → returns the input.
    let r2 = openhuman_internal_root(std::path::Path::new("/tmp/custom/ws"));
    assert_eq!(r2, std::path::Path::new("/tmp/custom/ws"));
}

#[cfg(target_os = "macos")]
#[test]
fn seatbelt_available_honors_opt_out() {
    let _env = super::super::ENV_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let prev = std::env::var("OPENHUMAN_CLAUDE_CODE_SANDBOX").ok();
    std::env::set_var("OPENHUMAN_CLAUDE_CODE_SANDBOX", "0");
    assert!(
        !seatbelt_available(),
        "explicit opt-out must disable the jail"
    );
    match prev {
        Some(v) => std::env::set_var("OPENHUMAN_CLAUDE_CODE_SANDBOX", v),
        None => std::env::remove_var("OPENHUMAN_CLAUDE_CODE_SANDBOX"),
    }
}

#[test]
fn full_access_defaults_off_and_opts_in_via_env() {
    let _env = super::super::ENV_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    // Empty workspace (no persisted toggle) → file layer resolves to OFF.
    let ws = std::env::temp_dir().join("oh_cc_fullaccess_env_test");
    let _ = std::fs::remove_dir_all(&ws);
    let key = "OPENHUMAN_CLAUDE_CODE_PERMISSION_MODE";
    let prev = std::env::var(key).ok();
    std::env::remove_var(key);
    assert!(
        !claude_code_full_access(&ws),
        "default posture must be acceptEdits (full access OFF)"
    );
    std::env::set_var(key, "bypass");
    assert!(
        claude_code_full_access(&ws),
        "explicit opt-in (`bypass`) enables full access"
    );
    std::env::set_var(key, "acceptEdits");
    assert!(
        !claude_code_full_access(&ws),
        "acceptEdits env override keeps the default (limited) posture"
    );
    match prev {
        Some(v) => std::env::set_var(key, v),
        None => std::env::remove_var(key),
    }
}

#[test]
fn full_access_reads_persisted_toggle_when_env_unset() {
    use super::super::settings::{self, ClaudeCodeSettings};
    let _env = super::super::ENV_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let ws = std::env::temp_dir().join("oh_cc_fullaccess_file_test");
    let _ = std::fs::remove_dir_all(&ws);
    std::fs::create_dir_all(&ws).unwrap();
    let key = "OPENHUMAN_CLAUDE_CODE_PERMISSION_MODE";
    let prev = std::env::var(key).ok();
    std::env::remove_var(key);

    settings::save(&ws, &ClaudeCodeSettings { full_access: true }).unwrap();
    assert!(
        claude_code_full_access(&ws),
        "persisted toggle ON must enable full access when env is unset"
    );

    // Env override beats the persisted toggle.
    std::env::set_var(key, "acceptEdits");
    assert!(
        !claude_code_full_access(&ws),
        "env override OFF must beat a persisted ON toggle"
    );

    match prev {
        Some(v) => std::env::set_var(key, v),
        None => std::env::remove_var(key),
    }
    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn structured_error_survives_a_nonzero_exit() {
    // The regression: Claude writes the actionable error to stdout and exits
    // nonzero with an empty stderr, and the driver reported only stderr.
    let msg = failure_message(
        Some(1),
        Some("Failed to authenticate. API Error: 403 Request not allowed"),
        "",
    );
    assert!(
        msg.contains("403 Request not allowed"),
        "the provider error must reach the user, got: {msg}"
    );
    assert_ne!(msg, "exit Some(1) stderr=");
}

#[test]
fn structured_error_keeps_the_exit_code() {
    let msg = failure_message(Some(1), Some("boom"), "");
    assert!(msg.contains("exit Some(1)"), "got: {msg}");
}

#[test]
fn stderr_is_still_used_when_there_is_no_structured_error() {
    // A process-level failure — bad binary, signal — produces no error event.
    let msg = failure_message(Some(127), None, "  command not found\n");
    assert_eq!(msg, "exit Some(127) stderr=command not found");
}

#[test]
fn structured_error_wins_over_stderr_when_both_exist() {
    let msg = failure_message(Some(1), Some("quota exhausted"), "warning: deprecated flag");
    assert!(msg.starts_with("quota exhausted"), "got: {msg}");
    assert!(
        !msg.contains("deprecated flag"),
        "stderr noise must not bury the actionable error, got: {msg}"
    );
}

#[test]
fn structured_error_is_reported_even_on_a_clean_exit() {
    // Claude prints the actionable text on stdout and exits 0.
    let msg = turn_failure(
        true,
        Some(0),
        Some("API Error: 403 Request not allowed"),
        "",
        false,
    )
    .expect("a structured error on a clean exit is still a failure");
    assert!(msg.contains("403 Request not allowed"), "got: {msg}");
}

/// The case the `result.subtype=error` synthetic string used to stand in for.
/// Now that the mapper records the failure as a flag rather than as prose,
/// nothing else marks this turn as failed — drop `terminal_error` from the
/// decision and a semantic failure with a clean exit returns an empty success.
#[test]
fn a_terminal_failure_without_a_message_is_still_a_failure() {
    let msg = turn_failure(true, Some(0), None, "auth token expired", true)
        .expect("result.is_error must not be reported as success");
    assert_eq!(msg, "exit Some(0) stderr=auth token expired");

    assert!(
        turn_failure(true, Some(0), None, "", false).is_none(),
        "a clean turn with no failure signal must stay a success"
    );
}

#[test]
fn a_signalled_process_without_a_structured_error_still_reports_stderr() {
    let msg = failure_message(None, None, "Killed");
    assert_eq!(msg, "exit None stderr=Killed");
}

/// `{"type":"error","error":""}` is a real shape: the parser's
/// `unwrap_or("claude-code error")` only fires when the field is *missing*,
/// so an empty one arrives as `Some("")` and used to render as a leading
/// space where the diagnosis should be, with stderr thrown away.
#[test]
fn an_empty_structured_error_falls_back_to_stderr() {
    let msg = failure_message(Some(1), Some(""), "claude: command not found");
    assert_eq!(msg, "exit Some(1) stderr=claude: command not found");
}

#[test]
fn a_whitespace_only_structured_error_falls_back_to_stderr() {
    let msg = failure_message(Some(1), Some("   \n  "), "segmentation fault");
    assert_eq!(msg, "exit Some(1) stderr=segmentation fault");
}

/// The fallback must still be honest when there is nothing to fall back TO.
#[test]
fn an_empty_structured_error_with_empty_stderr_reports_the_exit_code() {
    let msg = failure_message(Some(1), Some(""), "");
    assert_eq!(msg, "exit Some(1) stderr=");
}

/// Surrounding whitespace on a real message is trimmed, not treated as absent.
#[test]
fn a_padded_structured_error_is_still_reported() {
    let msg = failure_message(Some(1), Some("  API Error: 403 Request not allowed\n"), "");
    assert_eq!(msg, "API Error: 403 Request not allowed (exit Some(1))");
}

// ── turn_failure: is this turn a failure at all? ─────────────────

#[test]
fn a_clean_turn_with_no_structured_error_is_not_a_failure() {
    assert_eq!(turn_failure(true, Some(0), None, "", false), None);
}

// The #5712 regression itself, asserted where the driver actually decides it:
// a nonzero exit must not discard the error Claude printed on stdout. The
// failure_message tests above cannot see this -- they call the formatter
// directly and never the branch that reaches it.
#[test]
fn a_structured_error_survives_a_nonzero_exit_at_the_decision() {
    let failure = turn_failure(
        false,
        Some(1),
        Some("Failed to authenticate. API Error: 403 Request not allowed"),
        "",
        false,
    )
    .expect("a nonzero exit is a failure");
    assert!(
        failure.contains("403 Request not allowed"),
        "got: {failure}"
    );
    assert_ne!(failure, "exit Some(1) stderr=");
}

#[test]
fn a_clean_exit_carrying_a_structured_error_is_still_a_failure() {
    // `result.subtype=error` sets mapper.error while the process exits 0.
    let failure = turn_failure(true, Some(0), Some("quota exhausted"), "", false)
        .expect("a structured error is a failure whatever the exit code");
    assert!(failure.contains("quota exhausted"), "got: {failure}");
}

#[test]
fn a_nonzero_exit_is_a_failure_even_without_a_structured_error() {
    let failure = turn_failure(false, Some(127), None, "command not found", false)
        .expect("a nonzero exit is a failure on its own");
    assert!(failure.contains("command not found"), "got: {failure}");
}

#[test]
fn a_blank_structured_error_on_a_clean_exit_is_still_a_failure() {
    // `{"type":"error","error":""}` arrives as `Some("")`. It is an error event,
    // so the turn failed; the message falls back to stderr because a blank
    // structured error diagnoses nothing.
    let failure = turn_failure(true, Some(0), Some(""), "socket closed", false)
        .expect("an error event is a failure even when it carries no text");
    assert_eq!(failure, "exit Some(0) stderr=socket closed");
}
