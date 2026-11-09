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

fn audited_config() -> crate::config::Config {
    let mut config = crate::config::Config::default();
    config.audit.enabled = true;
    config
}

fn an_event() -> HostAuditEvent {
    HostAuditEvent::command_execution("tool:shell", "echo hello", true, true, true, 1)
}

#[cfg(not(feature = "security-module"))]
#[tokio::test]
async fn a_build_without_the_module_records_nothing_and_succeeds() {
    let ctx = crate::core::runtime::DispatchCtx::ambient();
    assert_eq!(
        record(&ctx, Some(&audited_config()), an_event()).await,
        Ok(())
    );
}

#[cfg(feature = "security-module")]
#[tokio::test]
async fn security_configured_off_records_nothing_and_succeeds() {
    let mut config = audited_config();
    config.security = crate::config::SecurityModeConfig::off();
    let ctx = crate::core::runtime::DispatchCtx::ambient();
    assert_eq!(record(&ctx, Some(&config), an_event()).await, Ok(()));
}

#[cfg(feature = "security-module")]
#[tokio::test]
async fn an_enforcing_audit_without_the_module_fails() {
    let mut config = audited_config();
    config.modules.enabled = false;
    let ctx = crate::core::runtime::DispatchCtx::ambient();
    assert!(record(&ctx, Some(&config), an_event()).await.is_err());
}
