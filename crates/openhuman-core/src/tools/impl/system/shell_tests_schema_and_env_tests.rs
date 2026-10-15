use super::*;

#[tokio::test]
async fn configured_shell_fails_closed_when_the_security_module_cannot_answer() {
    let gateway = StubGateway::refuse(Refusal::Unavailable);
    let tool = ShellTool::with_config(
        gated_security(AutonomyLevel::Full, gateway.clone()),
        test_runtime(),
        test_audit(),
        Arc::new(crate::config::Config::default()),
    );

    let result = tool
        .execute(json!({"command": "cat ~/.ssh/id_rsa"}))
        .await
        .unwrap();
    assert!(
        result.is_error,
        "an unanswered question must not run the command"
    );
    assert!(result.output().contains("authorization unavailable"));
    assert_eq!(gateway.asked().len(), 1);
}

#[tokio::test]
async fn shell_run_asks_the_module_at_the_execute_stage_with_the_exact_command() {
    let gateway = StubGateway::allow();
    let tool = ShellTool::new(
        gated_security(AutonomyLevel::Supervised, gateway.clone()),
        test_runtime(),
        test_audit(),
    );
    let _ = tool.execute(json!({"command": "true"})).await.unwrap();
    let asked = gateway.asked();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].command, "true");
    assert_eq!(asked[0].flow, CommandFlow::Harness);
    assert_eq!(asked[0].stage, CommandStage::Execute);
}

#[tokio::test]
async fn a_refused_shell_command_never_reaches_the_runtime_and_carries_the_reason() {
    for (reason, needle) in [
        (Refusal::ReadOnly, "read-only mode"),
        (Refusal::Hidden, "hidden command"),
        (Refusal::Protected, "protected path"),
        (Refusal::RateLimit, "Rate limit exceeded"),
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let marker = tmp.path().join("ran");
        let tool = ShellTool::new(
            gated_security(AutonomyLevel::Supervised, StubGateway::refuse(reason)),
            test_runtime(),
            test_audit(),
        );
        let result = tool
            .execute(json!({"command": format!("touch {}", marker.display())}))
            .await
            .unwrap();
        assert!(result.is_error, "{reason:?}");
        assert!(
            result.output().contains(needle),
            "{reason:?}: {}",
            result.output()
        );
        assert!(!marker.exists(), "{reason:?} must not run the command");
    }
}

#[cfg(feature = "modules")]
#[test]
fn docker_module_results_preserve_success_failure_and_timeout_semantics() {
    use crate::modules::tinybox::contract::DockerRunOutput;

    let success = docker_module_result(
        DockerRunOutput {
            exit_code: 0,
            stdout: "hello".to_owned(),
            stderr: String::new(),
            timed_out: false,
        },
        None,
    );
    assert!(!success.is_error);
    assert_eq!(success.output(), "hello");

    let failure = docker_module_result(
        DockerRunOutput {
            exit_code: 7,
            stdout: "partial".to_owned(),
            stderr: "failed".to_owned(),
            timed_out: false,
        },
        None,
    );
    assert!(failure.is_error);
    assert!(failure.output().contains("7"));
    assert!(failure.output().contains("partial"));
    assert!(failure.output().contains("failed"));

    let timeout = docker_module_result(
        DockerRunOutput {
            exit_code: -1,
            stdout: "before timeout".to_owned(),
            stderr: String::new(),
            timed_out: true,
        },
        Some(std::time::Duration::from_secs(3)),
    );
    assert!(timeout.is_error);
    assert!(timeout.output().contains("timed out after 3s"));
    assert!(timeout.output().contains("before timeout"));
}

#[cfg(not(windows))]
#[tokio::test]
async fn shell_emits_audit_line_on_success() {
    use crate::security::AuditEvent;
    let (audit, tmp) = audit_with_tempdir();
    let tool = ShellTool::new(
        test_security(AutonomyLevel::Supervised),
        test_runtime(),
        audit,
    );
    let _ = tool
        .execute(json!({"command": "echo hello"}))
        .await
        .unwrap();
    let log =
        std::fs::read_to_string(tmp.path().join("audit.log")).expect("audit log file should exist");
    assert!(!log.is_empty(), "audit log should not be empty");
    let parsed: AuditEvent = serde_json::from_str(log.trim()).expect("audit event JSON parses");
    let action = parsed.action.expect("action present");
    assert_eq!(action.command, Some("echo hello".to_string()));
    assert!(action.allowed, "allowed command should set allowed=true");
    let result = parsed.result.expect("result present");
    assert!(result.success, "echo hello should succeed");
    let actor = parsed.actor.expect("actor present");
    assert_eq!(actor.channel, "tool:shell");
}

