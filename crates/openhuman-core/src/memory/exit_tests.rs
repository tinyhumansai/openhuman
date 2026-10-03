use super::*;

use chrono::Utc;
use tinymemory::{ItemKind, MetaFilter};

use crate::memory::conversations::{self, buffer::CommittedTurn};
use crate::memory::test_fixtures::{bind_reference, config_in, stored};

fn turn(thread_id: &str) -> CommittedTurn {
    CommittedTurn {
        thread_id: thread_id.to_string(),
        agent_id: Some("orchestrator".into()),
        workspace: None,
        channel: None,
        user: "hello".into(),
        assistant: "hi".into(),
        tool_calls: Vec::new(),
        at: Utc::now(),
    }
}

#[tokio::test]
async fn stores_buffered_turns_on_exit() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut config = config_in(&tmp);
    config.memory.conversations.batch_turns = 10;
    let engine = bind_reference(&config);

    conversations::record_turn(&config, turn("t-exit")).await;
    assert_eq!(
        conversations::pending_turns(&config.workspace_dir, "t-exit"),
        1
    );

    assert_eq!(run(&config).await, Some(1));
    assert_eq!(
        conversations::pending_turns(&config.workspace_dir, "t-exit"),
        0
    );
    let items = stored(
        &engine,
        MetaFilter {
            kinds: vec![ItemKind::Conversation],
            ..MetaFilter::default()
        },
    )
    .await;
    assert_eq!(items.len(), 1, "the buffered turn was stored");
}

#[tokio::test]
async fn nothing_buffered_flushes_nothing() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config = config_in(&tmp);
    assert_eq!(
        run_within(&config, Duration::from_millis(500)).await,
        Some(0)
    );
}
