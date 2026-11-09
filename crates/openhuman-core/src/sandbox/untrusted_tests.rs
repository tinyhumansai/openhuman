use super::*;
#[cfg(all(feature = "modules", target_os = "linux"))]
use crate::sandbox::types::SandboxStatus;

fn config_in(root: &Path) -> Config {
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let mut config = Config {
        workspace_dir: workspace.clone(),
        action_dir: workspace,
        config_path: root.join("config.toml"),
        ..Config::default()
    };
    config.modules.enabled = false;
    config
}

fn shell<'a>(config: &'a Config, command: &'a str) -> UntrustedShell<'a> {
    UntrustedShell {
        dispatch_ctx: None,
        config,
        command,
        cwd: &config.action_dir,
        state_dir: &config.workspace_dir,
        timeout: Duration::from_secs(10),
        preserve_pipeline_status: false,
        env: &NO_ENV,
        stdin: None,
    }
}

/// With no TinyBox module the jail cannot be reached, and an untrusted command
/// must be refused rather than started on the host.
#[tokio::test]
async fn an_untrusted_command_never_runs_when_the_jail_is_unreachable() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(tmp.path());
    let marker = tmp.path().join("ran");
    let command = format!("touch '{}'", marker.display());

    let result = run(shell(&config, &command)).await;

    assert!(result.is_err(), "the jail is unreachable: {result:?}");
    assert!(
        !marker.exists(),
        "a refused spawn must never run the command"
    );
}

/// The operator's opt-out is only a setting declared to the module. Untrusted text never
/// runs on the host on the host's own say-so: without a module plan it is refused.
#[tokio::test]
async fn the_operator_opt_out_never_runs_untrusted_text_without_a_module_plan() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_in(tmp.path());
    config.sandbox.enabled = Some(false);
    let marker = tmp.path().join("ran");
    let command = format!("touch '{}'", marker.display());

    let result = run(shell(&config, &command)).await;

    assert!(result.is_err(), "no plan, no run: {result:?}");
    assert!(!marker.exists(), "the command ran on the host unplanned");
}

const SAAS_CHILD: &str = "OPENHUMAN_UNTRUSTED_SAAS_TEST_CHILD";

