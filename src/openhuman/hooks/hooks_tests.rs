//! Domain tests: config layering, selection, and the execution contract.
//!
//! The execution tests shell out to real scripts on purpose. The whole feature
//! is "spawn a process and read what it says", and a mocked runner would test
//! the mock's idea of exit codes rather than the operating system's.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use super::config::{self, HookDefinition, HookKind, HookLayer};
use super::engine::HookEngine;
use super::exec;
use super::types::{
    HookEvent, HookInput, HookOutput, HookPayload, HookPermission, ShellPayload, ToolPayload,
};

fn input(event: HookEvent, payload: HookPayload) -> HookInput {
    HookInput {
        hook_event_name: event.as_str().to_string(),
        conversation_id: None,
        generation_id: None,
        session_id: Some("sess-test".into()),
        model: None,
        agent_id: None,
        openhuman_version: "test".into(),
        workspace_roots: vec!["/tmp".into()],
        cwd: None,
        payload,
    }
}

fn tool_payload(tool: &str) -> HookPayload {
    HookPayload::Tool(ToolPayload {
        tool_name: tool.into(),
        tool_input: serde_json::json!({}),
        tool_use_id: "call-1".into(),
        ..ToolPayload::default()
    })
}

/// Write a `hooks.json` and an executable script into a fresh temp dir.
fn scratch(file: &str, script: Option<(&str, &str)>) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join(config::HOOKS_FILE_NAME), file).expect("write hooks.json");
    if let Some((name, body)) = script {
        let path = dir.path().join(name);
        std::fs::write(&path, body).expect("write script");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                .expect("chmod script");
        }
    }
    dir
}

fn definition(command: &str, dir: &std::path::Path) -> HookDefinition {
    HookDefinition {
        command: command.to_string(),
        kind: HookKind::Command,
        timeout: Some(20),
        matcher: None,
        loop_limit: None,
        fail_closed: false,
        model: None,
        enabled: true,
        layer: Some(HookLayer::Project),
        source_dir: Some(dir.to_path_buf()),
    }
}

#[test]
fn event_names_resolve_across_dialects() {
    assert_eq!(HookEvent::parse("preToolUse"), Some(HookEvent::PreToolUse));
    assert_eq!(HookEvent::parse("PreToolUse"), Some(HookEvent::PreToolUse));
    assert_eq!(
        HookEvent::parse("pre_tool_use"),
        Some(HookEvent::PreToolUse)
    );
    assert_eq!(
        HookEvent::parse("UserPromptSubmit"),
        Some(HookEvent::BeforeSubmitPrompt)
    );
    assert_eq!(HookEvent::parse("notAnEvent"), None);
}

#[test]
fn every_event_round_trips_through_its_wire_name() {
    for event in HookEvent::ALL {
        assert_eq!(HookEvent::parse(event.as_str()), Some(event), "{event}");
    }
}

#[test]
fn unsupported_version_is_a_warning_not_a_silent_drop() {
    let parsed = config::parse_one(
        &PathBuf::from("/tmp/hooks.json"),
        HookLayer::Project,
        r#"{"version": 2, "hooks": {"preToolUse": [{"command": "x"}]}}"#,
    );
    assert!(parsed.is_empty());
    assert_eq!(parsed.warnings.len(), 1);
    assert!(parsed.warnings[0].contains("unsupported version 2"));
}

#[test]
fn unknown_event_names_suggest_the_closest_match() {
    let parsed = config::parse_one(
        &PathBuf::from("/tmp/hooks.json"),
        HookLayer::Project,
        r#"{"version": 1, "hooks": {"preTool": [{"command": "x"}]}}"#,
    );
    assert!(parsed.is_empty());
    assert!(
        parsed.warnings[0].contains("did you mean 'preToolUse'"),
        "{:?}",
        parsed.warnings
    );
}

#[test]
fn definitions_are_tagged_with_their_layer_not_the_files_claim() {
    let parsed = config::parse_one(
        &PathBuf::from("/etc/openhuman/hooks.json"),
        HookLayer::System,
        r#"{"version": 1, "hooks": {"preToolUse": [{"command": "x", "layer": "project"}]}}"#,
    );
    let definitions = parsed.for_event(HookEvent::PreToolUse);
    assert_eq!(definitions.len(), 1);
    assert_eq!(definitions[0].layer, Some(HookLayer::System));
}

