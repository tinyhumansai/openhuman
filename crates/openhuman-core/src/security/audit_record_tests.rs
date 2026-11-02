use super::*;

#[test]
fn command_audit_rows_are_bounded_and_keep_execution_facts() {
    let event =
        HostAuditEvent::command_execution("tool:shell", "echo hello", true, true, false, 42);
    assert_eq!(event.kind, HostAuditKind::Execution);
    assert_eq!(event.attribution, "tool:shell");
    assert_eq!(
        event.arguments,
        json!({
            "command": "echo hello",
            "command_truncated": false,
            "approved": true,
            "allowed": true,
            "success": false,
            "duration_ms": 42,
        })
    );
}

#[test]
fn long_command_audit_rows_truncate_on_character_boundaries() {
    let command = "é".repeat(MAX_COMMAND_CHARS + 10);
    let event = HostAuditEvent::command_execution("tool:shell", &command, false, true, true, 1);
    assert_eq!(
        event.arguments["command"]
            .as_str()
            .map(|value| value.chars().count()),
        Some(MAX_COMMAND_CHARS)
    );
    assert_eq!(event.arguments["command_truncated"], true);
}

#[test]
fn diagnostic_format_does_not_print_command_content() {
    let secret = "private-token-value";
    let event = HostAuditEvent::command_execution(
        "tool:shell",
        &format!("curl -H 'Authorization: Bearer {secret}'"),
        false,
        true,
        true,
        1,
    );
    assert!(!format!("{event:?}").contains(secret));
}
