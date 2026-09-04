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

/// The turn budget is the difference between "the CLI is still working" and
/// "the CLI is broken", so its parse rules are worth pinning. Exercised through
/// the pure helper rather than the env var: an env-mutating test races every
/// other test in this binary.
#[test]
fn turn_timeout_defaults_when_unset_or_unparseable() {
    let default = Duration::from_secs(DEFAULT_TURN_TIMEOUT_SECS);

    assert_eq!(parse_turn_timeout(None), default);
    assert_eq!(parse_turn_timeout(Some("")), default);
    assert_eq!(parse_turn_timeout(Some("not-a-number")), default);
    assert_eq!(parse_turn_timeout(Some("-30")), default);
}

/// Zero is rejected rather than honoured — it would kill every child the
/// instant it started, which reads as a broken CLI rather than a bad setting.
#[test]
fn turn_timeout_rejects_zero_and_honours_a_real_override() {
    assert_eq!(
        parse_turn_timeout(Some("0")),
        Duration::from_secs(DEFAULT_TURN_TIMEOUT_SECS)
    );
    assert_eq!(parse_turn_timeout(Some("  120 ")), Duration::from_secs(120));
}

/// Only a spawn failure meaning "this binary is unusable" may carry the marker,
/// because the marker classifies as a NON-RETRYABLE setup problem. ETXTBSY (the
/// binary is being rewritten) and EAGAIN (fork pressure) are transient, and
/// telling the user to reinstall would both mislead and suppress the retry that
/// would have worked.
#[test]
fn only_permanent_spawn_failures_claim_the_setup_marker() {
    const MARKER: &str = "[claude-code] `claude` CLI";
    let path = std::path::Path::new("/usr/local/bin/claude");

    for kind in [
        std::io::ErrorKind::NotFound,
        std::io::ErrorKind::PermissionDenied,
    ] {
        let err = spawn_error(kind, path, &"boom");
        assert!(
            err.to_string().contains(MARKER),
            "{kind:?} is a broken install and must classify as provider_setup"
        );
    }

    for kind in [
        std::io::ErrorKind::ResourceBusy,
        std::io::ErrorKind::WouldBlock,
        std::io::ErrorKind::Interrupted,
    ] {
        let err = spawn_error(kind, path, &"boom");
        assert!(
            !err.to_string().contains(MARKER),
            "{kind:?} is transient and must stay retryable"
        );
    }
}
