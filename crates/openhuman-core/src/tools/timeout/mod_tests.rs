use super::*;

#[test]
fn default_when_env_missing() {
    assert_eq!(parse_tool_timeout_secs(None), DEFAULT_TIMEOUT_SECS);
}

#[test]
fn default_when_value_not_numeric() {
    assert_eq!(
        parse_tool_timeout_secs(Some("not-a-number")),
        DEFAULT_TIMEOUT_SECS
    );
    assert_eq!(parse_tool_timeout_secs(Some("")), DEFAULT_TIMEOUT_SECS);
    assert_eq!(parse_tool_timeout_secs(Some("12x")), DEFAULT_TIMEOUT_SECS);
}

#[test]
fn default_when_value_zero() {
    // 0 seconds would disable the timeout — reject and fall back.
    assert_eq!(parse_tool_timeout_secs(Some("0")), DEFAULT_TIMEOUT_SECS);
}

#[test]
fn default_when_value_above_max() {
    assert_eq!(parse_tool_timeout_secs(Some("3601")), DEFAULT_TIMEOUT_SECS);
    assert_eq!(
        parse_tool_timeout_secs(Some("99999999999")),
        DEFAULT_TIMEOUT_SECS
    );
}

#[test]
fn default_when_value_negative_or_signed() {
    // Negative values fail u64 parse and fall back to default.
    assert_eq!(parse_tool_timeout_secs(Some("-5")), DEFAULT_TIMEOUT_SECS);
}

#[test]
fn accepts_valid_values_at_boundaries() {
    assert_eq!(parse_tool_timeout_secs(Some("1")), MIN_TIMEOUT_SECS);
    assert_eq!(parse_tool_timeout_secs(Some("3600")), MAX_TIMEOUT_SECS);
}

#[test]
fn accepts_valid_midrange_value() {
    assert_eq!(parse_tool_timeout_secs(Some("300")), 300);
}

#[test]
fn env_override_takes_precedence_over_config() {
    // When the env var holds a valid value it wins over the config value.
    assert_eq!(resolve_effective(300, Some("600")), 600);
}

#[test]
fn config_value_used_when_env_absent_or_invalid() {
    // No env → config drives the effective value (bounded).
    assert_eq!(resolve_effective(300, None), 300);
    // Present-but-invalid env (non-numeric / 0 / out of range) is ignored,
    // so the config value still applies.
    assert_eq!(resolve_effective(300, Some("nonsense")), 300);
    assert_eq!(resolve_effective(300, Some("0")), 300);
    assert_eq!(resolve_effective(300, Some("4000")), 300);
}

#[test]
fn config_value_is_bounded() {
    // An out-of-range config value falls back to the default rather than
    // being applied verbatim.
    assert_eq!(resolve_effective(0, None), DEFAULT_TIMEOUT_SECS);
    assert_eq!(resolve_effective(99_999, None), DEFAULT_TIMEOUT_SECS);
}

#[test]
fn explicit_call_timeout_unbounded_when_absent_or_disabled() {
    // No request, or an explicit 0, means "run unbounded" (None).
    assert_eq!(explicit_call_timeout_secs(None, MAX_TIMEOUT_SECS), None);
    assert_eq!(explicit_call_timeout_secs(Some(0), MAX_TIMEOUT_SECS), None);
}

#[test]
fn explicit_call_timeout_enforces_and_clamps_request() {
    // An in-range request is enforced verbatim.
    assert_eq!(
        explicit_call_timeout_secs(Some(900), MAX_TIMEOUT_SECS),
        Some(900)
    );
    // Below the floor clamps up to MIN.
    assert_eq!(
        explicit_call_timeout_secs(Some(0), MAX_TIMEOUT_SECS),
        None,
        "0 disables rather than clamping to MIN"
    );
    // Above the cap clamps down to the cap.
    assert_eq!(
        explicit_call_timeout_secs(Some(99_999), MAX_TIMEOUT_SECS),
        Some(MAX_TIMEOUT_SECS)
    );
    // A tool with a tighter own ceiling (node/npm at 1800) clamps to it.
    assert_eq!(explicit_call_timeout_secs(Some(99_999), 1800), Some(1800));
    assert_eq!(explicit_call_timeout_secs(Some(600), 1800), Some(600));
}

#[test]
fn explicit_call_timeout_duration_matches_secs() {
    assert_eq!(
        explicit_call_timeout_duration(Some(900), MAX_TIMEOUT_SECS),
        Some(Duration::from_secs(900))
    );
    assert_eq!(explicit_call_timeout_duration(None, MAX_TIMEOUT_SECS), None);
}

#[test]
fn env_override_from_rejects_invalid() {
    assert_eq!(env_override_from(None), None);
    assert_eq!(env_override_from(Some("")), None);
    assert_eq!(env_override_from(Some("0")), None);
    assert_eq!(env_override_from(Some("abc")), None);
    assert_eq!(env_override_from(Some("3601")), None);
    assert_eq!(env_override_from(Some("120")), Some(120));
}

