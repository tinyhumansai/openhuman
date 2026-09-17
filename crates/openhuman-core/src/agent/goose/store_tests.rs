use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use goose_provider_types::conversation::message::MessageContent;
use tinyinference::usage::Usage;
use tokio_util::sync::CancellationToken;

use crate::agent::messages::ConversationMessage;

use super::{
    adapter_tests::*,
    convert::{final_text, goose_to_openhuman},
    store::FileGooseCheckpointStore,
    GooseCheckpointStore, GooseStopReason, GooseTurnAdapter, InMemoryGooseCheckpointStore,
};

struct RejectingStore {
    inner: InMemoryGooseCheckpointStore,
}

#[async_trait]
impl GooseCheckpointStore for RejectingStore {
    async fn load(&self, session_id: &str) -> Result<super::GooseCheckpoint> {
        self.inner.load(session_id).await
    }

    async fn compare_and_swap(
        &self,
        _session_id: &str,
        _expected_revision: u64,
        _checkpoint: super::GooseCheckpoint,
    ) -> Result<()> {
        Err(anyhow!("simulated persistence failure"))
    }

    async fn claim_execution(&self, session_id: &str, call_id: &str) -> Result<bool> {
        self.inner.claim_execution(session_id, call_id).await
    }
}

#[tokio::test]
async fn failed_action_checkpoint_prevents_tool_execution() {
    let store = Arc::new(RejectingStore {
        inner: InMemoryGooseCheckpointStore::default(),
    });
    store.inner.insert(
        "fail",
        GooseTurnAdapter::checkpoint_from_openhuman(&kickoff()).unwrap(),
    );
    let calls = Arc::new(AtomicUsize::new(0));
    let error = adapter(
        store,
        Arc::new(ScriptedModel::new(vec![tool_response(Usage::new(10, 2))])),
        calls.clone(),
        Arc::new(AllowSecurity),
        None,
        CancellationToken::new(),
    )
    .run("fail")
    .await
    .unwrap_err();
    assert!(error
        .to_string()
        .contains("persist Goose state-machine step"));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

struct FailObservationOnceStore {
    inner: InMemoryGooseCheckpointStore,
    failed: AtomicBool,
}

#[async_trait]
impl GooseCheckpointStore for FailObservationOnceStore {
    async fn load(&self, session_id: &str) -> Result<super::GooseCheckpoint> {
        self.inner.load(session_id).await
    }

    async fn compare_and_swap(
        &self,
        session_id: &str,
        expected_revision: u64,
        checkpoint: super::GooseCheckpoint,
    ) -> Result<()> {
        let contains_observation = checkpoint
            .actions
            .values()
            .any(|action| action.observation.is_some());
        if contains_observation && !self.failed.swap(true, Ordering::SeqCst) {
            return Err(anyhow!("simulated observation commit failure"));
        }
        self.inner
            .compare_and_swap(session_id, expected_revision, checkpoint)
            .await
    }

    async fn claim_execution(&self, session_id: &str, call_id: &str) -> Result<bool> {
        self.inner.claim_execution(session_id, call_id).await
    }
}

#[tokio::test]
async fn resume_after_observation_commit_failure_refuses_duplicate_side_effect() {
    let store = Arc::new(FailObservationOnceStore {
        inner: InMemoryGooseCheckpointStore::default(),
        failed: AtomicBool::new(false),
    });
    store.inner.insert(
        "crash-window",
        GooseTurnAdapter::checkpoint_from_openhuman(&kickoff()).unwrap(),
    );
    let model = Arc::new(ScriptedModel::new(vec![
        tool_response(Usage::new(10, 2)),
        final_response("reconciled", Usage::new(20, 3)),
    ]));
    let calls = Arc::new(AtomicUsize::new(0));

    let first_turn = adapter(
        store.clone(),
        model.clone(),
        calls.clone(),
        Arc::new(AllowSecurity),
        None,
        CancellationToken::new(),
    )
    .run("crash-window")
    .await;
    assert!(first_turn.is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    let second_turn = adapter(
        store,
        model,
        calls.clone(),
        Arc::new(AllowSecurity),
        None,
        CancellationToken::new(),
    )
    .run("crash-window")
    .await
    .unwrap();

    assert_eq!(second_turn.stop_reason, GooseStopReason::FinalAnswer);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let observation = second_turn.checkpoint.actions["call-1"]
        .observation
        .as_ref()
        .unwrap();
    assert!(!observation.success);
    assert!(observation.output.contains("refusing to repeat"));
}

#[tokio::test]
async fn qwen_correction_persistence_and_terminal_resume_across_store_handles() {
    let dir = tempfile::tempdir().unwrap();
    let session_id = "qwen-correction-session";
    let store1 = Arc::new(FileGooseCheckpointStore::new(dir.path()));
    store1
        .insert(
            session_id,
            GooseTurnAdapter::checkpoint_from_openhuman(&kickoff()).unwrap(),
        )
        .unwrap();

    let calls = Arc::new(AtomicUsize::new(0));
    let model1 = Arc::new(ScriptedModel::new(vec![final_response(
        "<tool_call>\n{\"name\": \"read_counter\", \"arguments\": {invalid}}\n</tool_call>",
        Usage::new(10, 2),
    )]));
    let mut adapter1 = to_qwen_route(adapter(
        store1.clone(),
        model1,
        calls.clone(),
        Arc::new(AllowSecurity),
        None,
        CancellationToken::new(),
    ));
    adapter1.max_primary_calls = 1;

    let outcome1 = adapter1.run(session_id).await.unwrap();

    assert_eq!(outcome1.stop_reason, GooseStopReason::CallCeiling);
    let checkpoint1 = store1.load(session_id).await.unwrap();
    assert_eq!(checkpoint1.protocol_correction_count, 1);
    assert!(!checkpoint1.terminal_protocol_failure);

    let corrections1: Vec<_> = checkpoint1
        .conversation
        .messages()
        .iter()
        .filter(|m| m.metadata.agent_visible && !m.metadata.user_visible)
        .collect();
    assert_eq!(corrections1.len(), 1);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(checkpoint1.actions.is_empty());

    let oh1 = goose_to_openhuman(&checkpoint1.conversation);
    let oh1_str = serde_json::to_string(&oh1).unwrap();
    assert!(!oh1_str.contains("<tool_call>"));
    assert!(!oh1_str.contains("</tool_call>"));
    let outcome1_str = serde_json::to_string(&outcome1.openhuman_messages).unwrap();
    assert!(!outcome1_str.contains("<tool_call>"));
    assert!(!outcome1_str.contains("</tool_call>"));

    let store2 = Arc::new(FileGooseCheckpointStore::new(dir.path()));
    let model2 = Arc::new(ScriptedModel::new(vec![final_response(
        "<tool_call>\n{\"name\": \"read_counter\", \"arguments\": {invalid_again}}\n</tool_call>",
        Usage::new(10, 2),
    )]));
    let adapter2 = to_qwen_route(adapter(
        store2.clone(),
        model2,
        calls.clone(),
        Arc::new(AllowSecurity),
        None,
        CancellationToken::new(),
    ));

    let outcome2 = adapter2.run(session_id).await.unwrap();

    assert_eq!(outcome2.stop_reason, GooseStopReason::FinalAnswer);
    let checkpoint2 = store2.load(session_id).await.unwrap();
    assert_eq!(checkpoint2.protocol_correction_count, 1);
    assert!(checkpoint2.terminal_protocol_failure);

    let corrections2: Vec<_> = checkpoint2
        .conversation
        .messages()
        .iter()
        .filter(|m| m.metadata.agent_visible && !m.metadata.user_visible)
        .collect();
    assert_eq!(corrections2.len(), 1);

    assert!(final_text(&checkpoint2.conversation).is_some());
    let has_user_visible_answer = outcome2.openhuman_messages.iter().any(|msg| match msg {
        ConversationMessage::Chat(chat) => {
            chat.role.as_str() == "assistant" && !chat.content.is_empty()
        }
        _ => false,
    });
    assert!(has_user_visible_answer);

    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(checkpoint2.actions.is_empty());

    let oh2 = goose_to_openhuman(&checkpoint2.conversation);
    let oh2_str = serde_json::to_string(&oh2).unwrap();
    assert!(!oh2_str.contains("<tool_call>"));
    assert!(!oh2_str.contains("</tool_call>"));
    let outcome2_str = serde_json::to_string(&outcome2.openhuman_messages).unwrap();
    assert!(!outcome2_str.contains("<tool_call>"));
    assert!(!outcome2_str.contains("</tool_call>"));
}

#[tokio::test]
async fn qwen_exact_route_multiple_provider_calls_pairing_and_single_execution() {
    let dir = tempfile::tempdir().unwrap();
    let session_id = "qwen-pairing-session";
    let store = Arc::new(FileGooseCheckpointStore::new(dir.path()));
    store
        .insert(
            session_id,
            GooseTurnAdapter::checkpoint_from_openhuman(&kickoff()).unwrap(),
        )
        .unwrap();

    let calls = Arc::new(AtomicUsize::new(0));
    let model = Arc::new(ScriptedModel::new(vec![
        tool_response(Usage::new(10, 2)),
        final_response("finished successfully", Usage::new(20, 3)),
    ]));

    let run_adapter = to_qwen_route(adapter(
        store.clone(),
        model.clone(),
        calls.clone(),
        Arc::new(AllowSecurity),
        None,
        CancellationToken::new(),
    ));

    let outcome = run_adapter.run(session_id).await.unwrap();

    assert_eq!(outcome.stop_reason, GooseStopReason::FinalAnswer);
    assert_eq!(model.requests.lock().unwrap().len(), 2);

    assert_eq!(outcome.checkpoint.actions.len(), 1);
    assert!(outcome.checkpoint.actions.contains_key("call-1"));

    for (call_id, action) in &outcome.checkpoint.actions {
        let observation = action
            .observation
            .as_ref()
            .expect("matching observation for accepted action");
        assert_eq!(&observation.call_id, call_id);
        assert!(observation.success);
        assert_eq!(observation.output, "value:\"alpha\"");
    }

    let response_ids: Vec<_> = outcome
        .checkpoint
        .conversation
        .messages()
        .iter()
        .flat_map(|m| {
            m.content.iter().filter_map(|c| match c {
                MessageContent::ToolResponse(r) => Some(r.id.clone()),
                _ => None,
            })
        })
        .collect();
    assert_eq!(response_ids.len(), 1);
    assert_eq!(response_ids[0], "call-1");

    assert_eq!(calls.load(Ordering::SeqCst), 1);

    let fresh_store = Arc::new(FileGooseCheckpointStore::new(dir.path()));
    let duplicate_claim = fresh_store
        .claim_execution(session_id, "call-1")
        .await
        .unwrap();
    assert!(
        !duplicate_claim,
        "fresh handle cannot duplicate an already claimed effect"
    );

    for resp_id in &response_ids {
        assert!(
            outcome.checkpoint.actions.contains_key(resp_id),
            "observation must belong to an accepted action"
        );
    }

    let resumed = to_qwen_route(adapter(
        fresh_store,
        Arc::new(ScriptedModel::new(Vec::new())),
        calls.clone(),
        Arc::new(AllowSecurity),
        None,
        CancellationToken::new(),
    ))
    .run(session_id)
    .await
    .unwrap();
    assert_eq!(resumed.stop_reason, GooseStopReason::FinalAnswer);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(resumed.checkpoint.actions.len(), 1);
    assert!(resumed.checkpoint.actions["call-1"].observation.is_some());
}

#[tokio::test]
async fn loop_guard_state_persistence_and_conversation_replacement_preservation() {
    use super::types::GooseCheckpoint;
    use goose_agent::machine::EffectHandler;
    use goose_agent::operation::ConversationEffect;

    let dir = tempfile::tempdir().unwrap();
    let session_id = "loop-guard-persistence-test";
    let store = Arc::new(FileGooseCheckpointStore::new(dir.path()));

    // 1. Backward compatibility: deserialize JSON without loop-guard fields
    let mut legacy_val =
        serde_json::to_value(GooseTurnAdapter::checkpoint_from_openhuman(&kickoff()).unwrap())
            .unwrap();
    let legacy_map = legacy_val.as_object_mut().unwrap();
    legacy_map.remove("last_call_signature");
    legacy_map.remove("last_failure_type");
    legacy_map.remove("repeated_failure_count");
    legacy_map.remove("no_progress_count");
    legacy_map.remove("unavailable_routes");
    legacy_map.remove("completion_state");
    legacy_map.remove("terminal_reason");
    let legacy_ckpt: GooseCheckpoint = serde_json::from_value(legacy_val).unwrap();
    assert_eq!(legacy_ckpt.last_call_signature, None);
    assert_eq!(legacy_ckpt.last_failure_type, None);
    assert_eq!(legacy_ckpt.repeated_failure_count, 0);
    assert_eq!(legacy_ckpt.no_progress_count, 0);
    assert!(legacy_ckpt.unavailable_routes.is_empty());
    assert_eq!(legacy_ckpt.completion_state, None);
    assert_eq!(legacy_ckpt.terminal_reason, None);

    // 2. Build initial checkpoint with loop guard state
    let mut initial_ckpt = GooseTurnAdapter::checkpoint_from_openhuman(&kickoff()).unwrap();
    initial_ckpt.last_call_signature = Some("read_file:{\"path\":\"foo.rs\"}".into());
    initial_ckpt.last_failure_type = Some("file_not_found".into());
    initial_ckpt.repeated_failure_count = 1;
    initial_ckpt.no_progress_count = 1;
    initial_ckpt.unavailable_routes = vec!["paid_generation".into()];
    initial_ckpt.completion_state = Some(
        crate::agent::primary_orchestration::CompletionStatus::Incomplete {
            reason: "awaiting file edit".into(),
            needs_final_model_call: true,
        },
    );
    initial_ckpt.terminal_reason = Some("test_reason".into());

    // Insert into store
    store.insert(session_id, initial_ckpt.clone()).unwrap();

    // 3. Reload from fresh store handle and assert all fields survived exactly
    let fresh_store = Arc::new(FileGooseCheckpointStore::new(dir.path()));
    let loaded = fresh_store.load(session_id).await.unwrap();
    assert_eq!(loaded.last_call_signature, initial_ckpt.last_call_signature);
    assert_eq!(loaded.last_failure_type, initial_ckpt.last_failure_type);
    assert_eq!(loaded.repeated_failure_count, 1);
    assert_eq!(loaded.no_progress_count, 1);
    assert_eq!(loaded.unavailable_routes, vec!["paid_generation"]);
    assert_eq!(loaded.completion_state, initial_ckpt.completion_state);
    assert_eq!(loaded.terminal_reason, Some("test_reason".into()));

    // 4. Test effect application via CheckpointRuntime
    let runtime = super::store::CheckpointRuntime {
        store: fresh_store.clone(),
    };
    let session = super::types::GooseSession {
        id: session_id.to_string(),
        checkpoint: loaded.clone(),
    };
    let (tx, _rx) = tokio::sync::mpsc::channel(16);
    let emitter = goose_agent::operation::Emitter::new(tx, CancellationToken::new());
    let mut effects = vec![
        super::types::OpenHumanEffect::SetLastCallSignature(Some(
            "read_file:{\"path\":\"bar.rs\"}".into(),
        )),
        super::types::OpenHumanEffect::RecordFailure("file_not_found".into()),
        super::types::OpenHumanEffect::IncrementNoProgress,
        super::types::OpenHumanEffect::MarkRouteUnavailable("expensive_search".into()),
        super::types::OpenHumanEffect::SetCompletionState(Some(
            crate::agent::primary_orchestration::CompletionStatus::Complete,
        )),
        super::types::OpenHumanEffect::SetTerminalReason(Some("completed_cleanly".into())),
    ];
    runtime
        .apply_effects(&session, &mut effects, &emitter)
        .await
        .unwrap();

    // Verify persisted state after effects
    let after_effects = fresh_store.load(session_id).await.unwrap();
    assert_eq!(after_effects.revision, 1);
    assert_eq!(
        after_effects.last_call_signature.as_deref(),
        Some("read_file:{\"path\":\"bar.rs\"}")
    );
    assert_eq!(
        after_effects.last_failure_type.as_deref(),
        Some("file_not_found")
    );
    assert_eq!(after_effects.repeated_failure_count, 2);
    assert_eq!(after_effects.no_progress_count, 2);
    assert_eq!(
        after_effects.unavailable_routes,
        vec!["paid_generation", "expensive_search"]
    );
    assert_eq!(
        after_effects.completion_state,
        Some(crate::agent::primary_orchestration::CompletionStatus::Complete)
    );
    assert_eq!(
        after_effects.terminal_reason.as_deref(),
        Some("completed_cleanly")
    );

    // 5. Test ReplaceConversation effect preserves loop guard state
    let session2 = super::types::GooseSession {
        id: session_id.to_string(),
        checkpoint: after_effects.clone(),
    };
    let new_conv = GooseTurnAdapter::checkpoint_from_openhuman(&kickoff())
        .unwrap()
        .conversation;
    let mut replace_effects = vec![super::types::OpenHumanEffect::Conversation(
        ConversationEffect::ReplaceConversation(new_conv),
    )];
    runtime
        .apply_effects(&session2, &mut replace_effects, &emitter)
        .await
        .unwrap();

    let after_replace = fresh_store.load(session_id).await.unwrap();
    assert_eq!(after_replace.revision, 2);
    assert_eq!(
        after_replace.last_call_signature,
        after_effects.last_call_signature
    );
    assert_eq!(
        after_replace.last_failure_type,
        after_effects.last_failure_type
    );
    assert_eq!(
        after_replace.repeated_failure_count,
        after_effects.repeated_failure_count
    );
    assert_eq!(
        after_replace.no_progress_count,
        after_effects.no_progress_count
    );
    assert_eq!(
        after_replace.unavailable_routes,
        after_effects.unavailable_routes
    );
    assert_eq!(
        after_replace.completion_state,
        after_effects.completion_state
    );
    assert_eq!(after_replace.terminal_reason, after_effects.terminal_reason);
}
