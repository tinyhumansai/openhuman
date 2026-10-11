//! Docker sandbox request mapping and Docker CLI health/cleanup probes.
//! Execution mapping is tested without a daemon; TinyBox owns command-line
//! construction, execution, and output limits.
#![cfg(unix)]

use super::*;
#[cfg(feature = "modules")]
use crate::sandbox::types::DockerOverrides;
#[cfg(feature = "modules")]
use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
#[cfg(feature = "modules")]
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

#[cfg(feature = "modules")]
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

#[cfg(feature = "modules")]
fn request(command: &str, timeout: Duration) -> SandboxExecRequest {
    SandboxExecRequest {
        command: command.into(),
        working_dir: PathBuf::from("/workspace"),
        env: Default::default(),
        timeout,
    }
}

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| (*x).to_string()).collect()
}

#[cfg(feature = "modules")]
#[test]
fn module_request_preserves_policy_and_only_forwards_allowed_environment() {
    use crate::modules::tinybox::contract::DockerRunRequest;

    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    let readonly = temp.path().join("readonly");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::create_dir_all(&readonly).unwrap();

    let mut sandbox_policy = policy(&workspace);
    sandbox_policy.read_only_mounts.push(readonly.clone());
    sandbox_policy.env_passthrough = vec!["OH_TINYBOX_ALLOWED_ENV".to_owned()];
    sandbox_policy.docker_overrides = Some(DockerOverrides {
        image: Some("debian:12".to_owned()),
        network: Some("bridge".to_owned()),
        memory_limit_mb: Some(2048),
        cpu_limit: Some(2.5),
        read_only_rootfs: Some(false),
        extra_caps_drop: vec!["NET_RAW".to_owned()],
    });

    let _environment = crate::config::TEST_ENV_LOCK.blocking_lock();
    std::env::set_var("OH_TINYBOX_ALLOWED_ENV", "host value");
    std::env::set_var("OH_TINYBOX_UNLISTED_ENV", "must not be forwarded");
    let mut exec_request = request("make test", Duration::from_millis(1250));
    exec_request.env.insert(
        OsString::from("OH_TINYBOX_ALLOWED_ENV"),
        OsString::from("request override"),
    );
    exec_request.env.insert(
        OsString::from("REQUEST_ENV"),
        OsString::from("request value"),
    );

    let actual = build_docker_request(&sandbox_policy, &exec_request).unwrap();
    std::env::remove_var("OH_TINYBOX_ALLOWED_ENV");
    std::env::remove_var("OH_TINYBOX_UNLISTED_ENV");
    assert_eq!(
        actual,
        DockerRunRequest {
            workspace: workspace.to_string_lossy().into_owned(),
            image: Some("debian:12".to_owned()),
            command: "make test".to_owned(),
            network: Some("bridge".to_owned()),
            memory_limit_mb: Some(2048),
            cpus: Some(2.5),
            read_only_rootfs: Some(false),
            read_only_mounts: vec![readonly.to_string_lossy().into_owned()],
            extra_cap_drops: vec!["NET_RAW".to_owned()],
            env: [
                (
                    "OH_TINYBOX_ALLOWED_ENV".to_owned(),
                    "request override".to_owned()
                ),
                ("REQUEST_ENV".to_owned(), "request value".to_owned()),
            ]
            .into(),
            label: Some("openhuman.sandbox=true".to_owned()),
            timeout_ms: 1250,
        }
    );
}

#[cfg(feature = "modules")]
#[test]
fn module_request_rounds_small_timeouts_up_and_rejects_unbounded_timeouts() {
    use crate::modules::tinybox::contract::MAX_DOCKER_RUN_TIMEOUT_MS;

    let temp = tempfile::tempdir().unwrap();
    let mut exec_request = request("true", Duration::from_nanos(1));
    let short = build_docker_request(&policy(temp.path()), &exec_request).unwrap();
    assert_eq!(short.timeout_ms, 1);

    exec_request.timeout = Duration::from_millis(MAX_DOCKER_RUN_TIMEOUT_MS + 1);
    let error = build_docker_request(&policy(temp.path()), &exec_request)
        .expect_err("unbounded request must not bypass the contract limit");
    assert!(error
        .to_string()
        .contains("timeout exceeds the supported limit"));
}

#[cfg(all(feature = "modules", unix))]
#[test]
fn module_request_rejects_non_utf8_host_paths_without_lossy_remapping() {
    use std::os::unix::ffi::OsStringExt;

    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join(OsString::from_vec(vec![b'w', 0xff]));
    let error = build_docker_request(&policy(&path), &request("true", Duration::from_secs(1)))
        .expect_err("the module contract cannot represent an arbitrary OS path");
    assert!(error.to_string().contains("workspace path is not UTF-8"));
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