/// SaaS never honours an opt-out: with the config and the host switch both
/// turning the sandbox off, the command still must not start on the host.
/// Deployment mode is process-owned and irreversible, so the SaaS half runs in
/// a fresh libtest process.
#[tokio::test]
async fn saas_refuses_an_untrusted_command_even_when_the_operator_opted_out() {
    let module = module_path!().split_once("::").expect("crate module").1;
    let test =
        format!("{module}::saas_refuses_an_untrusted_command_even_when_the_operator_opted_out");
    if std::env::var(SAAS_CHILD).as_deref() == Ok(test.as_str()) {
        crate::core::runtime::mode::lock_mode(crate::core::runtime::Mode::Saas).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let mut config = config_in(tmp.path());
        config.sandbox.enabled = Some(false);
        let marker = tmp.path().join("ran");
        let command = format!("touch '{}'", marker.display());

        let result = run(shell(&config, &command)).await;

        assert!(result.is_err(), "SaaS must not run it unjailed: {result:?}");
        assert!(!marker.exists(), "the command ran on the host under SaaS");
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().expect("test binary"))
        .args(["--exact", &test, "--nocapture", "--test-threads=1"])
        .env(SAAS_CHILD, &test)
        .env(super::super::operator::SANDBOX_OFF_ENV, "off")
        .output()
        .expect("run isolated SaaS test");
    assert!(
        output.status.success() && String::from_utf8_lossy(&output.stdout).contains("1 passed"),
        "isolated test {test} failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Whether the module's plan, rather than the host's own floor, refused the spawn.
#[cfg(all(feature = "modules", target_os = "linux"))]
fn refused_by_module(error: &anyhow::Error) -> bool {
    #[cfg(feature = "security-module")]
    {
        error
            .downcast_ref::<super::super::spawn_plan::PlanError>()
            .is_some()
    }
    #[cfg(not(feature = "security-module"))]
    {
        let _ = error;
        false
    }
}

/// Drives the real gate against the released TinyBox archive. What the jail
/// reports decides what must happen, and the two must agree:
///
/// * suitable for untrusted code: the command runs, can write its workspace and
///   cannot write a sibling directory;
/// * not suitable (no process isolation, for example AppArmor restricting
///   unprivileged user namespaces): the command is refused with
///   [`IsolationFloor`] and never starts.
///
/// Run with `cargo test -p openhuman --lib untrusted_gate_follows_the_released_jail -- --ignored`
/// (downloads the pinned archive into a scratch install under `target/tmp`).
#[cfg(all(feature = "modules", target_os = "linux"))]
#[tokio::test]
#[ignore = "requires the released TinyBox archive"]
async fn untrusted_gate_follows_the_released_jail() {
    let _environment_lock = crate::config::TEST_ENV_LOCK.lock().await;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp");
    std::fs::create_dir_all(&root).unwrap();
    let tmp = tempfile::tempdir_in(&root).unwrap();
    let base = tmp.path().canonicalize().unwrap();
    let workspace = base.join("workspace");
    let outside = base.join("outside");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    let mut config = Config {
        workspace_dir: workspace.clone(),
        action_dir: workspace.clone(),
        config_path: base.join("config.toml"),
        ..Config::default()
    };
    config.modules.enabled = true;
    config.modules.allow_download = true;
    config.modules.install_dir = Some(base.join("modules").display().to_string());
    let state = base.join("state");

    let (_backend, status, _isolation) = super::super::native_jail::status(&config)
        .await
        .expect("the released module reports its jail");

    let inside = run(UntrustedShell {
        dispatch_ctx: None,
        config: &config,
        command: "echo jailed > inside.txt && cat inside.txt",
        cwd: &workspace,
        state_dir: &state,
        timeout: Duration::from_secs(30),
        preserve_pipeline_status: false,
        env: &NO_ENV,
        stdin: None,
    })
    .await;
    let escape = outside.join("escaped.txt");
    let command = format!("echo escaped > '{}'", escape.display());
    let escaped = run(UntrustedShell {
        dispatch_ctx: None,
        config: &config,
        command: &command,
        cwd: &workspace,
        state_dir: &state,
        timeout: Duration::from_secs(30),
        preserve_pipeline_status: false,
        env: &NO_ENV,
        stdin: None,
    })
    .await;

    assert!(
        !escape.exists(),
        "an untrusted command wrote outside its workspace: {escaped:?}"
    );
    match status {
        SandboxStatus::Ready => {
            let inside = inside.expect("a suitable jail runs a workspace write");
            assert!(inside.success(), "{inside:?}");
            assert!(inside.stdout.contains("jailed"), "{}", inside.stdout);
            assert!(workspace.join("inside.txt").is_file());
        }
        _ => {
            let error = inside.expect_err("a jail below the floor must refuse");
            // Either floor refuses it: the module's plan (a jail that is not
            // kernel-isolated is `Unsupported` for untrusted text) or, without the
            // module, the host's own `IsolationFloor`.
            let module_refused = refused_by_module(&error);
            assert!(
                module_refused
                    || error
                        .downcast_ref::<super::super::ops::IsolationFloor>()
                        .is_some(),
                "{error}"
            );
            assert!(
                !workspace.join("inside.txt").exists(),
                "the refused command ran"
            );
            assert!(escaped.is_err());
        }
    }
}

/// The Docker path has no stdin channel, so input rides in the command as
/// base64 and the wrapped command reads it back byte for byte.
#[test]
fn docker_stdin_rides_inside_the_command_and_round_trips() {
    let payload = b"{\"event\":\"it's \\\"quoted\\\"\"}\n";
    let wrapped = with_inline_stdin("cat", payload).unwrap();
    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg(&wrapped)
        .output()
        .expect("sh runs");
    assert!(output.status.success(), "{wrapped}");
    assert_eq!(output.stdout, payload);
}

#[test]
fn docker_stdin_over_the_argument_limit_is_refused() {
    let error = with_inline_stdin("cat", &vec![b'x'; DOCKER_STDIN_MAX + 1]).unwrap_err();
    assert!(error.to_string().contains("exceeds"), "{error}");
}
