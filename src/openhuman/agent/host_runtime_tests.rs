use super::*;
use crate::openhuman::config::{DockerRuntimeConfig, RuntimeConfig};

#[test]
fn native_runtime_reports_capabilities_and_shell_command() {
    let runtime = NativeRuntime::new();
    assert_eq!(runtime.name(), "native");
    assert!(runtime.has_shell_access());
    assert!(runtime.has_filesystem_access());
    assert!(runtime.supports_long_running());
    assert_eq!(runtime.memory_budget(), 0);
    assert!(runtime.storage_path().ends_with("openhuman/runtime"));

    // Use a tempdir so `ensure_usable_cwd` accepts the path on every
    // OS (`/tmp` does not exist on Windows).
    let tempdir = tempfile::tempdir().unwrap();
    let command = runtime
        .build_shell_command("echo hi", tempdir.path())
        .unwrap();
    let prog = command
        .as_std()
        .get_program()
        .to_string_lossy()
        .into_owned();
    let args: Vec<String> = command
        .as_std()
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    // Shell selection is delegated to `platform_shell::build_tokio_command`
    // — this test just asserts NativeRuntime is wired into it. The full
    // per-platform matrix is covered by `platform_shell::tests`.
    if cfg!(windows) {
        assert_eq!(prog, "cmd");
        assert_eq!(args, vec!["/C".to_string(), "echo hi".to_string()]);
    } else {
        assert!(prog.ends_with("bash") || prog == "sh");
        assert_eq!(args.first().map(String::as_str), Some("-lc"));
    }
    assert_eq!(command.as_std().get_current_dir(), Some(tempdir.path()));
}

#[test]
fn docker_runtime_builds_expected_flags() {
    let runtime = DockerRuntime::new(DockerRuntimeConfig {
        image: "alpine:3.20".into(),
        network: "host".into(),
        mount_workspace: true,
        read_only_rootfs: true,
        memory_limit_mb: Some(512),
        cpu_limit: Some(1.5),
        ..DockerRuntimeConfig::default()
    });
    assert_eq!(runtime.name(), "docker");
    assert!(runtime.has_shell_access());
    assert!(runtime.has_filesystem_access());
    assert!(!runtime.supports_long_running());
    assert_eq!(runtime.memory_budget(), 512);
    assert!(runtime.storage_path().ends_with("openhuman/runtime/docker"));

    let tempdir = tempfile::tempdir().unwrap();
    let command = runtime.build_shell_command("pwd", tempdir.path()).unwrap();
    let args: Vec<String> = command
        .as_std()
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    let joined = args.join(" ");
    assert!(joined.contains("run --rm"));
    assert!(joined.contains("--network host"));
    assert!(joined.contains("-m 512m"));
    assert!(joined.contains("--cpus 1.5"));
    assert!(joined.contains("--read-only"));
    assert!(joined.contains(":/workspace"));
    assert!(joined.contains("-w /workspace"));
    assert!(joined.contains("alpine:3.20"));
    assert!(joined.ends_with("sh -lc pwd"));
}

#[test]
fn create_runtime_supports_native_and_docker_and_rejects_unknown() {
    let native = create_runtime(&RuntimeConfig::default(), false).unwrap();
    assert_eq!(native.name(), "native");

    let docker = create_runtime(
        &RuntimeConfig {
            kind: "docker".into(),
            docker: DockerRuntimeConfig::default(),
            ..RuntimeConfig::default()
        },
        false,
    )
    .unwrap();
    assert_eq!(docker.name(), "docker");

    let err = create_runtime(
        &RuntimeConfig {
            kind: "vm".into(),
            ..RuntimeConfig::default()
        },
        false,
    )
    .err()
    .unwrap();
    assert!(err.to_string().contains("Unsupported runtime kind: vm"));
}

