//! Wire-level characterization of the Docker sandbox: a fake `docker` binary on
//! `PATH` records the exact argv it was launched with, so these tests pin the
//! command line (and the exit/timeout/truncation behaviour) that
//! `docker_exec` produces, independent of which crate builds it. Real Docker is
//! never required.
#![cfg(unix)]

use super::*;
use crate::sandbox::types::DockerOverrides;
use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Install a fake `docker` that records argv (NUL separated) to `argv.out` and
/// then runs `body`. Returns (bin dir, argv file).
fn fake_docker(dir: &Path, body: &str) -> (PathBuf, PathBuf) {
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let out = dir.join("argv.out");
    let script = format!(
        "#!/bin/sh\n: > '{out}'\nfor a in \"$@\"; do printf '%s\\0' \"$a\" >> '{out}'; done\n{body}\n",
        out = out.display()
    );
    let path = bin.join("docker");
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    (bin, out)
}

fn recorded(out: &Path) -> Vec<String> {
    let raw = std::fs::read(out).unwrap();
    raw.split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect()
}

/// Run `f` with `bin` first on `PATH`, serialised against other env tests.
fn with_fake_path<T>(bin: &Path, f: impl FnOnce() -> T) -> T {
    let _guard = crate::config::TEST_ENV_LOCK.blocking_lock();
    let prev = std::env::var_os("PATH");
    let mut paths = vec![bin.to_path_buf()];
    if let Some(p) = &prev {
        paths.extend(std::env::split_paths(p));
    }
    std::env::set_var("PATH", std::env::join_paths(paths).unwrap());
    let result = f();
    match prev {
        Some(p) => std::env::set_var("PATH", p),
        None => std::env::remove_var("PATH"),
    }
    result
}

fn block_on<T>(fut: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(fut)
}

fn policy(workspace: &Path) -> SandboxPolicy {
    SandboxPolicy {
        backend: SandboxBackendKind::Docker,
        workspace_root: workspace.to_path_buf(),
        state_dir: PathBuf::from("/tmp/state"),
        read_only_mounts: vec![],
        read_write_mounts: vec![],
        allow_network: false,
        env_passthrough: vec![],
        docker_overrides: None,
    }
}

fn request(command: &str, timeout: Duration) -> SandboxExecRequest {
    SandboxExecRequest {
        command: command.into(),
        working_dir: PathBuf::from("/workspace"),
        env: Default::default(),
        timeout,
    }
}

fn run(
    dir: &Path,
    body: &str,
    policy: &SandboxPolicy,
    req: &SandboxExecRequest,
) -> (SandboxExecResult, Vec<String>) {
    let (bin, out) = fake_docker(dir, body);
    let result = with_fake_path(&bin, || block_on(docker_exec(policy, req))).unwrap();
    (result, recorded(&out))
}

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| (*x).to_string()).collect()
}

fn canon(p: &Path) -> String {
    p.canonicalize().unwrap().display().to_string()
}

#[test]
fn default_policy_argv_is_byte_identical() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("ws");
    std::fs::create_dir_all(&ws).unwrap();
    let (res, argv) = run(
        tmp.path(),
        "echo out; echo err >&2; exit 3",
        &policy(&ws),
        &request("echo hi && ls", Duration::from_secs(20)),
    );
    let mut expected = s(&[
        "run",
        "--rm",
        "--label",
        "openhuman.sandbox=true",
        "--network",
        "none",
        "--cap-drop",
        "ALL",
        "-m",
        "512m",
        "--cpus",
        "1",
        "--read-only",
        "--tmpfs",
        "/tmp:rw,noexec,nosuid,size=64m",
        "--tmpfs",
        "/var/tmp:rw,noexec,nosuid,size=64m",
        "--security-opt",
        "no-new-privileges",
        "-v",
    ]);
    expected.push(format!("{}:/workspace", canon(&ws)));
    expected.extend(s(&[
        "-w",
        "/workspace",
        "alpine:3.20",
        "sh",
        "-c",
        "echo hi && ls",
    ]));
    assert_eq!(argv, expected);
    assert_eq!(res.exit_code, 3);
    assert_eq!(res.stdout, "out\n");
    assert_eq!(res.stderr, "err\n");
    assert!(!res.timed_out);
}

