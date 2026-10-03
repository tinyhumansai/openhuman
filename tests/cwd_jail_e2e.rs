//! End-to-end tests for `openhuman::sandbox::cwd_jail`.
//!
//! Each test goes through the public surface only (`Jail`, `spawn`) and
//! actually exercises the OS sandbox by trying to do something it should be
//! blocked from doing. Registry, backend-selection and builder semantics are
//! covered by `vendor/tinybox/crates/tinybox-jail` unit tests.
//!
//! Platform breakdown:
//! - **Linux**: `target_os = "linux"` gate exercises Landlock by spawning
//!   `/bin/sh` and trying to write outside the jail.
//! - **macOS**: same shape, exercises Seatbelt via `/usr/bin/touch`.
//! - **Windows**: AppContainer integration is marked `#[ignore]` until
//!   the raw-`HANDLE` → `Child` bridge lands (see TODO in
//!   `crates/openhuman-core/src/cwd_jail/windows.rs`).

#![allow(dead_code, unused_imports)]

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
use openhuman_core::sandbox::cwd_jail::spawn;
use openhuman_core::sandbox::cwd_jail::Jail;

fn unique_tempdir(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!(
        "openhuman-e2e-{}-{}-{}",
        tag,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
    ));
    fs::create_dir_all(&p).unwrap();
    p
}

// ── Linux: Landlock real-sandbox enforcement ────────────────────────
//
// Landlock is always compiled in on Linux (tinybox-jail enables it by default),
// but the running kernel may not support it; then `spawn` answers
// `Unsupported` rather than running the command unconfined, and these tests
// skip.

#[cfg(target_os = "linux")]
fn landlock_in_force() -> bool {
    let ok = openhuman_core::sandbox::cwd_jail::default_backend().name() == "landlock";
    if !ok {
        eprintln!("SKIP: this kernel has no Landlock");
    }
    ok
}

#[cfg(target_os = "linux")]
#[test]
fn linux_landlock_blocks_write_outside_root() {
    if !landlock_in_force() {
        return;
    }
    let root = unique_tempdir("ll-root");
    let outside = unique_tempdir("ll-outside");
    let outside_target = outside.join("forbidden.txt");

    let jail = Jail::new(&root, "e2e.landlock");
    let mut cmd = Command::new("/bin/sh");
    cmd.arg("-c")
        .arg(format!("echo hi > {}", outside_target.display()))
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    let mut child = spawn(&jail, cmd).expect("spawn under landlock");
    let _ = child.wait().expect("wait");

    assert!(
        !outside_target.exists(),
        "Landlock failed to block write to {}",
        outside_target.display()
    );
    fs::remove_dir_all(&root).ok();
    fs::remove_dir_all(&outside).ok();
}

#[cfg(target_os = "linux")]
#[test]
fn linux_landlock_allows_write_inside_root() {
    if !landlock_in_force() {
        return;
    }
    let root = unique_tempdir("ll-root-write");
    let inside = root.join("ok.txt");

    let jail = Jail::new(&root, "e2e.landlock.ok");
    let mut cmd = Command::new("/bin/sh");
    cmd.arg("-c")
        .arg(format!("echo hi > {}", inside.display()))
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    let mut child = spawn(&jail, cmd).expect("spawn under landlock");
    let status = child.wait().expect("wait");
    assert!(status.success(), "write inside root should succeed");
    assert!(inside.exists());
    fs::remove_dir_all(&root).ok();
}

// ── macOS: Seatbelt real-sandbox enforcement ────────────────────────

