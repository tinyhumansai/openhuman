use crate::agent::{
    goose::{GooseCheckpointStore, GooseTurnAdapter},
    messages::{ChatMessage, ConversationMessage},
};

use super::*;

#[tokio::test]
async fn checkpoint_survives_a_fresh_store_handle_and_removal() {
    let workspace = tempfile::tempdir().expect("temp workspace");
    let session_id = "web:thread-cold-boot";
    let checkpoint = GooseTurnAdapter::checkpoint_from_openhuman(&[ConversationMessage::Chat(
        ChatMessage::user("continue the task"),
    )])
    .expect("checkpoint");

    primary_checkpoint_store(workspace.path())
        .insert(session_id, checkpoint)
        .expect("persist checkpoint");
    assert!(has_live_primary_checkpoint(workspace.path(), session_id).await);

    let fresh = primary_checkpoint_store(workspace.path());
    let mut updated = fresh.load(session_id).await.expect("reload checkpoint");
    updated.revision = 1;
    fresh
        .compare_and_swap(session_id, 0, updated)
        .await
        .expect("replace checkpoint");
    assert_eq!(
        primary_checkpoint_store(workspace.path())
            .load(session_id)
            .await
            .expect("load replaced checkpoint")
            .revision,
        1
    );
    assert!(fresh
        .claim_execution(session_id, "call-1")
        .await
        .expect("first claim"));
    assert!(!primary_checkpoint_store(workspace.path())
        .claim_execution(session_id, "call-1")
        .await
        .expect("duplicate claim"));
    clear_primary_checkpoint(workspace.path(), session_id);
    assert!(!has_live_primary_checkpoint(workspace.path(), session_id).await);
}