#[test]
fn strictest_verdict_wins_when_outputs_merge() {
    let mut allow = HookOutput {
        permission: Some(HookPermission::Allow),
        ..HookOutput::default()
    };
    allow.merge(HookOutput {
        permission: Some(HookPermission::Ask),
        ..HookOutput::default()
    });
    assert_eq!(allow.permission, Some(HookPermission::Ask));
    allow.merge(HookOutput::deny("no"));
    assert_eq!(allow.permission, Some(HookPermission::Deny));
    // A later allow cannot undo the denial.
    allow.merge(HookOutput {
        permission: Some(HookPermission::Allow),
        ..HookOutput::default()
    });
    assert_eq!(allow.permission, Some(HookPermission::Deny));
}

#[test]
fn merged_messages_concatenate_and_rewrites_take_the_last() {
    let mut first = HookOutput {
        agent_message: Some("one".into()),
        updated_input: Some(serde_json::json!({"a": 1})),
        ..HookOutput::default()
    };
    first.merge(HookOutput {
        agent_message: Some("two".into()),
        updated_input: Some(serde_json::json!({"a": 2})),
        ..HookOutput::default()
    });
    assert_eq!(first.agent_message.as_deref(), Some("one\ntwo"));
    assert_eq!(first.updated_input.unwrap()["a"], 2);
}

#[cfg(unix)]
#[tokio::test]
async fn a_hook_that_prints_a_decision_is_honoured() {
    let dir = scratch(
        "{}",
        Some((
            "deny.sh",
            "#!/bin/sh\necho '{\"permission\":\"deny\",\"agent_message\":\"nope\"}'\n",
        )),
    );
    let run = exec::run(
        &definition("./deny.sh", dir.path()),
        &input(HookEvent::PreToolUse, tool_payload("shell")),
        &BTreeMap::new(),
        Duration::from_secs(10),
    )
    .await;
    assert!(run.error.is_none(), "{:?}", run.error);
    assert!(run.output.is_deny());
    assert_eq!(run.output.agent_message.as_deref(), Some("nope"));
}

#[cfg(unix)]
#[tokio::test]
async fn exit_code_two_denies_with_stderr_as_the_reason() {
    let dir = scratch(
        "{}",
        Some((
            "deny.sh",
            "#!/bin/sh\necho 'blocked by policy' 1>&2\nexit 2\n",
        )),
    );
    let run = exec::run(
        &definition("./deny.sh", dir.path()),
        &input(HookEvent::PreToolUse, tool_payload("shell")),
        &BTreeMap::new(),
        Duration::from_secs(10),
    )
    .await;
    assert!(run.output.is_deny());
    assert_eq!(
        run.output.agent_message.as_deref(),
        Some("blocked by policy")
    );
}

#[cfg(unix)]
#[tokio::test]
async fn a_crashing_hook_fails_open_by_default_and_closed_on_request() {
    let dir = scratch("{}", Some(("boom.sh", "#!/bin/sh\nexit 9\n")));
    let open = exec::run(
        &definition("./boom.sh", dir.path()),
        &input(HookEvent::PreToolUse, tool_payload("shell")),
        &BTreeMap::new(),
        Duration::from_secs(10),
    )
    .await;
    assert!(!open.output.is_deny());
    assert!(open.error.is_some(), "the failure is still reported");

    let mut closed = definition("./boom.sh", dir.path());
    closed.fail_closed = true;
    let closed = exec::run(
        &closed,
        &input(HookEvent::PreToolUse, tool_payload("shell")),
        &BTreeMap::new(),
        Duration::from_secs(10),
    )
    .await;
    assert!(closed.output.is_deny());
}

