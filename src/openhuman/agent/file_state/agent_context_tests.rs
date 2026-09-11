use super::*;

#[tokio::test]
async fn returns_none_outside_scope() {
    assert_eq!(current_file_state_agent_id(), None);
}

#[tokio::test]
async fn installs_and_reads_agent_id() {
    let observed =
        with_file_state_agent_id("agent-1".into(), async { current_file_state_agent_id() }).await;
    assert_eq!(observed, Some("agent-1".to_string()));
}

#[tokio::test]
async fn does_not_leak_across_scopes() {
    with_file_state_agent_id("agent-1".into(), async {
        assert_eq!(current_file_state_agent_id(), Some("agent-1".to_string()));
    })
    .await;
    assert_eq!(current_file_state_agent_id(), None);
}

#[tokio::test]
async fn nested_scope_overrides_outer() {
    with_file_state_agent_id("parent".into(), async {
        assert_eq!(current_file_state_agent_id(), Some("parent".to_string()));
        with_file_state_agent_id("child".into(), async {
            assert_eq!(current_file_state_agent_id(), Some("child".to_string()));
        })
        .await;
        assert_eq!(current_file_state_agent_id(), Some("parent".to_string()));
    })
    .await;
}