#[cfg(target_os = "macos")]
#[test]
fn macos_seatbelt_blocks_write_outside_root() {
    if !PathBuf::from("/usr/bin/sandbox-exec").exists() {
        return;
    }
    let root = unique_tempdir("sb-root");
    let outside =
        std::env::temp_dir().join(format!("openhuman-e2e-sb-forbidden-{}", std::process::id()));
    let _ = fs::remove_file(&outside);

    let jail = Jail::new(&root, "e2e.seatbelt");
    let mut cmd = Command::new("/usr/bin/touch");
    cmd.arg(&outside)
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    let mut child = spawn(&jail, cmd).expect("spawn under seatbelt");
    let _ = child.wait().expect("wait");

    assert!(
        !outside.exists(),
        "Seatbelt failed to block write to {}",
        outside.display()
    );
    fs::remove_dir_all(&root).ok();
}

#[cfg(target_os = "macos")]
#[test]
fn macos_seatbelt_allows_write_inside_root() {
    if !PathBuf::from("/usr/bin/sandbox-exec").exists() {
        return;
    }
    let root = unique_tempdir("sb-root-ok");
    let inside = root.join("ok.txt");

    let jail = Jail::new(&root, "e2e.seatbelt.ok");
    let mut cmd = Command::new("/usr/bin/touch");
    cmd.arg(&inside).stdout(Stdio::null()).stderr(Stdio::null());

    let mut child = spawn(&jail, cmd).expect("spawn under seatbelt");
    let status = child.wait().expect("wait");
    assert!(status.success(), "writing inside root should succeed");
    assert!(inside.exists());
    fs::remove_dir_all(&root).ok();
}

#[cfg(target_os = "macos")]
#[test]
fn macos_seatbelt_allows_shell_redirection_to_dev_null() {
    if !PathBuf::from("/usr/bin/sandbox-exec").exists() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let jail = Jail::new(root.path(), "e2e.seatbelt.null");
    let mut cmd = Command::new("/bin/sh");
    cmd.arg("-c")
        .arg(
            "set -e; echo ignored >/dev/null; echo hidden 2>/dev/null >&2; echo completed > output",
        )
        .current_dir(root.path())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    let mut child = spawn(&jail, cmd).expect("spawn under seatbelt");
    assert!(child.wait().expect("wait").success());
    assert_eq!(
        fs::read_to_string(root.path().join("output")).unwrap(),
        "completed\n"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn macos_seatbelt_blocks_network_when_denied() {
    if !PathBuf::from("/usr/bin/sandbox-exec").exists() {
        return;
    }
    // `nc -z 1.1.1.1 80` is a simple connect probe. Under deny_net the
    // sandbox should refuse the socket; under allow_net (default) it may
    // succeed *or* fail depending on environment, so we only assert the
    // deny side here.
    let root = unique_tempdir("sb-net");
    let jail = Jail::new(&root, "e2e.seatbelt.nonet").deny_net();
    let mut cmd = Command::new("/bin/sh");
    cmd.arg("-c")
        .arg("/usr/bin/nc -z -w 1 1.1.1.1 80 2>/dev/null && echo OPEN")
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let child = spawn(&jail, cmd).expect("spawn under seatbelt");
    let out = child.wait_with_output().expect("wait");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("OPEN"),
        "Seatbelt failed to block network: stdout={stdout:?}"
    );
    fs::remove_dir_all(&root).ok();
}

// ── Windows: AppContainer (gated until HANDLE→Child bridge lands) ───

#[cfg(target_os = "windows")]
#[test]
#[ignore = "AppContainer spawn returns Unsupported pending Child handle bridge; see windows.rs TODO"]
fn windows_appcontainer_blocks_write_outside_root() {
    let root = unique_tempdir("ac-root");
    let outside = std::env::temp_dir().join(format!(
        "openhuman-e2e-ac-forbidden-{}.txt",
        std::process::id()
    ));
    let _ = fs::remove_file(&outside);

    let jail = Jail::new(&root, "e2e.appcontainer");
    let mut cmd = Command::new("cmd");
    cmd.args(["/C", &format!("echo hi > \"{}\"", outside.display())]);

    // Once the Child bridge is implemented, flip this from `unwrap_err`
    // to `wait` + assert(!outside.exists()).
    let err = spawn(&jail, cmd).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::Unsupported);
    fs::remove_dir_all(&root).ok();
}