#[cfg(unix)]
#[tokio::test]
async fn a_hook_that_hangs_times_out_rather_than_stalling_the_turn() {
    let dir = scratch("{}", Some(("hang.sh", "#!/bin/sh\nsleep 30\n")));
    let mut hook = definition("./hang.sh", dir.path());
    hook.timeout = Some(1);
    let run = exec::run(
        &hook,
        &input(HookEvent::PreToolUse, tool_payload("shell")),
        &BTreeMap::new(),
        Duration::from_secs(1),
    )
    .await;
    assert!(run
        .error
        .as_deref()
        .unwrap_or_default()
        .contains("timed out"));
    assert!(
        !run.output.is_deny(),
        "a timeout fails open unless asked otherwise"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn the_event_reaches_the_hook_on_stdin() {
    let dir = scratch(
        "{}",
        Some((
            "echo.sh",
            "#!/bin/sh\nbody=$(cat)\ncase \"$body\" in *beforeShellExecution*) \
             echo '{\"permission\":\"deny\",\"agent_message\":\"saw the event\"}';; \
             *) echo '{}';; esac\n",
        )),
    );
    let run = exec::run(
        &definition("./echo.sh", dir.path()),
        &input(
            HookEvent::BeforeShellExecution,
            HookPayload::Shell(ShellPayload {
                command: "rm -rf /".into(),
                sandbox: false,
                ..ShellPayload::default()
            }),
        ),
        &BTreeMap::new(),
        Duration::from_secs(10),
    )
    .await;
    assert_eq!(run.output.agent_message.as_deref(), Some("saw the event"));
}

#[cfg(unix)]
#[tokio::test]
async fn matchers_decide_which_hook_runs() {
    let engine = HookEngine::default();
    let dir = scratch(
        r#"{"version": 1, "hooks": {"preToolUse": [
             {"command": "./deny.sh", "matcher": "shell"}
           ]}}"#,
        Some(("deny.sh", "#!/bin/sh\necho '{\"permission\":\"deny\"}'\n")),
    );
    let loaded = config::load(None, Some(dir.path()));
    // The workspace layer reads `<dir>/hooks.json` directly.
    assert_eq!(loaded.len(), 1, "{:?}", loaded.warnings);
    engine.install(loaded).await;

    let denied = engine
        .dispatch(
            HookEvent::PreToolUse,
            input(HookEvent::PreToolUse, tool_payload("shell")),
        )
        .await;
    assert!(denied.is_deny());

    let allowed = engine
        .dispatch(
            HookEvent::PreToolUse,
            input(HookEvent::PreToolUse, tool_payload("memory_store")),
        )
        .await;
    assert!(!allowed.is_deny(), "a non-matching tool is untouched");
}

#[cfg(unix)]
#[tokio::test]
async fn a_denial_short_circuits_the_remaining_hooks() {
    let engine = HookEngine::default();
    let dir = scratch(
        r#"{"version": 1, "hooks": {"preToolUse": [
             {"command": "./deny.sh"},
             {"command": "./second.sh"}
           ]}}"#,
        Some((
            "deny.sh",
            "#!/bin/sh\necho '{\"permission\":\"deny\",\"agent_message\":\"first\"}'\n",
        )),
    );
    std::fs::write(
        dir.path().join("second.sh"),
        "#!/bin/sh\necho '{\"agent_message\":\"second\"}'\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            dir.path().join("second.sh"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
    }
    engine.install(config::load(None, Some(dir.path()))).await;

    let outcome = engine
        .dispatch(
            HookEvent::PreToolUse,
            input(HookEvent::PreToolUse, tool_payload("shell")),
        )
        .await;
    assert!(outcome.is_deny());
    assert_eq!(outcome.runs.len(), 1, "the second hook never ran");
    assert_eq!(outcome.denial_reason(), Some("first"));
}

#[cfg(unix)]
#[tokio::test]
async fn followups_stop_once_a_hook_exhausts_its_budget() {
    let engine = HookEngine::default();
    let dir = scratch(
        r#"{"version": 1, "hooks": {"stop": [
             {"command": "./again.sh", "loop_limit": 2}
           ]}}"#,
        Some((
            "again.sh",
            "#!/bin/sh\necho '{\"followup_message\":\"keep going\"}'\n",
        )),
    );
    engine.install(config::load(None, Some(dir.path()))).await;

    let mut granted = 0;
    for _ in 0..4 {
        let outcome = engine
            .dispatch(
                HookEvent::Stop,
                input(
                    HookEvent::Stop,
                    HookPayload::Stop(super::types::StopPayload {
                        status: "completed".into(),
                        loop_count: 0,
                        iteration_count: None,
                    }),
                ),
            )
            .await;
        if outcome.output.followup_message.is_some() {
            granted += 1;
        }
    }
    assert_eq!(granted, 2, "the loop limit caps the follow-ups");
}

