use super::*;

use chrono::Utc;
use tinyagents_harness::summarization::ConcatSummarizer;

use crate::memory::lifecycle::hooks::{post_turn, pre_turn_work};
use crate::memory::test_fixtures::{bind_reference, config_in};

struct Failing;

#[async_trait]
impl Summarizer for Failing {
    async fn summarize(&self, _messages: &[Message]) -> Result<SummaryRecord> {
        Err(tinyagents_harness::error::TinyAgentsError::Validation(
            "summarizer down".into(),
        ))
    }
}

async fn bound_turn(tmp: &tempfile::TempDir) -> Arc<MemoryTurn> {
    let config = config_in(tmp);
    bind_reference(&config);
    let identity = tinymemory_tools::layout::identity::Identity::agent("orchestrator")
        .resolve(&crate::memory::scope::identity_pins(&config));
    pre_turn_work(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &identity,
        "t".into(),
        0,
        "our launch date is March 3rd".into(),
        0,
        Utc::now(),
        false,
        None,
        None,
    )
    .await;
    post_turn(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &identity,
        "t".into(),
        1,
        "Noted: launch on March 3rd.".into(),
        Vec::new(),
        Utc::now(),
        None,
    )
    .await;
    Arc::new(MemoryTurn {
        config: Arc::new(config),
        identity,
        thread_id: "t".into(),
        pack: None,
        in_prompt_from: Arc::new(tinymemory_tools::lifecycle::window::InPromptFrom::new(
            0, 20,
        )),
        core: None,
    })
}

fn dropped() -> Vec<Message> {
    vec![
        Message::system("ignored"),
        Message::user("our launch date is March 3rd"),
        Message::assistant("Noted: launch on March 3rd."),
        Message::tool("c1", "tool output is the summarizer's"),
    ]
}

#[test]
fn only_user_and_assistant_text_become_turns() {
    let turns = dropped_turns(&dropped());
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[0].role, Role::User);
    assert_eq!(turns[1].role, Role::Assistant);
}

#[tokio::test]
async fn the_checkpoint_uses_completed_memory_without_waiting_for_refresh() {
    let tmp = tempfile::tempdir().unwrap();
    let turn = bound_turn(&tmp).await;
    let summarizer = MemoryRecallSummarizer::wrap(Box::new(ConcatSummarizer), Some(turn.clone()));
    let cold = summarizer.summarize(&dropped()).await.unwrap();
    assert!(!cold.summary.text().contains(RECALLED_HEADING));
    crate::memory::lifecycle::prefetch::settle_for_test(
        &turn.dispatch_ctx(),
        &turn.config,
        &turn.identity,
        &turn.thread_id,
        true,
    )
    .await;
    let record = summarizer.summarize(&dropped()).await.unwrap();
    let text = record.summary.text();
    assert!(text.contains(RECALLED_HEADING), "{text}");
    assert!(text.contains("March 3rd"));

    let request = SummaryRequest::new(dropped());
    let record = summarizer.summarize_request(&request).await.unwrap();
    assert!(record.summary.text().contains(RECALLED_HEADING));
}

#[tokio::test]
async fn without_recall_the_summarizer_is_unchanged_and_its_failure_is_its_own() {
    let tmp = tempfile::tempdir().unwrap();
    let turn = bound_turn(&tmp).await;

    let plain = MemoryRecallSummarizer::wrap(Box::new(ConcatSummarizer), None);
    let text = plain.summarize(&dropped()).await.unwrap().summary.text();
    assert!(!text.contains(RECALLED_HEADING));

    let mut quiet = (*turn).clone();
    quiet.identity.recall = false;
    let off = MemoryRecallSummarizer::wrap(Box::new(ConcatSummarizer), Some(Arc::new(quiet)));
    assert!(!off
        .summarize(&dropped())
        .await
        .unwrap()
        .summary
        .text()
        .contains(RECALLED_HEADING));

    let failing = MemoryRecallSummarizer::wrap(Box::new(Failing), Some(turn));
    assert!(failing.summarize(&dropped()).await.is_err());
}

#[tokio::test]
async fn with_recall_off_a_compaction_still_moves_the_memory_window() {
    let tmp = tempfile::tempdir().unwrap();
    let turn = bound_turn(&tmp).await;
    let mut quiet = (*turn).clone();
    quiet.identity.recall = false;
    let quiet = Arc::new(quiet);
    let summarizer = MemoryRecallSummarizer::wrap(Box::new(ConcatSummarizer), Some(quiet.clone()));
    let text = summarizer
        .summarize(&dropped())
        .await
        .unwrap()
        .summary
        .text();
    assert!(!text.contains(RECALLED_HEADING), "no recall when it is off");
    assert_eq!(
        quiet.in_prompt().from_turn,
        2,
        "the agent's memory tool still reads by the window"
    );
}

#[test]
fn an_earlier_recall_section_is_dropped_before_the_new_one() {
    let mut summary = Message::user(format!(
        "the summary so far\n\n{RECALLED_HEADING}\n\n- an old recall"
    ));
    strip_recalled(&mut summary);
    assert_eq!(summary.text(), "the summary so far");

    let mut untouched = Message::user("nothing recalled here");
    strip_recalled(&mut untouched);
    assert_eq!(untouched.text(), "nothing recalled here");
}

