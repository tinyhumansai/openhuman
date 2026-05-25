//! End-to-end tests for `openhuman::cwd_jail`.
//!
//! Each test goes through the public surface only — `Jail`, `spawn`,
//! `JailRegistry`, `default_backend` — and (where the platform allows it)
//! actually exercises the OS sandbox by trying to do something it should
//! be blocked from doing.
//!
//! Platform breakdown:
//! - **Common** (all OSes): registry CRUD + spawn via `NoopBackend`, jail
//!   builder semantics. Runs in every CI matrix slot.
//! - **Linux**: `target_os = "linux"` gate exercises Landlock by spawning
//!   `/bin/sh` and trying to write outside the jail.
//! - **macOS**: same shape, exercises Seatbelt via `/usr/bin/touch`.
//! - **Windows**: AppContainer integration is marked `#[ignore]` until
//!   the raw-`HANDLE` → `Child` bridge lands (see TODO in
//!   `src/openhuman/cwd_jail/windows.rs`).

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use openhuman_core::openhuman::cwd_jail::{
    default_backend, spawn, spawn_with, Jail, JailRegistry, NoopBackend,
};

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

// ── Common: runs on every platform ──────────────────────────────────

#[test]
fn registry_full_lifecycle_with_noop() {
    let base = unique_tempdir("lifecycle");
    let reg = JailRegistry::open(&base).unwrap();

    // Create several jails in parallel.
    let a = reg.create("agent-a").unwrap();
    let b = reg.create("agent-b").unwrap();
    let c = reg.create("agent-c").unwrap();
    assert_eq!(reg.list().len(), 3);

    // Rename + notes update timestamps.
    let renamed = reg.rename(&a.id, "agent-a-renamed").unwrap();
    assert_eq!(renamed.label, "agent-a-renamed");
    let noted = reg
        .set_notes(&a.id, Some("owner=stevent95".into()))
        .unwrap();
    assert_eq!(noted.notes.as_deref(), Some("owner=stevent95"));

    // Spawn through the registry into one of the jails.
    let mut cmd = noop_exit_zero_cmd();
    cmd.stdout(Stdio::null()).stderr(Stdio::null());
    let mut child = reg.spawn_in_with(&b.id, &NoopBackend, cmd).unwrap();
    let status = child.wait().unwrap();
    assert!(status.success() || cfg!(windows));

    // Delete one, clear the rest.
    reg.delete(&c.id).unwrap();
    assert!(reg.get(&c.id).is_none());
    let cleared = reg.clear().unwrap();
    assert_eq!(cleared, 2);
    assert!(reg.list().is_empty());

    // Reopen — empty index round-trips.
    drop(reg);
    let reg2 = JailRegistry::open(&base).unwrap();
    assert!(reg2.list().is_empty());

    fs::remove_dir_all(&base).ok();
}

#[test]
fn jail_canonicalize_rejects_missing_root() {
    let jail = Jail::new("/does/not/exist/at/all", "missing");
    let err = spawn_with(&NoopBackend, &jail, noop_exit_zero_cmd()).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
}

#[test]
fn default_backend_is_named_and_available() {
    let b = default_backend();
    assert!(!b.name().is_empty());
    // On every supported platform the auto-detected backend should be
    // available — even noop returns true.
    assert!(b.is_available());
}

#[test]
fn jail_builder_carries_intent_through_clone() {
    // `spawn` clones the jail before canonicalize; verify a chained
    // builder still produces the right shape.
    let dir = unique_tempdir("builder");
    let j = Jail::new(&dir, "build")
        .add_read_only("/usr/lib")
        .add_read_only("/usr/share")
        .deny_net()
        .deny_subprocess();
    assert_eq!(j.read_only.len(), 2);
    assert!(!j.allow_net);
    assert!(!j.allow_subprocess);
    fs::remove_dir_all(&dir).ok();
}

// ── Linux: Landlock real-sandbox enforcement ────────────────────────

#[cfg(all(target_os = "linux", feature = "sandbox-landlock"))]
#[test]
fn linux_landlock_blocks_write_outside_root() {
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

#[cfg(all(target_os = "linux", feature = "sandbox-landlock"))]
#[test]
fn linux_landlock_allows_write_inside_root() {
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

// ── Helpers ─────────────────────────────────────────────────────────

fn noop_exit_zero_cmd() -> Command {
    if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.args(["/C", "exit"]);
        c
    } else {
        Command::new("true")
    }
}