/// `[shell] hide_window` plumbs through `create_runtime` into the native
/// adapter, and a hide-window native runtime still builds a usable shell
/// command on every platform (the `CREATE_NO_WINDOW` flag is Windows-only
/// and applied without disturbing the command on macOS/Linux).
#[test]
fn native_runtime_with_hide_window_still_builds_shell_command() {
    let native = create_runtime(&RuntimeConfig::default(), true).unwrap();
    assert_eq!(native.name(), "native");

    // Tempdir so `ensure_usable_cwd` accepts it on Windows CI too.
    let tempdir = tempfile::tempdir().unwrap();
    let runtime = NativeRuntime::with_hide_window(true);
    let command = runtime
        .build_shell_command("echo hi", tempdir.path())
        .expect("hide_window should not break command construction");
    assert_eq!(command.as_std().get_current_dir(), Some(tempdir.path()));

    // The program/args are identical with and without the flag — hiding the
    // window must not alter what is executed.
    let plain = NativeRuntime::with_hide_window(false)
        .build_shell_command("echo hi", tempdir.path())
        .unwrap();
    assert_eq!(command.as_std().get_program(), plain.as_std().get_program());
}

/// `maybe_hide_window` is a no-op when disabled (and on non-Windows hosts
/// even when enabled), and must never panic.
#[test]
fn maybe_hide_window_is_safe_no_op() {
    let mut disabled = tokio::process::Command::new("echo");
    maybe_hide_window(&mut disabled, false);
    let mut enabled = tokio::process::Command::new("echo");
    maybe_hide_window(&mut enabled, true);
    assert_eq!(disabled.as_std().get_program(), "echo");
    assert_eq!(enabled.as_std().get_program(), "echo");
}

/// Regression: a failed stage in a pipeline must surface as a non-zero exit
/// (pipefail), so the harness records the call as failed and the
/// repeated-failure circuit breaker can trip — rather than `… | tail`
/// masking the failure as success and letting the agent loop. Only
/// meaningful where bash is present (the pipefail wrapper); on bash-less
/// hosts we fall back to plain `sh` and skip.
#[cfg(unix)]
#[tokio::test]
async fn native_shell_pipefail_surfaces_failed_pipe_stage() {
    if platform_shell::bash_path().is_none() {
        return; // no bash → plain sh, pipefail unavailable
    }
    let rt = NativeRuntime::new();
    let dir = std::env::temp_dir();

    let mut failing = rt.build_shell_command("false | true", &dir).unwrap();
    let status = failing.status().await.unwrap();
    assert!(
        !status.success(),
        "pipefail must surface the failed `false` stage, not mask it behind `true`"
    );

    // A clean pipeline still succeeds.
    let mut ok = rt.build_shell_command("true | true", &dir).unwrap();
    assert!(ok.status().await.unwrap().success());
}

/// #3353: a CWD that can't be made usable (here: a path *under an existing
/// file*, which `create_dir_all` cannot create) must yield a descriptive,
/// path-naming error from `build_shell_command` instead of an opaque OS
/// error 267 (ERROR_DIRECTORY) at spawn time.
#[test]
fn native_shell_command_rejects_uncreatable_cwd_with_clear_error() {
    let rt = NativeRuntime::new();
    let tmp = tempfile::tempdir().unwrap();
    let parent_file = tmp.path().join("a-file");
    std::fs::write(&parent_file, b"x").unwrap();
    let bad_cwd = parent_file.join("child"); // parent is a file → uncreatable

    let err = rt
        .build_shell_command("echo hi", &bad_cwd)
        .expect_err("an uncreatable CWD must be rejected up front");
    let msg = err.to_string();
    assert!(
        msg.contains("could not be created"),
        "expected an actionable message, got: {msg}"
    );
    assert!(
        msg.contains(&bad_cwd.to_string_lossy().to_string()),
        "error should name the offending path: {msg}"
    );
}

/// A valid-but-missing CWD is defensively created (covers a dir deleted
/// after launch), so the command builds successfully and runs there.
#[test]
fn native_shell_command_creates_missing_cwd() {
    let rt = NativeRuntime::new();
    let tmp = tempfile::tempdir().unwrap();
    let missing = tmp.path().join("nested").join("workdir");
    assert!(!missing.exists());

    let command = rt
        .build_shell_command("echo hi", &missing)
        .expect("missing CWD should be auto-created");
    assert!(missing.is_dir(), "CWD should have been created");
    assert_eq!(command.as_std().get_current_dir(), Some(missing.as_path()));
}
