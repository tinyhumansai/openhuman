use super::part_04_tests::scripted_reply;
use super::*;

// ── Lane C: auto-recall of facts about the user (#6040) ──────────────────────
//
// The lane is exercised through a real `turn()`: a scripted source stands in
// for the memory driver, the scripted chat model records every request, and
// the assertions read the user message the model was actually sent.

use crate::openhuman::memory::api::error::MemoryError;
use crate::openhuman::memory::api::provider::retrieval::{
    FastRetrieveQuery, RetrievalHit, RetrievalNodeKind, RetrievalResponse,
};
use crate::openhuman::memory::api::types::NamespaceMemoryHit;
use crate::openhuman::memory::auto_recall::{
    AutoRecall, AutoRecallSource, AUTO_RECALL_BANNER, AUTO_RECALL_HINT,
};
use crate::openhuman::memory::guard::test_support::{
    embedded_policy, guarded_with, namespace_hit, RecordingProvider,
};

const IDOL_QUESTION: &str = "who is my idol and why?";
const IDOL_FACT: &str = "Idol: Virat Kohli, because of his dedication and consistency";

struct ScriptedAutoRecallSource {
    hits: Vec<RetrievalHit>,
    delay: Duration,
    calls: AtomicUsize,
    notes_calls: AtomicUsize,
}

impl ScriptedAutoRecallSource {
    fn with_fact(content: &str) -> Arc<Self> {
        Arc::new(Self {
            hits: vec![RetrievalHit {
                node_id: "leaf-1".into(),
                node_kind: RetrievalNodeKind::Leaf,
                tree_id: String::new(),
                tree_kind: None,
                tree_scope: "folder:profile".into(),
                level: 0,
                content: content.to_string(),
                entities: Vec::new(),
                topics: Vec::new(),
                time_range_start: chrono::DateTime::<chrono::Utc>::default(),
                time_range_end: chrono::DateTime::<chrono::Utc>::default(),
                score: 0.9,
                child_ids: Vec::new(),
                source_ref: None,
            }],
            delay: Duration::ZERO,
            calls: AtomicUsize::new(0),
            notes_calls: AtomicUsize::new(0),
        })
    }

