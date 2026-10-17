use super::*;

use std::time::Duration;

use tinymemory_api::{ItemKind, MetaFilter};

use crate::memory::error::{INVALID_REQUEST, MEMORY_OFF};
use crate::memory::test_fixtures::{bind_reference, config_in, stored};
use crate::threads::store::{ConversationMessage, CreateConversationThread};

fn message(id: &str, sender: &str, content: &str, at: &str) -> ConversationMessage {
    ConversationMessage {
        id: id.to_string(),
        content: content.to_string(),
        message_type: "text".to_string(),
        extra_metadata: serde_json::Value::Null,
        sender: sender.to_string(),
        created_at: at.to_string(),
    }
}

async fn seed_thread(workspace: &Path, thread_id: &str, messages: &[(&str, &str)]) {
    threads::ensure_thread(
        crate::core::runtime::DispatchCtx::ambient(),
        workspace.to_path_buf(),
        CreateConversationThread {
            id: thread_id.to_string(),
            title: thread_id.to_string(),
            created_at: "2026-09-01T10:00:00Z".to_string(),
            parent_thread_id: None,
            labels: None,
            personality_id: None,
            working_dir: None,
        },
    )
    .await
    .unwrap();
    for (index, (sender, content)) in messages.iter().enumerate() {
        threads::append_message(
            crate::core::runtime::DispatchCtx::ambient(),
            workspace.to_path_buf(),
            thread_id.to_string(),
            message(
                &format!("{thread_id}-{index}"),
                sender,
                content,
                &format!("2026-09-01T10:{:02}:00Z", index),
            ),
        )
        .await
        .unwrap();
    }
}

async fn wait_done(config: &Config) -> BackfillView {
    for _ in 0..200 {
        let view = status(&DispatchCtx::ambient(), config).await.unwrap();
        if view.state.phase != ImportPhase::Running {
            return view;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the backfill did not finish");
}

#[tokio::test]
async fn refuses_without_consent_and_with_memory_off() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let error = start(
        &DispatchCtx::ambient(),
        &config,
        BackfillStartParams { consent: false },
    )
    .await
    .unwrap_err();
    assert_eq!(error.code(), INVALID_REQUEST);
    let error = start(
        &DispatchCtx::ambient(),
        &config,
        BackfillStartParams { consent: true },
    )
    .await
    .unwrap_err();
    assert_eq!(error.code(), MEMORY_OFF);
}

#[tokio::test]
async fn stores_past_chats_once_and_skips_what_live_logging_took() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let engine = bind_reference(&config);
    let workspace = config.workspace_dir.clone();

    seed_thread(
        &workspace,
        "old-thread",
        &[
            ("user", "What is ownership?"),
            ("agent", "Values have one owner."),
            ("user", "And borrowing?"),
            ("agent", "References without ownership."),
        ],
    )
    .await;
    seed_thread(
        &workspace,
        "live-thread",
        &[
            ("user", "Before logging"),
            ("agent", "An old answer."),
            ("user", "After logging"),
            ("agent", "A live answer."),
        ],
    )
    .await;
    // The lifecycle logged live-thread's second turn (user index 2).
    let identity = MemoryIdentity::agent(MAIN_AGENT).resolve(&config);
    crate::memory::lifecycle::hooks::pre_turn(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &identity,
        crate::memory::lifecycle::hooks::PreTurnInput {
            thread_id: "live-thread".into(),
            turn_index: 2,
            user_text: "After logging".into(),
            in_prompt_from: 0,
            at: Utc::now(),
            resumed_after_compaction: false,
            observed_actor: None,
        },
    )
    .await;

    let before = status(&DispatchCtx::ambient(), &config).await.unwrap();
    assert_eq!(before.state.phase, ImportPhase::Idle);
    assert_eq!((before.pending_threads, before.pending_turns), (2, 3));

    let started = start(
        &DispatchCtx::ambient(),
        &config,
        BackfillStartParams { consent: true },
    )
    .await
    .unwrap();
    assert_eq!(started.state.threads_total, 2);
    let done = wait_done(&config).await;
    assert_eq!(done.state.phase, ImportPhase::Done, "{:?}", done.state);
    assert_eq!(done.state.turns_stored, 3);
    assert_eq!(done.state.items_stored, 6);
    assert!(done.state.finished_at.is_some());
    assert_eq!((done.pending_threads, done.pending_turns), (0, 0));

    let filter = MetaFilter {
        kinds: vec![ItemKind::Conversation],
        tags_any: vec![BACKFILL_TAG.to_string()],
        ..MetaFilter::default()
    };
    let items = stored(&engine, filter.clone()).await;
    assert_eq!(items.len(), 6);
    assert!(
        !items.iter().any(|hit| hit.text.contains("After logging")),
        "live's turn is not re-stored"
    );

    // A second run has nothing to send.
    start(
        &DispatchCtx::ambient(),
        &config,
        BackfillStartParams { consent: true },
    )
    .await
    .unwrap();
    let again = wait_done(&config).await;
    assert_eq!(again.state.items_stored, 0);
    assert_eq!(stored(&engine, filter).await.len(), 6);
}