/// Table of `(policy, inherited config secs, expected (deadline_ms, budget_secs))`.
/// Pins `resolve_tool_deadline` so its behaviour is identical whether it is
/// implemented locally or delegated to the vendored `ToolTimeoutSettings`.
#[test]
fn resolve_tool_deadline_table() {
    use tinytools::ToolTimeout;
    let ms = |m: u64| Some(Duration::from_millis(m));
    let cases: Vec<(ToolTimeout, u64, (Option<Duration>, u64))> = vec![
        // Inherit: the global config value, no grace.
        (ToolTimeout::Inherit, 120, (ms(120_000), 120)),
        (ToolTimeout::Inherit, 1, (ms(1_000), 1)),
        (ToolTimeout::Inherit, 3600, (ms(3_600_000), 3600)),
        // Unbounded: no deadline, zero budget.
        (ToolTimeout::Unbounded, 120, (None, 0)),
        // Millis: rounded UP to whole seconds, clamped, +5s grace on deadline.
        (ToolTimeout::Millis(0), 120, (ms(6_000), 1)),
        (ToolTimeout::Millis(1), 120, (ms(6_000), 1)),
        (ToolTimeout::Millis(1_000), 120, (ms(6_000), 1)),
        (ToolTimeout::Millis(1_001), 120, (ms(7_000), 2)),
        (ToolTimeout::Millis(1_500), 120, (ms(7_000), 2)),
        (ToolTimeout::Millis(30_000), 120, (ms(35_000), 30)),
        (ToolTimeout::Millis(3_600_000), 120, (ms(3_605_000), 3600)),
        (ToolTimeout::Millis(3_600_001), 120, (ms(3_605_000), 3600)),
        (ToolTimeout::Millis(u64::MAX), 120, (ms(3_605_000), 3600)),
    ];
    for (policy, inherited_secs, expected) in cases {
        // Resolve against the pure helper so the process-global stays untouched.
        let got = resolve_tool_deadline_with(policy, inherited_secs);
        assert_eq!(
            got, expected,
            "policy {policy:?} inherited {inherited_secs}s"
        );
    }
}

/// Env / config precedence table for the effective inherited timeout.
#[test]
fn resolve_effective_table() {
    let cases: &[(u64, Option<&str>, u64)] = &[
        (300, None, 300),
        (300, Some("600"), 600),
        (300, Some("1"), 1),
        (300, Some("3600"), 3600),
        (300, Some("3601"), 300),
        (300, Some("0"), 300),
        (300, Some("-1"), 300),
        (300, Some(""), 300),
        (300, Some("abc"), 300),
        (0, None, DEFAULT_TIMEOUT_SECS),
        (0, Some("45"), 45),
        (3601, None, DEFAULT_TIMEOUT_SECS),
        (u64::MAX, None, DEFAULT_TIMEOUT_SECS),
        (1, None, 1),
        (3600, None, 3600),
    ];
    for &(config, env, expected) in cases {
        assert_eq!(
            resolve_effective(config, env),
            expected,
            "config {config} env {env:?}"
        );
    }
}