    fn slow(content: &str, delay: Duration) -> Arc<Self> {
        let mut source = Arc::try_unwrap(Self::with_fact(content))
            .ok()
            .expect("fresh arc");
        source.delay = delay;
        Arc::new(source)
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn notes_calls(&self) -> usize {
        self.notes_calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl AutoRecallSource for ScriptedAutoRecallSource {
    async fn fast_retrieve(
        &self,
        _query: &str,
        _options: FastRetrieveQuery,
    ) -> Result<RetrievalResponse, MemoryError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if !self.delay.is_zero() {
            tokio::time::sleep(self.delay).await;
        }
        Ok(RetrievalResponse {
            total: self.hits.len(),
            hits: self.hits.clone(),
            truncated: false,
        })
    }

    async fn recall_namespace_scored(
        &self,
        _namespace: &str,
        _query: &str,
        _limit: usize,
    ) -> Result<Vec<NamespaceMemoryHit>, MemoryError> {
        // This fake scripts the tree; the notes leg is exercised through the
        // production wiring below, over a `RecordingProvider`.
        self.notes_calls.fetch_add(1, Ordering::SeqCst);
        Ok(Vec::new())
    }
}

/// A turn-capable agent with Lane C bound (or deliberately absent), over the
/// same real embedded store the other turn tests use.
fn make_agent_with_auto_recall(
    provider: Arc<dyn ChatModel<()>>,
    auto_recall: Option<Arc<AutoRecall>>,
) -> Agent {
    let workspace = tempfile::TempDir::new().expect("temp workspace");
    let workspace_path = workspace.path().to_path_buf();
    std::mem::forget(workspace);
    let _memory_cfg = crate::openhuman::config::MemoryConfig {
        backend: "none".into(),
        ..crate::openhuman::config::MemoryConfig::default()
    };
    let mem: Arc<dyn Memory> = crate::openhuman::memory::test_support::noop_memory();

    Agent::builder()
        .chat_model(provider)
        .tools(vec![])
        .memory(mem)
        .auto_recall(auto_recall)
        .tool_dispatcher(Box::new(XmlToolDispatcher))
        .config(crate::openhuman::config::AgentConfig::default())
        .context_config(crate::openhuman::config::ContextConfig::default())
        .workspace_dir(workspace_path)
        .event_context("turn-test-session", "turn-test-channel")
        .build()
        .unwrap()
}

/// The messages the model was sent on the first (only) request, split by role.
async fn first_request(provider: &SequenceProvider) -> (Vec<String>, Vec<String>) {
    let requests = provider.requests.lock().await;
    let request = requests.first().expect("the model was called once");
    let system = request
        .iter()
        .filter(|m| m.role == "system")
        .map(|m| m.content.clone())
        .collect();
    let user = request
        .iter()
        .filter(|m| m.role == "user")
        .map(|m| m.content.clone())
        .collect();
    (system, user)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auto_recall_block_rides_the_user_message_for_a_personal_question() {
    let source = ScriptedAutoRecallSource::with_fact(IDOL_FACT);
    let lane = Arc::new(AutoRecall::new(source.clone(), true, None));
    let provider_impl = scripted_reply("Virat Kohli, for his dedication and consistency.");
    let provider: Arc<dyn ChatModel<()>> = provider_impl.clone();
    let mut agent = make_agent_with_auto_recall(provider, Some(lane));

    let reply = agent.turn(IDOL_QUESTION).await.expect("turn succeeds");
    assert!(reply.contains("Virat"));
    assert_eq!(source.calls(), 1, "one bounded lookup for one gated turn");
    assert_eq!(source.notes_calls(), 1, "and one notes lookup beside it");

    let (system, user) = first_request(&provider_impl).await;
    let last_user = user.last().expect("a user message");
    assert!(
        last_user.contains(AUTO_RECALL_BANNER) && last_user.contains("Virat Kohli"),
        "the block must ride the user message: {last_user}"
    );
    assert!(
        last_user.contains(IDOL_QUESTION),
        "the user's own words must follow the block: {last_user}"
    );
    assert!(
        system.iter().all(|s| !s.contains(AUTO_RECALL_BANNER)),
        "the block must never land in the system prompt"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auto_recall_stays_silent_and_costs_nothing_on_small_talk() {
    let source = ScriptedAutoRecallSource::with_fact(IDOL_FACT);
    let lane = Arc::new(AutoRecall::new(source.clone(), true, None));
    let provider_impl = scripted_reply("Sunny, probably.");
    let provider: Arc<dyn ChatModel<()>> = provider_impl.clone();
    let mut agent = make_agent_with_auto_recall(provider, Some(lane));

    agent
        .turn("what's the weather today")
        .await
        .expect("turn succeeds");
    assert_eq!(source.calls(), 0, "a closed gate never reaches the store");
    let (_, user) = first_request(&provider_impl).await;
    assert!(user.iter().all(|u| !u.contains(AUTO_RECALL_BANNER)));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auto_recall_disabled_by_the_hooks_switch_injects_nothing() {
    let source = ScriptedAutoRecallSource::with_fact(IDOL_FACT);
    let lane = Arc::new(AutoRecall::new(source.clone(), false, None));
    let provider_impl = scripted_reply("I don't have that stored.");
    let provider: Arc<dyn ChatModel<()>> = provider_impl.clone();
    let mut agent = make_agent_with_auto_recall(provider, Some(lane));

    agent.turn(IDOL_QUESTION).await.expect("turn succeeds");
    assert_eq!(source.calls(), 0);
    let (_, user) = first_request(&provider_impl).await;
    assert!(user.iter().all(|u| !u.contains(AUTO_RECALL_BANNER)));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auto_recall_timeout_yields_an_ordinary_turn() {
    let source = ScriptedAutoRecallSource::slow(IDOL_FACT, Duration::from_millis(300));
    let lane = Arc::new(
        AutoRecall::new(source.clone(), true, None).with_budget(Duration::from_millis(20)),
    );
    let provider_impl = scripted_reply("Still here.");
    let provider: Arc<dyn ChatModel<()>> = provider_impl.clone();
    let mut agent = make_agent_with_auto_recall(provider, Some(lane));

    let reply = agent.turn(IDOL_QUESTION).await.expect("turn succeeds");
    assert_eq!(reply, "Still here.");
    assert_eq!(source.calls(), 1);
    let (_, user) = first_request(&provider_impl).await;
    assert!(user.iter().all(|u| !u.contains(AUTO_RECALL_BANNER)));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_session_without_the_lane_turns_as_before() {
    let provider_impl = scripted_reply("Hello.");
    let provider: Arc<dyn ChatModel<()>> = provider_impl.clone();
    let mut agent = make_agent_with_auto_recall(provider, None);

    let reply = agent.turn(IDOL_QUESTION).await.expect("turn succeeds");
    assert_eq!(reply, "Hello.");
    let (_, user) = first_request(&provider_impl).await;
    assert!(user.iter().all(|u| !u.contains(AUTO_RECALL_BANNER)));
}

// ── Lane C: the notes leg (#6063) ────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auto_recall_injects_a_note_written_by_memory_store_when_the_tree_is_empty() {
    // The #6063 repro through the production wiring: `memory_store` filed the
    // fact in `global`, the tree never saw it, and the lane is bound from the
    // guard exactly as `bind_session_memory` binds it.
    let provider = RecordingProvider::new().with_namespace_hits(vec![namespace_hit(
        "global",
        "favourite_tea_oolong",
        "User's favourite tea is oolong.",
        0.72,
    )]);
    let (provider, guard) = guarded_with(provider, embedded_policy());
    let lane = Arc::new(AutoRecall::from_guard(Arc::new(guard)));
    let model_impl = scripted_reply("Oolong.");
    let model: Arc<dyn ChatModel<()>> = model_impl.clone();
    let mut agent = make_agent_with_auto_recall(model, Some(lane));

    let reply = agent
        .turn("What's my favourite tea?")
        .await
        .expect("turn succeeds");
    assert_eq!(reply, "Oolong.");

    let (system, user) = first_request(&model_impl).await;
    let last_user = user.last().expect("a user message");
    assert!(last_user.contains(AUTO_RECALL_BANNER), "{last_user}");
    assert!(
        last_user.contains(AUTO_RECALL_HINT),
        "the block says what it is: {last_user}"
    );
    assert!(
        last_user.contains("- User's favourite tea is oolong. (note: favourite_tea_oolong)"),
        "the note rides the user message: {last_user}"
    );
    assert!(
        system.iter().all(|s| !s.contains(AUTO_RECALL_BANNER)),
        "the block must never land in the system prompt"
    );

    let mut methods: Vec<String> = provider.calls().into_iter().map(|c| c.method).collect();
    methods.sort();
    assert_eq!(
        methods,
        vec![
            "retrieval.fast_retrieve".to_string(),
            "retrieval.recall_namespace_scored".to_string()
        ],
        "both legs, once each, through the guard"
    );
    let notes_call = provider
        .calls()
        .into_iter()
        .find(|c| c.method == "retrieval.recall_namespace_scored")
        .expect("the notes leg");
    assert_eq!(
        notes_call.content.as_deref(),
        Some("namespace=global limit=3")
    );
}
