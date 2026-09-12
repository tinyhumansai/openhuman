use super::*;

fn class_of(text: &str) -> ToolFailureClass {
    classify(text, false).class
}

#[test]
fn timeout_flag_wins_regardless_of_text() {
    assert_eq!(
        classify("anything at all", true).class,
        ToolFailureClass::Timeout
    );
}

#[test]
fn timeout_detected_from_text() {
    assert_eq!(
        class_of("tool 'shell' timed out after 120 seconds"),
        ToolFailureClass::Timeout
    );
}

#[test]
fn missing_permission_from_os_error() {
    assert_eq!(
        class_of("Error executing file_write: Permission denied (os error 13)"),
        ToolFailureClass::MissingPermission
    );
    assert_eq!(
        class_of("EACCES: operation not permitted"),
        ToolFailureClass::MissingPermission
    );
}

#[test]
fn missing_app_from_command_not_found() {
    assert_eq!(
        class_of("bash: gh: command not found"),
        ToolFailureClass::MissingApp
    );
    assert_eq!(
        class_of("ffmpeg is not installed on this system"),
        ToolFailureClass::MissingApp
    );
}

#[test]
fn service_unavailable_from_connection_errors() {
    assert_eq!(
        class_of("connection refused (ECONNREFUSED)"),
        ToolFailureClass::ServiceUnavailable
    );
    assert_eq!(
        class_of("upstream returned 503 Service Unavailable"),
        ToolFailureClass::ServiceUnavailable
    );
}

#[test]
fn bad_credentials_from_auth_errors() {
    assert_eq!(
        class_of("HTTP 401 Unauthorized"),
        ToolFailureClass::BadCredentials
    );
    assert_eq!(
        class_of("invalid api key provided"),
        ToolFailureClass::BadCredentials
    );
    assert_eq!(
        class_of("auth token expired, please sign in again"),
        ToolFailureClass::BadCredentials
    );
}

#[test]
fn blocked_by_policy_from_gate_and_forbidden() {
    assert_eq!(
        class_of("Permission denied for tool 'shell': requires Execute, channel allows ReadOnly"),
        ToolFailureClass::BlockedByPolicy
    );
    assert_eq!(
        class_of("blocked by policy: destructive command"),
        ToolFailureClass::BlockedByPolicy
    );
    // OpenHuman's own path guard stays policy...
    assert_eq!(
        class_of("write rejected: forbidden path outside action_dir"),
        ToolFailureClass::BlockedByPolicy
    );
}

#[test]
fn external_403_is_credentials_not_policy() {
    // A bare external authz failure must route to credentials (reconnect /
    // grant scopes), NOT OpenHuman's Agent-access policy.
    assert_eq!(
        class_of("HTTP 403 Forbidden"),
        ToolFailureClass::BadCredentials
    );
    assert_eq!(
        class_of("Gmail API error: 403 insufficient authentication scopes"),
        ToolFailureClass::BadCredentials
    );
    assert_eq!(
        class_of("401 Unauthorized"),
        ToolFailureClass::BadCredentials
    );
}

#[test]
fn policy_denied_marker_is_denied_and_not_recoverable() {
    use crate::openhuman::security::POLICY_DENIED_MARKER;
    let text = format!("{POLICY_DENIED_MARKER} you declined this shell action");
    assert_eq!(class_of(&text), ToolFailureClass::Denied);
    // UserDeclined family — never eligible for an auto-retry (#4459).
    assert!(!classify(&text, false).recoverable);
}

#[test]
fn policy_denied_ttl_expiry_is_approval_expired_not_timeout() {
    use crate::openhuman::security::POLICY_DENIED_MARKER;
    // A TTL-expiry deny reason literally contains "timed out" — the policy
    // marker must win over the timeout sniff so it classifies as an expired
    // approval, NOT an execution Timeout that promises an auto-retry (#4459).
    let text = format!("{POLICY_DENIED_MARKER} Approval for 'shell' timed out after 600s");
    assert_eq!(class_of(&text), ToolFailureClass::ApprovalExpired);
    assert_ne!(class_of(&text), ToolFailureClass::Timeout);
    assert!(!classify(&text, false).recoverable);
}

#[test]
fn numeric_status_codes_need_word_boundaries() {
    // `403`/`503` embedded in a longer digit run must NOT trip the code
    // needles — these fall through to Unknown.
    assert_eq!(
        class_of("processed 14033 records before aborting"),
        ToolFailureClass::Unknown
    );
    assert_eq!(
        class_of("listening on port 15032 failed unexpectedly"),
        ToolFailureClass::Unknown
    );
    // ...but a standalone 503 is still a service outage.
    assert_eq!(
        class_of("upstream returned 503"),
        ToolFailureClass::ServiceUnavailable
    );
}

#[test]
fn model_connection_from_provider_errors() {
    assert_eq!(
        class_of("Provider error (retryable=true): boom"),
        ToolFailureClass::ModelConnection
    );
    assert_eq!(
        class_of("could not reach the model endpoint"),
        ToolFailureClass::ModelConnection
    );
    assert_eq!(
        class_of("ollama daemon not responding"),
        ToolFailureClass::ModelConnection
    );
}

#[test]
fn unknown_when_nothing_matches() {
    assert_eq!(
        class_of("some totally novel failure mode"),
        ToolFailureClass::Unknown
    );
}

#[test]
fn credentials_precedence_over_service_when_both_present() {
    // A 401 that also mentions a connection should read as credentials, not
    // a transient service blip — the ordering guarantees this.
    assert_eq!(
        class_of("could not connect: 401 unauthorized"),
        ToolFailureClass::BadCredentials
    );
}

#[test]
fn every_class_produces_nonempty_user_copy() {
    for class in [
        ToolFailureClass::MissingPermission,
        ToolFailureClass::MissingApp,
        ToolFailureClass::ServiceUnavailable,
        ToolFailureClass::BadCredentials,
        ToolFailureClass::BlockedByPolicy,
        ToolFailureClass::ModelConnection,
        ToolFailureClass::Timeout,
        ToolFailureClass::Denied,
        ToolFailureClass::ApprovalExpired,
        ToolFailureClass::Unknown,
    ] {
        let f = describe(class);
        assert!(!f.cause_plain.is_empty(), "empty cause for {class:?}");
        assert!(!f.next_action.is_empty(), "empty next_action for {class:?}");
        assert_eq!(f.recoverable, f.category.is_recoverable());
    }
}

#[test]
fn recoverable_flag_matches_category() {
    assert!(classify("503 service unavailable", false).recoverable);
    assert!(!classify("permission denied (os error 13)", false).recoverable);
    assert!(!classify("blocked by policy", false).recoverable);
}
