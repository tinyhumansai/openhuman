use super::*;
use crate::rpc::StructuredRpcError;

#[test]
fn not_found_display_is_stable() {
    let err = ThreadsError::not_found("thread-123");
    assert_eq!(err.to_string(), "thread thread-123 not found");
}

#[test]
fn parses_canonical_and_legacy_thread_not_found_messages() {
    assert_eq!(
        parse_thread_not_found_message("thread thread-123 not found"),
        Some("thread-123")
    );
    assert_eq!(
        parse_thread_not_found_message("thread thread-123 does not exist"),
        Some("thread-123")
    );
    assert_eq!(
        parse_thread_not_found_message("message msg-1 not found in thread thread-123"),
        None
    );
}

#[test]
fn not_found_serializes_to_structured_rpc_error_at_boundary() {
    let raw: String = ThreadsError::not_found("thread-123").into();
    let structured =
        StructuredRpcError::decode(&raw).expect("NotFound must emit a structured envelope");
    assert_eq!(structured.message, "thread thread-123 not found");
    assert!(structured.expected_user_state);
    let data = structured.data.expect("structured error must carry data");
    assert_eq!(data["kind"], THREAD_NOT_FOUND_KIND);
    assert_eq!(data["thread_id"], "thread-123");
}

#[test]
fn message_variant_stays_plain_at_boundary() {
    let raw: String = ThreadsError::Message("kaboom".into()).into();
    assert_eq!(raw, "kaboom");
    assert!(
        StructuredRpcError::decode(&raw).is_none(),
        "plain messages must not carry the structured sentinel"
    );
}

#[test]
fn from_thread_scoped_store_error_id_guard() {
    // Matching id → NotFound
    let matching = ThreadsError::from_thread_scoped_store_error(
        "thread-123",
        "thread thread-123 not found".to_string(),
    );
    assert_eq!(matching, ThreadsError::not_found("thread-123"));

    // Mismatched id → Message (avoid clearing the wrong thread on the frontend)
    let mismatch = ThreadsError::from_thread_scoped_store_error(
        "thread-456",
        "thread thread-123 not found".to_string(),
    );
    assert!(
        matches!(mismatch, ThreadsError::Message(_)),
        "mismatched id must produce Message, not NotFound"
    );

    // Unrecognised format → Message
    let unrecognised =
        ThreadsError::from_thread_scoped_store_error("thread-123", "some other error".to_string());
    assert!(matches!(unrecognised, ThreadsError::Message(_)));
}