/// The deadline must take the command's whole process group with it. A shell
/// pipeline is grandchildren of the tool's child, and a `grep -rl … /` the tool
/// had reported as killed ran on for half an hour at a full core.
#[cfg(unix)]
#[tokio::test]
async fn a_timed_out_command_takes_its_whole_process_group_with_it() {
    let dir = std::env::temp_dir().join(format!("oh-pgkill-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let pidfile = dir.join("grandchild.pid");
    // `sleep 30 &` is a grandchild in the shell's own process group; `wait`
    // keeps the shell alive past the deadline.
    let mut cmd = crate::agent::platform_shell::build_tokio_command(&format!(
        "sleep 30 & echo $! > {}; wait",
        pidfile.display()
    ));
    let result = output_or_kill(&mut cmd, Duration::from_millis(700)).await;
    assert!(result.is_err(), "the deadline must fire on a 30s sleep");

    let grandchild: i32 = loop {
        if let Ok(text) = std::fs::read_to_string(&pidfile) {
            if let Ok(pid) = text.trim().parse() {
                break pid;
            }
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    let mut gone = false;
    for _ in 0..100 {
        // kill(pid, 0) succeeds while the process (or its zombie) exists.
        if unsafe { libc::kill(grandchild, 0) } != 0 {
            gone = true;
            break;
        }
        // A container's PID 1 may leave a killed orphan unreaped. It cannot
        // execute side effects; use the same kernel-state rule as cleanup.
        #[cfg(target_os = "linux")]
        if let Ok(stat) = std::fs::read_to_string(format!("/proc/{grandchild}/stat")) {
            let group = unsafe { libc::getpgid(grandchild) };
            if group > 0 && !super::group_exit::is_active_member(&stat, group as u32) {
                gone = true;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(gone, "grandchild {grandchild} survived the deadline");
}

/// Without a deadline the child still dies with the handle: a cancelled tool
/// future must not leave the command running either.
#[cfg(unix)]
#[tokio::test]
async fn a_shell_family_child_dies_with_a_dropped_future() {
    let dir = std::env::temp_dir().join(format!("oh-dropkill-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let pidfile = dir.join("child.pid");
    let mut cmd = crate::agent::platform_shell::build_tokio_command(&format!(
        "echo $$ > {}; sleep 30",
        pidfile.display()
    ));
    // `timeout` takes the future by value and drops it when the deadline
    // passes -- exactly the abandoned-future path a cancelled tool call takes.
    let _ = tokio::time::timeout(Duration::from_millis(300), cmd.output()).await;
    let shell: i32 = loop {
        if let Ok(text) = std::fs::read_to_string(&pidfile) {
            if let Ok(pid) = text.trim().parse() {
                break pid;
            }
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    let mut gone = false;
    for _ in 0..100 {
        if unsafe { libc::kill(shell, 0) } != 0 {
            gone = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(gone, "shell {shell} survived its dropped future");
}

// ── Harness installation (regression: the 120s default was never installed
// into the TinyAgents harness after f33a398faa, so `Inherit` tools ran until
// the run's wall-clock budget) ──────────────────────────────────────────────

mod harness_install {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    use async_trait::async_trait;
    use serde_json::{json, Value};
    use tinyagents_harness::runtime::AgentHarness;
    use tinyagents_harness::testkit::{text_response, tool_call_response};
    use tinyinference_llm::message::Message;
    use tinyinference_llm::providers::MockModel;
    use tinyinference_llm::tool::ToolCall;
    use tinytools::{Tool, ToolResult};

    /// A tool that inherits the global timeout and never finishes in time.
    struct HungTool {
        finished: Arc<AtomicBool>,
    }

    #[async_trait]
    impl Tool for HungTool {
        fn name(&self) -> &str {
            "hung_mcp_call"
        }
        fn description(&self) -> &str {
            "stands in for an MCP call that never answers"
        }
        fn parameters_schema(&self) -> Value {
            json!({"type": "object"})
        }
        async fn execute(&self, _args: Value) -> anyhow::Result<ToolResult> {
            tokio::time::sleep(Duration::from_secs(60)).await;
            self.finished.store(true, Ordering::SeqCst);
            Ok(ToolResult::success("too late"))
        }
    }

    #[test]
    fn the_host_installs_the_shared_settings() {
        let mut harness: AgentHarness<(), ()> = AgentHarness::new();
        assert!(harness.tool_timeout_settings().is_none());
        install_harness_tool_timeouts(&mut harness);
        let installed = harness
            .tool_timeout_settings()
            .expect("the host installs per-tool timeout settings");
        assert_eq!(installed, settings());
        assert_eq!(
            installed.resolve(ToolTimeout::Inherit).deadline,
            Some(Duration::from_secs(tool_execution_timeout_secs())),
            "an Inherit tool gets the configured deadline"
        );
        assert_eq!(
            installed.resolve(ToolTimeout::Unbounded).deadline,
            None,
            "long-running tools that declare Unbounded stay exempt"
        );
    }

    #[tokio::test]
    async fn a_hung_inherit_tool_is_cut_off_at_its_budget() {
        let finished = Arc::new(AtomicBool::new(false));
        let mut harness: AgentHarness<(), ()> = AgentHarness::new();
        harness.register_model(
            "mock",
            Arc::new(MockModel::with_responses(vec![
                tool_call_response(ToolCall::new("c1", "hung_mcp_call", json!({}))),
                text_response("done"),
            ])),
        );
        harness.register_tool(Arc::new(HungTool {
            finished: finished.clone(),
        }));
        // One-second inherited budget so the test is fast; production installs
        // the same type with the configured 120s.
        install_with(&mut harness, build_settings(1));

        let started = std::time::Instant::now();
        let run = harness
            .invoke_in_context(
                &(),
                tinyagents_harness::context::RunContext::new(
                    tinyagents_harness::context::RunConfig::new("timeout-e2e"),
                    (),
                ),
                vec![Message::user("go")],
            )
            .await
            .expect("a timed-out tool is a recoverable result, not a run failure");
        let elapsed = started.elapsed();

        assert!(
            elapsed < Duration::from_secs(20),
            "the call is cut at its budget, not left to run: {elapsed:?}"
        );
        assert!(
            !finished.load(Ordering::SeqCst),
            "the hung call never completed"
        );
        let transcript = format!("{:?}", run.messages);
        assert!(
            transcript.contains("hung_mcp_call") && transcript.contains("timed out after 1000 ms"),
            "the model is told the tool timed out: {transcript}"
        );
    }
}