#[tokio::test]
async fn shell_emits_audit_line_on_denial() {
    use crate::security::AuditEvent;
    let (audit, tmp) = audit_with_tempdir();
    let tool = ShellTool::new(
        gated_security(
            AutonomyLevel::ReadOnly,
            StubGateway::refuse(Refusal::ReadOnly),
        ),
        test_runtime(),
        audit,
    );
    // A write command in read-only mode is denied before execution.
    let _ = tool
        .execute(json!({"command": "touch denied_file"}))
        .await
        .unwrap();
    let log =
        std::fs::read_to_string(tmp.path().join("audit.log")).expect("audit log file should exist");
    let parsed: AuditEvent = serde_json::from_str(log.trim()).expect("audit event JSON parses");
    let action = parsed.action.expect("action present");
    assert!(
        !action.allowed,
        "denied command should set allowed=false on the audit event"
    );
}

#[test]
fn shell_tool_name() {
    let tool = ShellTool::new(
        test_security(AutonomyLevel::Supervised),
        test_runtime(),
        test_audit(),
    );
    assert_eq!(tool.name(), "shell");
}

#[test]
fn shell_tool_description() {
    let tool = ShellTool::new(
        test_security(AutonomyLevel::Supervised),
        test_runtime(),
        test_audit(),
    );
    assert!(!tool.description().is_empty());
}

#[test]
fn shell_tool_schema_has_command() {
    let tool = ShellTool::new(
        test_security(AutonomyLevel::Supervised),
        test_runtime(),
        test_audit(),
    );
    let schema = tool.parameters_schema();
    assert!(schema["properties"]["command"].is_object());
    assert!(schema["required"]
        .as_array()
        .unwrap()
        .contains(&json!("command")));
    // The self-asserted `approved` param was removed — approval now happens
    // at the harness ApprovalGate, not via a model-set flag.
    assert!(schema["properties"]["approved"].is_null());
}

#[cfg(not(windows))]
#[tokio::test]
async fn shell_executes_allowed_command() {
    let tool = ShellTool::new(
        test_security(AutonomyLevel::Supervised),
        test_runtime(),
        test_audit(),
    );
    let result = tool
        .execute(json!({"command": "echo hello"}))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.output());
    assert!(result.output().trim().contains("hello"));
    assert!(!result.is_error);
}

#[tokio::test]
async fn shell_destructive_command_is_gated_not_run_inline() {
    // `rm -rf /` is Destructive → it must route through the human approval
    // gate (external_effect), never auto-run. The class itself is the security
    // module's answer (pinned by the module-backed command tests); the tool's
    // part is declaring every command an external effect.
    let security = test_security(AutonomyLevel::Supervised);
    let tool = ShellTool::new(security.clone(), test_runtime(), test_audit());
    assert!(tool.external_effect_with_args(&json!({"command": "rm -rf /"})));
}

/// End-to-end regression guard for #3238.
///
/// PR #3074 split `Config.action_dir` (the agent's read/write root)
/// from `Config.workspace_dir` (internal product state). `ShellTool`
/// is contractually obligated to spawn its child process with
/// `current_dir = security.action_dir` so `pwd` inside the shell
/// reports the action sandbox path, never `workspace_dir` and never
/// the cargo-test caller's CWD.
///
/// This test constructs a `SecurityPolicy` whose `action_dir` is a
/// fresh tempdir (distinct from `workspace_dir` and from `cwd`),
/// runs `pwd`, and asserts the captured stdout canonicalises to the
/// same path as `action_dir`. If `ShellTool::run_with_security`
/// stops passing `&security.action_dir` as the module request's working
/// directory, this
/// test fails before the regression ships.
#[cfg(not(windows))]
#[tokio::test]
async fn shell_pwd_returns_action_dir_not_workspace_dir() {
    let action_tmp = tempfile::tempdir().expect("create action tempdir");
    let workspace_tmp = tempfile::tempdir().expect("create workspace tempdir");
    let security = Arc::new(SecurityPolicy {
        autonomy: AutonomyLevel::Supervised,
        workspace_dir: workspace_tmp.path().to_path_buf(),
        action_dir: action_tmp.path().to_path_buf(),
        ..SecurityPolicy::default()
    });
    let tool = ShellTool::new(security.clone(), test_runtime(), test_audit());

    let result = tool
        .execute(json!({"command": "pwd"}))
        .await
        .expect("pwd should execute without harness error");
    assert!(
        !result.is_error,
        "pwd unexpectedly errored: {}",
        result.output()
    );

    // Canonicalise both sides — on macOS `/tmp` is a symlink to
    // `/private/tmp`, so the raw strings won't match even when the
    // paths are the same.
    let reported = std::path::PathBuf::from(result.output().trim());
    let actual = reported.canonicalize().unwrap_or_else(|_| reported.clone());
    let expected = security
        .action_dir
        .canonicalize()
        .unwrap_or_else(|_| security.action_dir.clone());
    let workspace_canon = security
        .workspace_dir
        .canonicalize()
        .unwrap_or_else(|_| security.workspace_dir.clone());

    assert_eq!(
        actual,
        expected,
        "pwd must report `action_dir`. got `{}`, expected `{}`. \
         If this fails, `ShellTool::run_with_security` likely stopped \
         passing `&security.action_dir` as the native shell request's cwd. See #3238.",
        actual.display(),
        expected.display(),
    );
    assert_ne!(
        actual, workspace_canon,
        "pwd reported `workspace_dir` instead of `action_dir` — the \
         action/internal split is broken. See #3074, #3238."
    );
}

