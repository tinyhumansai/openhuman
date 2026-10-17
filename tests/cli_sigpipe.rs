#[cfg(unix)]
#[test]
fn help_output_closed_pipe_does_not_panic() {
    use std::io::{BufRead, BufReader};
    use std::os::unix::process::ExitStatusExt;
    use std::process::{Command, Stdio};

    let mut child = Command::new(env!("CARGO_BIN_EXE_openhuman-core"))
        .arg("--help")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn openhuman-core --help");

    // Mirror the issue repro (`openhuman-core --help | head -n 1`): read a
    // single line, then drop the read end mid-stream so the child's next write
    // lands on a closed pipe. Closing before reading (as a naive test does)
    // lets the child buffer its entire few-KB `--help` output in one successful
    // write well under the 64 KB pipe buffer and exit cleanly, never exercising
    // the broken-pipe path — a false green that passes even without the fix.
    let stdout = child.stdout.take().expect("capture stdout");
    let mut reader = BufReader::new(stdout);
    let mut first_line = String::new();
    reader
        .read_line(&mut first_line)
        .expect("read first help line");
    drop(reader);

    let output = child.wait_with_output().expect("wait for openhuman-core");
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        !stderr.contains("Broken pipe"),
        "stderr must not include a broken-pipe panic: {stderr}"
    );
    assert!(
        !stderr.contains("panicked"),
        "stderr must not include a panic report: {stderr}"
    );

    // Acceptance criterion #1: a closed downstream reader must yield a clean
    // exit — either the child finished before the pipe closed, or the restored
    // default disposition let SIGPIPE terminate it. A normal-code crash (e.g.
    // the panic exit code 101) is precisely the regression this guards against.
    assert!(
        output.status.success() || output.status.signal() == Some(libc::SIGPIPE),
        "process must exit cleanly or via SIGPIPE, got {:?}",
        output.status
    );
}

#[test]
fn help_does_not_require_a_keyring_master_key() {
    use std::process::Command;

    let workspace = tempfile::tempdir().unwrap();
    for args in [Vec::<&str>::new(), vec!["--help"]] {
        let output = Command::new(env!("CARGO_BIN_EXE_openhuman-core"))
            .args(args)
            .current_dir(workspace.path())
            .env("OPENHUMAN_WORKSPACE", workspace.path())
            .env("OPENHUMAN_KEYRING_MASTER_KEY", "invalid")
            .env_remove("OPENHUMAN_KEYRING_MASTER_KEY_FILE")
            .env_remove("OPENHUMAN_APP_ENV")
            .env_remove("OPENHUMAN_KEYRING_BACKEND")
            .env_remove("OPENHUMAN_MODE")
            .output()
            .expect("run help-only CLI invocation");
        assert!(
            output.status.success(),
            "help should not initialize the keyring: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn headless_default_keyring_stores_provider_secrets_encrypted() {
    use std::process::Command;

    let workspace = tempfile::tempdir().expect("temporary OpenHuman workspace");
    // macOS exposes the system temporary directory through aliases under
    // `/var`; the secrets driver deliberately refuses symlinked namespace
    // components, so pass the canonical workspace path to the child.
    let workspace_path = std::fs::canonicalize(workspace.path())
        .expect("canonicalize temporary OpenHuman workspace");
    let secret = "provider-secret-e2e";
    let params = format!(r#"{{"provider":"openai","token":"{secret}"}}"#);
    let output = Command::new(env!("CARGO_BIN_EXE_openhuman-core"))
        .args([
            "call",
            "--method",
            "openhuman.auth_store_provider_credentials",
            "--params",
            &params,
        ])
        .current_dir(&workspace_path)
        .env("OPENHUMAN_WORKSPACE", &workspace_path)
        .env("OPENHUMAN_KEYRING_MASTER_KEY", "42".repeat(32))
        .env_remove("OPENHUMAN_KEYRING_MASTER_KEY_FILE")
        .env_remove("OPENHUMAN_APP_ENV")
        .env_remove("OPENHUMAN_KEYRING_BACKEND")
        .env_remove("OPENHUMAN_MODE")
        .output()
        .expect("run provider credential store command");
    assert!(
        output.status.success(),
        "credential store failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Since the storage ports (#7686) a configured backend (the CLI default is
    // `sqlite:<workspace>/storage`) holds user secrets as encrypted `enc2:`
    // documents (`storage/secrets.rs`) instead of `secrets.enc`. Assert the
    // property, not the file name: the plaintext is nowhere under the
    // workspace, and an encrypted record is.
    let files = files_under(&workspace_path);
    let mut saw_encrypted_record = false;
    for path in &files {
        let bytes = std::fs::read(path).expect("read workspace file");
        assert!(
            !bytes
                .windows(secret.len())
                .any(|window| window == secret.as_bytes()),
            "provider secret stored in plaintext at {}",
            path.display()
        );
        saw_encrypted_record |= bytes.windows(5).any(|window| window == b"enc2:");
    }
    assert!(
        saw_encrypted_record,
        "no encrypted secret record under the workspace; files: {files:?}"
    );

    // And the stored credential reads back in a fresh process.
    let listed = Command::new(env!("CARGO_BIN_EXE_openhuman-core"))
        .args([
            "call",
            "--method",
            "openhuman.auth_list_provider_credentials",
            "--params",
            r#"{"provider":"openai"}"#,
        ])
        .current_dir(&workspace_path)
        .env("OPENHUMAN_WORKSPACE", &workspace_path)
        .env("OPENHUMAN_KEYRING_MASTER_KEY", "42".repeat(32))
        .env_remove("OPENHUMAN_KEYRING_MASTER_KEY_FILE")
        .env_remove("OPENHUMAN_APP_ENV")
        .env_remove("OPENHUMAN_KEYRING_BACKEND")
        .env_remove("OPENHUMAN_MODE")
        .output()
        .expect("run provider credential list command");
    let stdout = String::from_utf8_lossy(&listed.stdout);
    let listing: serde_json::Value = serde_json::from_str(&stdout).unwrap_or_default();
    // The profile must come back with its token, not just its metadata: a lost
    // token still lists the profile with `hasToken: false`.
    assert!(
        listed.status.success() && lists_openai_token(&listing),
        "stored credential not read back with its token: stdout={stdout} stderr={}",
        String::from_utf8_lossy(&listed.stderr)
    );
}

/// Whether `value` holds the `openai` profile summary with its token present.
fn lists_openai_token(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(map) => {
            (map.get("provider").and_then(serde_json::Value::as_str) == Some("openai")
                && map.get("hasToken").and_then(serde_json::Value::as_bool) == Some(true))
                || map.values().any(lists_openai_token)
        }
        serde_json::Value::Array(items) => items.iter().any(lists_openai_token),
        _ => false,
    }
}

/// Every regular file under `root`, recursively.
fn files_under(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    let mut dirs = vec![root.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).expect("read workspace dir") {
            let path = entry.expect("workspace dir entry").path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.is_file() {
                files.push(path);
            }
        }
    }
    files
}