#[tokio::test]
async fn a_second_compaction_keeps_one_recall_section() {
    let tmp = tempfile::tempdir().unwrap();
    let turn = bound_turn(&tmp).await;
    let summarizer = MemoryRecallSummarizer::wrap(Box::new(ConcatSummarizer), Some(turn.clone()));
    summarizer.summarize(&dropped()).await.unwrap();
    crate::memory::lifecycle::prefetch::settle_for_test(
        &turn.dispatch_ctx(),
        &turn.config,
        &turn.identity,
        &turn.thread_id,
        true,
    )
    .await;
    let first = summarizer.summarize(&dropped()).await.unwrap();
    let mut again = dropped();
    again.insert(0, first.summary.clone());
    let second = summarizer.summarize(&again).await.unwrap();
    assert_eq!(second.summary.text().matches(RECALLED_HEADING).count(), 1);
}

struct HeldRecall {
    inner: tinymemory_api::conformance::ReferenceEngine,
}
#[async_trait::async_trait]
impl tinymemory_api::MemoryEngine for HeldRecall {
    fn descriptor(&self) -> &tinymemory_api::EngineDescriptor {
        self.inner.descriptor()
    }

    async fn health(&self) -> tinymemory_api::EngineHealth {
        self.inner.health().await
    }

    async fn recall(
        &self,
        _req: tinymemory_api::RecallRequest,
    ) -> tinymemory_api::Result<tinymemory_api::RecallAnswer> {
        std::future::pending().await
    }

    async fn fetch(
        &self,
        _req: tinymemory_api::FetchRequest,
    ) -> tinymemory_api::Result<tinymemory_api::FetchPage> {
        std::future::pending().await
    }

    async fn store(
        &self,
        _item: tinymemory_api::StoreItem,
    ) -> tinymemory_api::Result<tinymemory_api::StoreReceipt> {
        std::future::pending().await
    }

    async fn forget(
        &self,
        _target: tinymemory_api::ForgetTarget,
    ) -> tinymemory_api::Result<tinymemory_api::ForgetReport> {
        std::future::pending().await
    }

    async fn list(
        &self,
        _req: tinymemory_api::ListRequest,
    ) -> tinymemory_api::Result<tinymemory_api::ListPage> {
        std::future::pending().await
    }

    async fn export(
        &self,
        _req: tinymemory_api::ListRequest,
    ) -> tinymemory_api::Result<tinymemory_api::ExportPage> {
        std::future::pending().await
    }

    async fn consolidate(
        &self,
        _req: tinymemory_api::ConsolidateRequest,
    ) -> tinymemory_api::Result<tinymemory_api::ConsolidateReceipt> {
        std::future::pending().await
    }
}

#[tokio::test]
async fn a_held_memory_engine_does_not_delay_transcript_compaction() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    crate::memory::engine::install_test_engine(
        &config.workspace_dir,
        Arc::new(HeldRecall {
            inner: tinymemory_api::conformance::ReferenceEngine::new(),
        }),
    );
    let identity = tinymemory_tools::layout::identity::Identity::agent("orchestrator")
        .resolve(&crate::memory::scope::identity_pins(&config));
    let turn = Arc::new(MemoryTurn {
        config: Arc::new(config),
        identity,
        thread_id: "held".into(),
        pack: None,
        in_prompt_from: Arc::new(tinymemory_tools::lifecycle::window::InPromptFrom::new(
            0, 20,
        )),
        core: None,
    });
    let summarizer = MemoryRecallSummarizer::wrap(Box::new(ConcatSummarizer), Some(turn));
    let record = tokio::time::timeout(
        std::time::Duration::from_millis(100),
        summarizer.summarize(&dropped()),
    )
    .await
    .expect("memory must not hold the inner summarizer")
    .unwrap();
    assert!(record.summary.text().contains("March 3rd"));
}

#[tokio::test]
async fn a_compaction_during_the_turn_moves_its_memory_window() {
    let tmp = tempfile::tempdir().unwrap();
    let turn = bound_turn(&tmp).await;
    assert_eq!(turn.in_prompt().from_turn, 0);

    // A failed fold folds nothing: the window stays.
    let failing = MemoryRecallSummarizer::wrap(Box::new(Failing), Some(turn.clone()));
    assert!(failing.summarize(&dropped()).await.is_err());
    assert_eq!(turn.in_prompt().from_turn, 0);

    // One user message folded: its turn and the reply leave the prompt, so the
    // turn's next memory read may return them.
    let summarizer = MemoryRecallSummarizer::wrap(Box::new(ConcatSummarizer), Some(turn.clone()));
    summarizer.summarize(&dropped()).await.unwrap();
    assert_eq!(turn.in_prompt().from_turn, 2);

    // The same span folded again moves nothing; a new one moves it on.
    summarizer.summarize(&dropped()).await.unwrap();
    assert_eq!(turn.in_prompt().from_turn, 2);
    summarizer
        .summarize(&[
            Message::user("what about the venue?"),
            Message::assistant("The hall."),
        ])
        .await
        .unwrap();
    assert_eq!(turn.in_prompt().from_turn, 4);
}
