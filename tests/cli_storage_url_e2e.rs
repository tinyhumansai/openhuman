//! One-shot `openhuman-core` subcommands honor the configured storage URL
//! (`OPENHUMAN_STORAGE_URL`) the way the server does, instead of reading the
//! classic on-disk layout. The `storage` domain's URL rule is exercised
//! through the real binary.

use std::process::Command;

#[path = "support/storage_case.rs"]
mod storage_case;

use storage_case::UrlCase;

#[cfg(unix)]
fn make_workspace_private(workspace: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;

    std::fs::set_permissions(workspace, std::fs::Permissions::from_mode(0o700))
        .expect("make the test workspace private");
}

#[cfg(not(unix))]
fn make_workspace_private(_workspace: &std::path::Path) {}

fn core(workspace: &std::path::Path, url: Option<&str>, args: &[&str]) -> std::process::Output {
    make_workspace_private(workspace);
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_openhuman-core"));
    cmd.args(args)
        .env("HOME", workspace)
        .env("USERPROFILE", workspace)
        .env("OPENHUMAN_WORKSPACE", workspace)
        .env("OPENHUMAN_MODULE_SOURCE", "linked")
        // A shared backend keeps the config document encrypted under the
        // keyring master key and fails closed without one. CI has no OS
        // keychain, so every process gets the same fixed key, the way a
        // headless deployment would.
        .env("OPENHUMAN_KEYRING_MASTER_KEY", "22".repeat(32))
        .env_remove("OPENHUMAN_KEYRING_MASTER_KEY_FILE")
        .env_remove("OPENHUMAN_STORAGE_URL");
    if let Some(url) = url {
        cmd.env("OPENHUMAN_STORAGE_URL", url);
    }
    cmd.output().expect("run openhuman-core")
}

#[test]
fn a_one_shot_command_refuses_a_storage_url_it_cannot_open() {
    let tmp = tempfile::tempdir().unwrap();
    let output = core(tmp.path(), Some("nonsense://nowhere"), &["cron", "list"]);
    assert!(!output.status.success(), "a bad URL must fail the command");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("storage"), "{stderr}");
}

#[test]
fn a_one_shot_command_opens_a_valid_storage_url_and_help_does_not_need_one() {
    let tmp = tempfile::tempdir().unwrap();
    let output = core(tmp.path(), Some("memory"), &["cron", "list"]);
    assert!(output.status.success(), "a valid URL must open the backend");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("opening the configured storage backend"),
        "{stderr}"
    );
    // Help never opens the backend, even a bad one.
    let help = core(tmp.path(), Some("nonsense://nowhere"), &["--help"]);
    assert!(help.status.success());
    let nested = core(
        tmp.path(),
        Some("nonsense://nowhere"),
        &["cron", "list", "--help"],
    );
    assert!(nested.status.success(), "nested help never opens storage");
}

/// Runs `args` as a one-shot in its own workspace (so the classic on-disk
/// layout cannot be what two runs share) against `url`.
fn one_shot(url: &str, args: &[&str]) -> (std::process::Output, tempfile::TempDir) {
    let workspace = tempfile::tempdir().unwrap();
    let output = core(workspace.path(), Some(url), args);
    (output, workspace)
}

/// Write a job in one process, list in another: the backend is what carries
/// it from the first to the second.
#[test]
fn a_second_process_reads_what_a_one_shot_wrote_to_the_backend() {
    let case = UrlCase::open();
    let args = [
        "cron",
        "add",
        "--name",
        "storage-probe",
        "--schedule",
        r#"{"kind":"every","every_ms":3600000}"#,
        "--command",
        "echo probe",
    ];
    let (added, _first) = one_shot(&case.url, &args);
    let stderr = String::from_utf8_lossy(&added.stderr);
    assert!(
        added.status.success(),
        "cron add against {}: {stderr}",
        case.url
    );

    // A different workspace: only the backend can hold the job.
    let (listed, _second) = one_shot(&case.url, &["cron", "list"]);
    assert!(
        listed.status.success(),
        "{}",
        String::from_utf8_lossy(&listed.stderr)
    );
    let stdout = String::from_utf8_lossy(&listed.stdout);
    assert!(
        stdout.contains("storage-probe"),
        "second process sees the job: {stdout}"
    );

    // And without the URL the same fresh workspace has nothing.
    let workspace = tempfile::tempdir().unwrap();
    let classic = core(workspace.path(), None, &["cron", "list"]);
    assert!(
        classic.status.success(),
        "{}",
        String::from_utf8_lossy(&classic.stderr)
    );
    assert!(
        !String::from_utf8_lossy(&classic.stdout).contains("storage-probe"),
        "the job lives in the backend, not in files"
    );
}
