use super::*;

#[test]
fn a_command_row_names_the_outcome_and_the_actor_not_the_output() {
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
fn a_long_command_is_cut_on_a_char_boundary_and_marked() {
    let command = "é".repeat(MAX_COMMAND_CHARS + 10);
    let event = HostAuditEvent::command_execution("tool:shell", &command, false, true, true, 1);
    assert_eq!(
        event.arguments["command"]
            .as_str()
            .map(|c| c.chars().count()),
        Some(MAX_COMMAND_CHARS)
    );
    assert_eq!(event.arguments["command_truncated"], json!(true));
}

#[test]
fn diagnostics_never_print_the_command() {
    let token = ["private", "-", "token"].concat();
    let event = HostAuditEvent::command_execution(
        "tool:shell",
        &format!("curl -H 'Authorization: Bearer {token}'"),
        false,
        true,
        true,
        1,
    );
    assert!(!format!("{event:?}").contains(&token));
}