/// The reference engine, recording how long each bulk store asked to wait.
struct WaitRecorder {
    inner: tinymemory_api::conformance::ReferenceEngine,
    waits: std::sync::Mutex<Vec<tinymemory_api::WaitFor>>,
}

#[async_trait::async_trait]
impl tinymemory_api::MemoryEngine for WaitRecorder {
    fn descriptor(&self) -> &tinymemory_api::EngineDescriptor {
        self.inner.descriptor()
    }
    async fn health(&self) -> tinymemory_api::EngineHealth {
        self.inner.health().await
    }
    async fn recall(
        &self,
        req: tinymemory_api::RecallRequest,
    ) -> tinymemory_api::Result<tinymemory_api::RecallAnswer> {
        self.inner.recall(req).await
    }
    async fn fetch(
        &self,
        req: tinymemory_api::FetchRequest,
    ) -> tinymemory_api::Result<tinymemory_api::FetchPage> {
        self.inner.fetch(req).await
    }
    async fn store(
        &self,
        item: tinymemory_api::StoreItem,
    ) -> tinymemory_api::Result<tinymemory_api::StoreReceipt> {
        self.inner.store(item).await
    }
    async fn store_many_with(
        &self,
        items: Vec<tinymemory_api::StoreItem>,
        options: tinymemory_api::WriteOptions,
    ) -> tinymemory_api::Result<Vec<tinymemory_api::StoreReceipt>> {
        self.waits.lock().unwrap().push(options.wait);
        self.inner.store_many_with(items, options).await
    }
    async fn forget(
        &self,
        target: tinymemory_api::ForgetTarget,
    ) -> tinymemory_api::Result<tinymemory_api::ForgetReport> {
        self.inner.forget(target).await
    }
    async fn list(
        &self,
        req: tinymemory_api::ListRequest,
    ) -> tinymemory_api::Result<tinymemory_api::ListPage> {
        self.inner.list(req).await
    }
}

#[tokio::test]
async fn batches_are_stored_accepted_and_the_last_one_visible() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let engine = std::sync::Arc::new(WaitRecorder {
        inner: tinymemory_api::conformance::ReferenceEngine::new(),
        waits: std::sync::Mutex::new(Vec::new()),
    });
    crate::memory::engine::install_test_engine(&config.workspace_dir, engine.clone());
    seed_thread(
        &config.workspace_dir,
        "old-thread",
        &[
            ("user", "What is ownership?"),
            ("agent", "Values have one owner."),
            ("user", "And borrowing?"),
            ("agent", "References without ownership."),
        ],
    )
    .await;
    seed_thread(
        &config.workspace_dir,
        "older-thread",
        &[
            ("user", "What is a lifetime?"),
            ("agent", "How long a reference is valid."),
        ],
    )
    .await;

    start(
        &DispatchCtx::ambient(),
        &config,
        BackfillStartParams { consent: true },
    )
    .await
    .unwrap();
    let done = wait_done(&config).await;
    assert_eq!(done.state.phase, ImportPhase::Done, "{:?}", done.state);
    assert_eq!(done.state.items_stored, 6);

    let waits = engine.waits.lock().unwrap().clone();
    assert_eq!(waits.len(), 2, "one batch per thread: {waits:?}");
    let (last, earlier) = waits.split_last().expect("the backfill stores in bulk");
    assert!(
        earlier
            .iter()
            .all(|wait| *wait == tinymemory_api::WaitFor::Accepted),
        "every batch but the last is stored accepted: {waits:?}"
    );
    assert_eq!(
        *last,
        tinymemory_api::WaitFor::Visible,
        "the last batch waits until it is visible, so a done run is searchable"
    );
    let filter = MetaFilter {
        kinds: vec![ItemKind::Conversation],
        tags_any: vec![BACKFILL_TAG.to_string()],
        ..MetaFilter::default()
    };
    assert_eq!(
        stored(&engine.inner, filter).await.len(),
        6,
        "an engine without accepted writes still stores everything"
    );
}