#[test]
fn shell_is_unbounded_by_default() {
    let tool = ShellTool::new(
        test_security(AutonomyLevel::Supervised),
        test_runtime(),
        test_audit(),
    );

    // No `timeout_secs` ⇒ no deadline. A long script must not be hard-killed.
    assert_eq!(
        tool.timeout_policy(&json!({"command": "make"})),
        ToolTimeout::Unbounded
    );
    assert_eq!(tool.explicit_timeout(None), None);
    // An explicit 0 disables the timeout too.
    assert_eq!(
        tool.timeout_policy(&json!({"command": "make", "timeout_secs": 0})),
        ToolTimeout::Unbounded
    );
    assert_eq!(tool.explicit_timeout(Some(0)), None);
}

#[test]
fn shell_timeout_honors_explicit_per_call_value() {
    let tool = ShellTool::new(
        test_security(AutonomyLevel::Supervised),
        test_runtime(),
        test_audit(),
    );

    // An explicit in-range request is enforced verbatim.
    assert_eq!(
        tool.timeout_policy(&json!({"command": "make", "timeout_secs": 1800})),
        ToolTimeout::Millis(1_800_000)
    );
    assert_eq!(
        tool.explicit_timeout(Some(1800)),
        Some(Duration::from_secs(1800))
    );

    // Above the cap clamps down to MAX_TIMEOUT_SECS (3600).
    assert_eq!(
        tool.explicit_timeout(Some(9_999)),
        Some(Duration::from_secs(
            tinytools_std::timeout::MAX_TIMEOUT_SECS
        ))
    );
}

#[cfg(not(windows))]
#[tokio::test]
async fn shell_per_call_timeout_kills_slow_command() {
    let tool = ShellTool::new(
        test_security(AutonomyLevel::Supervised),
        test_runtime(),
        test_audit(),
    );

    // `sleep 30` would survive any sane default, but a per-call 1s budget
    // must kill it and report the per-call value in the error message.
    let result = tool
        .execute(json!({"command": "sleep 30", "timeout_secs": 1}))
        .await
        .unwrap();

    assert!(result.is_error, "slow command should time out");
    let text = result.text();
    assert!(
        text.contains("timed out after 1s"),
        "timeout message should reflect the per-call budget, got: {text}"
    );
}

#[test]
fn shell_schema_advertises_timeout_secs() {
    let tool = ShellTool::new(
        test_security(AutonomyLevel::Supervised),
        test_runtime(),
        test_audit(),
    );
    let schema = tool.parameters_schema();
    let timeout = &schema["properties"]["timeout_secs"];
    assert_eq!(timeout["type"], "integer");
    // 0 is the documented "no deadline" value, so a schema-validating
    // provider must accept it.
    assert_eq!(timeout["minimum"], 0);
    assert_eq!(timeout["maximum"], 3600);
}

#[test]
fn shell_output_limit_is_1mb() {
    assert_eq!(
        MAX_OUTPUT_BYTES, 1_048_576,
        "max output must be 1 MB to prevent OOM"
    );
}

// ── §5.3 Non-UTF8 binary output tests ────────────────────

#[test]
fn shell_safe_env_vars_excludes_secrets() {
    for var in crate::sandbox::ops::SANDBOX_ENV_PASSTHROUGH {
        let lower = var.to_lowercase();
        assert!(
            !lower.contains("key") && !lower.contains("secret") && !lower.contains("token"),
            "the shell environment allowlist must not include sensitive variable: {var}"
        );
    }
}

#[test]
fn shell_safe_env_vars_includes_essentials() {
    let allowlist = crate::sandbox::ops::SANDBOX_ENV_PASSTHROUGH;
    for essential in ["PATH", "HOME", "TERM"] {
        assert!(
            allowlist.contains(&essential),
            "{essential} must be in the shell environment allowlist"
        );
    }
}