#[tokio::test]
async fn an_unconfigured_engine_reports_no_hooks_for_any_event() {
    let engine = HookEngine::default();
    for event in HookEvent::ALL {
        assert!(!engine.has_hooks(event).await, "{event}");
    }
}

#[test]
fn configuring_an_unwired_event_warns_rather_than_failing_silently() {
    let parsed = config::parse_one(
        &PathBuf::from("/tmp/hooks.json"),
        HookLayer::Project,
        r#"{"version": 1, "hooks": {"sessionStart": [{"command": "x"}]}}"#,
    );
    assert_eq!(parsed.for_event(HookEvent::SessionStart).len(), 1);
    assert!(
        parsed.warnings.iter().any(|w| w.contains("never run")),
        "{:?}",
        parsed.warnings
    );
}

#[test]
fn wired_events_do_not_warn() {
    let parsed = config::parse_one(
        &PathBuf::from("/tmp/hooks.json"),
        HookLayer::Project,
        r#"{"version": 1, "hooks": {"preToolUse": [{"command": "x"}]}}"#,
    );
    assert!(parsed.warnings.is_empty(), "{:?}", parsed.warnings);
}

#[test]
fn a_payload_is_parsed_for_its_event_not_by_untagged_guessing() {
    // `{"trigger": "auto"}` also satisfies `SessionPayload` (every field
    // optional), which is declared earlier in the untagged enum — so untagged
    // dispatch would silently drop the trigger.
    let payload = HookPayload::from_value_for(
        HookEvent::PreCompact,
        serde_json::json!({"trigger": "auto", "message_count": 12}),
    )
    .expect("a compact payload parses");
    match payload {
        HookPayload::Compact(compact) => {
            assert_eq!(compact.trigger, "auto");
            assert_eq!(compact.message_count, Some(12));
        }
        other => panic!("expected a compact payload, got {other:?}"),
    }
}

#[test]
fn a_payload_that_does_not_fit_its_event_is_rejected() {
    let error = HookPayload::from_value_for(HookEvent::BeforeShellExecution, serde_json::json!({}))
        .expect_err("a shell payload needs a command");
    assert!(error.contains("command"), "{error}");
}

#[test]
fn a_relative_script_that_is_missing_is_reported_at_load_time() {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(
        dir.path().join(config::HOOKS_FILE_NAME),
        r#"{"version": 1, "hooks": {"preToolUse": [{"command": "./nope.sh"}]}}"#,
    )
    .unwrap();
    let loaded = config::load(None, Some(dir.path()));
    assert!(
        loaded.warnings.iter().any(|w| w.contains("missing script")),
        "{:?}",
        loaded.warnings
    );
}

#[test]
fn a_bare_command_name_is_not_second_guessed() {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(
        dir.path().join(config::HOOKS_FILE_NAME),
        r#"{"version": 1, "hooks": {"preToolUse": [{"command": "audit-tool --strict"}]}}"#,
    )
    .unwrap();
    let loaded = config::load(None, Some(dir.path()));
    assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
}

/// `subagentStart` has a call site in `subagent_runner`; `subagentStop` does
/// not — `ops::subagent_stopped` is never called. Reporting it as wired is the
/// one outcome `is_wired`'s own documentation calls the worst this system can
/// produce: the author believes a policy is enforced and nothing says otherwise.
#[test]
fn subagent_stop_is_reported_as_not_yet_fired() {
    assert!(
        HookEvent::SubagentStart.is_wired(),
        "subagentStart is fired from subagent_runner"
    );
    assert!(
        !HookEvent::SubagentStop.is_wired(),
        "nothing calls ops::subagent_stopped yet"
    );

    let parsed = config::parse_one(
        &PathBuf::from("/tmp/hooks.json"),
        HookLayer::Project,
        r#"{"version": 1, "hooks": {"subagentStop": [{"command": "x"}]}}"#,
    );
    assert_eq!(parsed.for_event(HookEvent::SubagentStop).len(), 1);
    assert!(
        parsed.warnings.iter().any(|w| w.contains("never run")),
        "{:?}",
        parsed.warnings
    );
}
