use super::*;

#[test]
fn parent_key_follows_last_double_underscore() {
    assert_eq!(parent_session_key("1719_orchestrator"), None);
    assert_eq!(
        parent_session_key("1719_orchestrator__1720_researcher"),
        Some("1719_orchestrator".to_string())
    );
    assert_eq!(
        parent_session_key("a__b__c"),
        Some("a__b".to_string()),
        "two-level chains keep the full parent chain as the parent key"
    );
}

#[test]
fn sanitize_maps_unsafe_bytes_and_guards_dots() {
    assert_eq!(sanitize_store_name("1719_agent"), "1719_agent");
    assert_eq!(sanitize_store_name("a/b:c d"), "a_b_c_d");
    assert_eq!(sanitize_store_name(""), "session");
    assert_eq!(sanitize_store_name(".."), "session");
}

#[test]
fn thread_id_synthesized_only_when_absent() {
    assert_eq!(
        effective_thread_id("s1", Some("t-1")),
        ("t-1".to_string(), false)
    );
    assert_eq!(
        effective_thread_id("s1", None),
        ("imported-s1".to_string(), true)
    );
    assert_eq!(
        effective_thread_id("s1", Some("")),
        ("imported-s1".to_string(), true)
    );
}

#[test]
fn stream_name_is_store_safe() {
    assert_eq!(
        stream_name("1719_a__1720_b"),
        "session.1719_a__1720_b.messages"
    );
}