#[test]
fn overrides_mounts_env_and_writable_rootfs_argv() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("ws");
    let ro = tmp.path().join("ro");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::create_dir_all(&ro).unwrap();
    let missing_ro = PathBuf::from("/definitely/not/here");
    let mut p = policy(&ws);
    p.read_only_mounts = vec![ro.clone(), missing_ro];
    p.env_passthrough = vec!["OH_DOCKER_TEST_SET".into(), "OH_DOCKER_TEST_UNSET".into()];
    p.docker_overrides = Some(DockerOverrides {
        image: Some("debian:12".into()),
        network: Some("bridge".into()),
        memory_limit_mb: Some(2048),
        cpu_limit: Some(2.5),
        read_only_rootfs: Some(false),
        extra_caps_drop: vec!["NET_RAW".into(), "MKNOD".into()],
    });
    let mut req = request("make test", Duration::from_secs(20));
    req.env
        .insert(OsString::from("REQ_KEY"), OsString::from("req value=1"));
    std::env::set_var("OH_DOCKER_TEST_SET", "pass-val");
    std::env::remove_var("OH_DOCKER_TEST_UNSET");
    let (res, argv) = run(tmp.path(), "exit 0", &p, &req);
    std::env::remove_var("OH_DOCKER_TEST_SET");
    let mut expected = s(&[
        "run",
        "--rm",
        "--label",
        "openhuman.sandbox=true",
        "--network",
        "bridge",
        "--cap-drop",
        "ALL",
        "--cap-drop",
        "NET_RAW",
        "--cap-drop",
        "MKNOD",
        "-m",
        "2048m",
        "--cpus",
        "2.5",
        "--security-opt",
        "no-new-privileges",
        "-v",
    ]);
    expected.push(format!("{}:/workspace", canon(&ws)));
    expected.extend(s(&["-w", "/workspace", "-v"]));
    expected.push(format!("{0}:{0}:ro", canon(&ro)));
    expected.extend(s(&["-v", "/definitely/not/here:/definitely/not/here:ro"]));
    expected.extend(s(&[
        "-e",
        "OH_DOCKER_TEST_SET=pass-val",
        "-e",
        "REQ_KEY=req value=1",
        "debian:12",
        "sh",
        "-c",
        "make test",
    ]));
    assert_eq!(argv, expected);
    assert_eq!(res.exit_code, 0);
}

#[test]
fn timeout_reports_timed_out_result() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("ws");
    std::fs::create_dir_all(&ws).unwrap();
    let (res, _argv) = run(
        tmp.path(),
        "exec sleep 5",
        &policy(&ws),
        &request("sleep 100", Duration::from_millis(300)),
    );
    assert!(res.timed_out);
    assert_eq!(res.exit_code, -1);
    assert!(res.stdout.is_empty());
    assert_eq!(res.stderr, "Command timed out after 0s and was killed");
}

#[test]
fn oversized_output_is_truncated_at_one_mebibyte() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("ws");
    std::fs::create_dir_all(&ws).unwrap();
    let (res, _argv) = run(
        tmp.path(),
        "head -c 1100000 /dev/zero | tr '\\0' 'a'; head -c 1100000 /dev/zero | tr '\\0' 'b' >&2",
        &policy(&ws),
        &request("big", Duration::from_secs(20)),
    );
    assert_eq!(
        res.stdout.len(),
        1_048_576 + "\n... [output truncated at 1MB]".len()
    );
    assert!(res.stdout.ends_with("\n... [output truncated at 1MB]"));
    assert_eq!(
        res.stderr.len(),
        1_048_576 + "\n... [stderr truncated at 1MB]".len()
    );
    assert!(res.stderr.ends_with("\n... [stderr truncated at 1MB]"));
}

#[test]
fn missing_docker_binary_is_an_execution_error() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("ws");
    std::fs::create_dir_all(&ws).unwrap();
    // Replace PATH wholesale so no `docker` can be found.
    let _guard = crate::config::TEST_ENV_LOCK.blocking_lock();
    let prev = std::env::var_os("PATH");
    std::env::set_var("PATH", tmp.path());
    let err = block_on(docker_exec(
        &policy(&ws),
        &request("true", Duration::from_secs(5)),
    ))
    .unwrap_err();
    if let Some(p) = prev {
        std::env::set_var("PATH", p);
    }
    assert!(
        err.to_string().starts_with("Docker execution failed: "),
        "{err}"
    );
}

#[test]
fn availability_probe_argv_and_status() {
    let tmp = tempfile::tempdir().unwrap();
    let (bin, out) = fake_docker(tmp.path(), "exit 0");
    assert!(with_fake_path(&bin, || block_on(is_docker_available())));
    assert_eq!(
        recorded(&out),
        s(&["info", "--format", "{{.ServerVersion}}"])
    );

    let tmp = tempfile::tempdir().unwrap();
    let (bin, _) = fake_docker(tmp.path(), "exit 1");
    assert!(!with_fake_path(&bin, || block_on(is_docker_available())));
}

#[test]
fn orphan_cleanup_lists_by_label_then_kills() {
    let tmp = tempfile::tempdir().unwrap();
    // `ps` prints two ids; `kill` is recorded last (argv.out is rewritten per call).
    let body = "if [ \"$1\" = ps ]; then echo abc; echo def; fi";
    let (bin, out) = fake_docker(tmp.path(), body);
    let n = with_fake_path(&bin, || block_on(cleanup_orphaned_containers())).unwrap();
    assert_eq!(n, 2);
    assert_eq!(recorded(&out), s(&["kill", "abc", "def"]));
}
