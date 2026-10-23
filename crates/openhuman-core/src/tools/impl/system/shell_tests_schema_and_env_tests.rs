use super::*;

#[tokio::test]
async fn configured_shell_fails_closed_when_the_security_module_cannot_answer() {
    let gateway = StubGateway::refuse(Refusal::Unavailable);
    let tool = ShellTool::with_config(
        gated_security(AutonomyLevel::Full, gateway.clone()),
        test_runtime(),
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

#[test]
fn shell_tool_name() {
    let tool = ShellTool::new(test_security(AutonomyLevel::Supervised), test_runtime());
    assert_eq!(tool.name(), "shell");
}

#[test]
fn shell_tool_description() {
    let tool = ShellTool::new(test_security(AutonomyLevel::Supervised), test_runtime());
    assert!(!tool.description().is_empty());
}

#[test]
fn shell_tool_schema_has_command() {
    let tool = ShellTool::new(test_security(AutonomyLevel::Supervised), test_runtime());
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
    let tool = ShellTool::new(test_security(AutonomyLevel::Supervised), test_runtime());
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
    let tool = ShellTool::new(security.clone(), test_runtime());
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
    let security = Arc::new(
        SecurityPolicy {
            autonomy: AutonomyLevel::Supervised,
            workspace_dir: workspace_tmp.path().to_path_buf(),
            action_dir: action_tmp.path().to_path_buf(),
            ..SecurityPolicy::default()
        }
        .with_gateway(StubGateway::allow()),
    );
    let tool = ShellTool::new(security.clone(), test_runtime());

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
    let tool = ShellTool::new(test_security(AutonomyLevel::Supervised), test_runtime());

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
    let tool = ShellTool::new(test_security(AutonomyLevel::Supervised), test_runtime());

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
    let tool = ShellTool::new(test_security(AutonomyLevel::Supervised), test_runtime());

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
    let tool = ShellTool::new(test_security(AutonomyLevel::Supervised), test_runtime());
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

#[tokio::test]
async fn shell_block_verdict_never_reaches_the_runtime() {
    use crate::security::GateDecision;
    use tinysecurity_bus::CommandClass;

    let tmp = tempfile::tempdir().unwrap();
    let marker = tmp.path().join("ran");
    let tool = ShellTool::new(
        gated_security(
            AutonomyLevel::ReadOnly,
            StubGateway::gate(CommandClass::Write, GateDecision::Block),
        ),
        test_runtime(),
    );
    let result = tool
        .execute(json!({"command": format!("touch {}", marker.display())}))
        .await
        .unwrap();
    assert!(result.is_error, "a Block verdict must reject execution");
    assert!(result.output().contains("blocked by security policy"));
    assert!(!marker.exists(), "a Block verdict must not run the command");
}

#[tokio::test]
async fn shell_prompt_verdict_never_runs_without_a_bound_approval() {
    let tmp = tempfile::tempdir().unwrap();
    let marker = tmp.path().join("ran");
    let tool = ShellTool::new(
        gated_security(AutonomyLevel::Supervised, StubGateway::prompt()),
        test_runtime(),
    );
    let result = tool
        .execute(json!({"command": format!("touch {}", marker.display())}))
        .await
        .unwrap();
    assert!(result.is_error, "Prompt has no in-tool approval proof");
    assert!(
        !marker.exists(),
        "a Prompt verdict must not run the command"
    );
}
