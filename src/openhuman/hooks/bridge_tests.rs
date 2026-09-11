use super::*;

fn context(tool: &str, arguments: Value) -> ToolHookContext {
    ToolHookContext {
        event: crate::openhuman::agent::hooks::ToolHookEvent::PreToolUse,
        call_id: "call-1".into(),
        tool_name: tool.into(),
        arguments,
        success: None,
        duration_ms: None,
        output: None,
        error: None,
        session_id: Some("sess-1".into()),
        agent_id: None,
    }
}

#[test]
fn shell_tools_derive_shell_events() {
    assert_eq!(
        derived_event("shell", true),
        Some(HookEvent::BeforeShellExecution)
    );
    assert_eq!(
        derived_event("shell", false),
        Some(HookEvent::AfterShellExecution)
    );
}

#[test]
fn reads_have_no_after_event_and_writes_have_no_before_event() {
    assert_eq!(
        derived_event("file_read", true),
        Some(HookEvent::BeforeReadFile)
    );
    assert_eq!(derived_event("file_read", false), None);
    assert_eq!(derived_event("file_write", true), None);
    assert_eq!(
        derived_event("file_write", false),
        Some(HookEvent::AfterFileEdit)
    );
}

#[test]
fn mcp_tools_derive_mcp_events() {
    assert_eq!(
        derived_event("mcp_call_tool", true),
        Some(HookEvent::BeforeMcpExecution)
    );
    assert_eq!(derived_event("memory_store", true), None);
}

#[test]
fn shell_payload_carries_the_command_line() {
    let ctx = context("shell", serde_json::json!({"command": "rm -rf /tmp/x"}));
    match derived_payload(HookEvent::BeforeShellExecution, &ctx) {
        HookPayload::Shell(shell) => {
            assert_eq!(shell.command, "rm -rf /tmp/x");
            assert!(!shell.sandbox);
        }
        other => panic!("expected a shell payload, got {other:?}"),
    }
}

#[test]
fn file_path_is_read_from_any_of_the_spellings() {
    assert_eq!(
        file_path_argument(&serde_json::json!({"file_path": "/a"})),
        "/a"
    );
    assert_eq!(file_path_argument(&serde_json::json!({"path": "/b"})), "/b");
    assert_eq!(file_path_argument(&serde_json::json!({})), "");
}

#[test]
fn whole_file_write_reports_one_edit_with_empty_old_string() {
    let edits = file_edits(&serde_json::json!({"content": "hello"}));
    assert_eq!(edits.len(), 1);
    assert!(edits[0].old_string.is_empty());
    assert_eq!(edits[0].new_string, "hello");
}

#[test]
fn denial_carries_the_agent_message() {
    let decision = decision_from(HookOutput::deny("policy says no"), "shell");
    match decision {
        ToolHookDecision::Deny(reason) => assert_eq!(reason, "policy says no"),
        other => panic!("expected a denial, got {other:?}"),
    }
}

#[test]
fn updated_input_becomes_a_rewrite() {
    let output = HookOutput {
        updated_input: Some(serde_json::json!({"command": "ls"})),
        ..HookOutput::default()
    };
    match decision_from(output, "shell") {
        ToolHookDecision::ProceedWith(arguments) => {
            assert_eq!(arguments["command"], "ls");
        }
        other => panic!("expected a rewrite, got {other:?}"),
    }
}

#[test]
fn timeout_text_classifies_as_a_timeout_failure() {
    assert_eq!(classify_failure("tool timed out after 30s"), "timeout");
    assert_eq!(
        classify_failure("[policy-blocked] nope"),
        "permission_denied"
    );
    assert_eq!(classify_failure("something else"), "error");
}
