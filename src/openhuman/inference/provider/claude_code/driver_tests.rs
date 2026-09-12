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
fn child_path_prepends_cli_dir_and_keeps_inherited_entries() {
    let _env = super::super::ENV_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    struct PathRestore(Option<std::ffi::OsString>);
    impl Drop for PathRestore {
        fn drop(&mut self) {
            match self.0.take() {
                Some(path) => std::env::set_var("PATH", path),
                None => std::env::remove_var("PATH"),
            }
        }
    }

    let _restore = PathRestore(std::env::var_os("PATH"));
    let inherited = std::env::join_paths([
        std::path::Path::new("/usr/bin"),
        std::path::Path::new("/bin"),
    ])
    .expect("valid test paths");
    std::env::set_var("PATH", inherited);

    let combined = child_path_with_user_bins(std::path::Path::new("/Users/test/.local/bin/claude"));
    let dirs: Vec<PathBuf> = std::env::split_paths(&combined).collect();

    assert_eq!(
        dirs.first().map(|p| p.as_path()),
        Some(std::path::Path::new("/Users/test/.local/bin"))
    );
    assert!(dirs.iter().any(|p| p == std::path::Path::new("/usr/bin")));
    assert!(dirs.iter().any(|p| p == std::path::Path::new("/bin")));
    let cli_idx = dirs
        .iter()
        .position(|p| p == std::path::Path::new("/Users/test/.local/bin"))
        .expect("CLI directory");
    let usr_idx = dirs
        .iter()
        .position(|p| p == std::path::Path::new("/usr/bin"))
        .expect("inherited directory");
    assert!(
        cli_idx < usr_idx,
        "CLI dir must come before inherited /usr/bin"
    );
    assert!(!dirs.iter().any(|p| p.as_os_str().is_empty()));
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

/// `String::truncate` takes a byte index and panics when it is not a character
/// boundary, so bounding the stderr accumulator with `acc.truncate(16_384)`
/// aborted the drain task whenever a multi-byte character straddled the cap.
/// The join is `unwrap_or_default()`, so the operator saw `stderr=` and lost
/// the whole error output for that turn.
#[test]
fn push_bounded_never_splits_a_character() {
    // "aéb" is 4 bytes: a=0, é=1..2, b=3. A cap of 2 lands *inside* 'é', so the
    // helper must back up to byte 1 -- `String::truncate(2)` would panic here.
    let mut acc = String::new();
    super::push_bounded(&mut acc, "aéb", 2);
    assert_eq!(acc, "a", "must back up to the boundary, not split it");

    // A cap of 3 lands exactly on a boundary, so nothing is given up needlessly.
    let mut acc = String::new();
    super::push_bounded(&mut acc, "aéb", 3);
    assert_eq!(acc, "aé");
    assert!(acc.len() <= 3);

    // The same, driven the way the reader does it: many small chunks over the cap.
    let mut acc = String::new();
    for _ in 0..600 {
        super::push_bounded(&mut acc, "日本語テキスト", 1024);
    }
    assert!(acc.len() <= 1024);
    // The real assertion: it is still valid UTF-8 and did not panic getting here.
    assert!(std::str::from_utf8(acc.as_bytes()).is_ok());
}

#[test]
fn push_bounded_keeps_everything_below_the_cap() {
    let mut acc = String::new();
    super::push_bounded(&mut acc, "hello ", 64);
    super::push_bounded(&mut acc, "world", 64);
    assert_eq!(acc, "hello world");
}

#[test]
fn push_bounded_handles_an_ascii_cap_exactly() {
    let mut acc = String::new();
    super::push_bounded(&mut acc, "abcdef", 3);
    assert_eq!(acc, "abc");
}

#[test]
fn parse_error_events_produce_a_log_line() {
    let ev = ClaudeCodeEvent::ParseError {
        line: "{not json".to_string(),
        reason: "expected value at line 1 column 2".to_string(),
    };
    let msg = parse_error_log_line(&ev).expect("a ParseError must be reported");
    assert!(msg.contains("expected value at line 1 column 2"), "{msg}");
    assert!(msg.contains("9 bytes"), "{msg}");
}

#[test]
fn other_events_produce_nothing() {
    let ev = ClaudeCodeEvent::Error {
        message: "boom".to_string(),
    };
    assert!(parse_error_log_line(&ev).is_none());
}

/// An unparsable line can be a well-formed event of an unknown type, so it
/// can hold the prompt, the reply, or a credential. None of it is quoted.
#[test]
fn the_line_itself_is_never_quoted() {
    let ev = ClaudeCodeEvent::ParseError {
        line: r#"{"type":"secret_leak","api_key":"sk-ant-not-in-the-log"}"#.to_string(),
        reason: "unknown event type `secret_leak`".to_string(),
    };
    let msg = parse_error_log_line(&ev).unwrap();
    assert!(!msg.contains("sk-ant-not-in-the-log"), "{msg}");
    assert!(!msg.contains("api_key"), "{msg}");
}

/// Size is reported instead of content, so a truncated stream still reads
/// differently from a chatty one.
#[test]
fn the_size_of_the_line_is_reported() {
    let ev = ClaudeCodeEvent::ParseError {
        line: "x".repeat(5_000),
        reason: "trailing characters".to_string(),
    };
    assert!(parse_error_log_line(&ev).unwrap().contains("5000 bytes"));
}

#[test]
fn the_shape_of_the_line_is_reported() {
    let shape = |line: &str| {
        parse_error_log_line(&ClaudeCodeEvent::ParseError {
            line: line.to_string(),
            reason: "r".to_string(),
        })
        .unwrap()
    };
    assert!(shape(r#"  {"type":"x"}"#).contains("json object"));
    assert!(shape("[1,2]").contains("json array"));
    assert!(shape("panic: claude-code crashed").contains("non-json"));
    assert!(shape("   ").contains("blank"));
}
