//! Recall for the current message (`MemoryBinding::recall_each_turn`), end to
//! end: a real turn on a stateless runtime over TinyMemory's reference engine.
//!
//! - Recall that answers within the deadline reaches the turn's first model
//!   request as a note after the user's message, and is never written into
//!   the transcript.
//! - Recall slower than the deadline leaves the turn without a note, and the
//!   turn still completes.
//! - Without `recall_each_turn` a first turn carries no note: it reads only
//!   packs completed for an earlier turn, as before.
//!
//! One `#[test]` because a runtime claims a process-wide slot, and the host
//! engine it installs is process-wide too.

mod common;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use common::{chat_requests, offline_config, provider, runtime, stub_backend};
use openhuman_embed::memory::api::conformance::ReferenceEngine;
use openhuman_embed::memory::api::{self as memory_api, MemoryEngine};
use openhuman_embed::memory::LearnParams;
use openhuman_embed::{
    Access, AgentSpec, MemoryBinding, Provider, Runtime, SessionStoreProvider, Workspace,
};
use tinyagents_session::port::InMemorySessionStores;

/// How the pack opens in a model request (`tinymemory_tools`' `OPEN_TAG`).
const MEMORY_TAG: &str = "<memory-context>";
const QUESTION: &str = "what colour do I like?";
const ROOT: &str = "team:acme";
/// Bounds every turn, so a regression that blocks on recall fails here
/// instead of hanging the runner.
const TURN_BUDGET: Duration = Duration::from_secs(120);

/// The reference engine, whose reads can be held back forever on demand.
struct HoldableEngine {
    inner: ReferenceEngine,
    hold_reads: AtomicBool,
}

impl HoldableEngine {
    async fn read_gate(&self) {
        if self.hold_reads.load(Ordering::SeqCst) {
            std::future::pending::<()>().await;
        }
    }
}

#[async_trait::async_trait]
impl MemoryEngine for HoldableEngine {
    fn descriptor(&self) -> &memory_api::EngineDescriptor {
        self.inner.descriptor()
    }

    async fn health(&self) -> memory_api::EngineHealth {
        self.inner.health().await
    }

    async fn recall(
        &self,
        req: memory_api::RecallRequest,
    ) -> memory_api::Result<memory_api::RecallAnswer> {
        self.read_gate().await;
        self.inner.recall(req).await
    }

    async fn fetch(
        &self,
        req: memory_api::FetchRequest,
    ) -> memory_api::Result<memory_api::FetchPage> {
        self.read_gate().await;
        self.inner.fetch(req).await
    }

    async fn store(
        &self,
        item: memory_api::StoreItem,
    ) -> memory_api::Result<memory_api::StoreReceipt> {
        self.inner.store(item).await
    }

    async fn forget(
        &self,
        target: memory_api::ForgetTarget,
    ) -> memory_api::Result<memory_api::ForgetReport> {
        self.inner.forget(target).await
    }

    async fn list(&self, req: memory_api::ListRequest) -> memory_api::Result<memory_api::ListPage> {
        self.read_gate().await;
        self.inner.list(req).await
    }

    async fn get(&self, req: memory_api::GetRequest) -> memory_api::Result<Vec<memory_api::Hit>> {
        self.read_gate().await;
        self.inner.get(req).await
    }
}

/// The messages of a recorded chat-completion request, as JSON text each.
fn messages(request: &wiremock::Request) -> Vec<String> {
    let body: serde_json::Value = serde_json::from_slice(&request.body).expect("a JSON request");
    body["messages"]
        .as_array()
        .expect("a messages array")
        .iter()
        .map(|message| message["content"].to_string())
        .collect()
}

#[test]
fn recall_each_turn_reaches_the_first_request_in_time_and_never_the_transcript() {
    let _ = env_logger::builder().is_test(true).try_init();
    runtime().block_on(async {
        tokio::spawn(async move {
            let engine = Arc::new(HoldableEngine {
                inner: ReferenceEngine::new(),
                hold_reads: AtomicBool::new(false),
            });
            let backend = stub_backend().await;
            let store = Arc::new(InMemorySessionStores::new());
            let mut config = offline_config();
            config.modules.allow_download = false;
            let runtime = Runtime::builder()
                .config(config)
                .workspace(Workspace::stateless())
                .backend_url(backend.uri())
                .session_store(store.clone())
                .memory_engine(engine.clone())
                .build()
                .await
                .expect("a stateless runtime over the reference engine");
            let learned: LearnParams = serde_json::from_value(serde_json::json!({
                "text": "The user's favourite colour is teal"
            }))
            .expect("learn params");
            runtime
                .memory(ROOT)
                .expect("the tenant's memory")
                .learn(learned)
                .await
                .expect("a learning to recall");

            let agent_on = |id: &str, model: &wiremock::MockServer, binding: MemoryBinding| {
                runtime
                    .agent(
                        AgentSpec::new(id)
                            .provider(
                                Provider::openai_compatible(
                                    format!("{}/v1", model.uri()),
                                    "sk-test",
                                )
                                .model("recall-model"),
                            )
                            .access(Access::readonly())
                            .memory(binding),
                    )
                    .expect("the agent instantiates")
            };
            let ask = |agent: openhuman_embed::Agent| async move {
                tokio::time::timeout(TURN_BUDGET, agent.turn(QUESTION).send())
                    .await
                    .expect("the turn finishes")
                    .expect("the turn succeeds")
            };

            // ── in time: the note rides the first request, after the question ──
            let in_time_model = provider("noted").await;
            let in_time = agent_on(
                "in-time",
                &in_time_model,
                MemoryBinding::new("in-time")
                    .root(ROOT)
                    .recall_each_turn(Duration::from_secs(30)),
            );
            let reply = ask(in_time.clone()).await;
            let requests = chat_requests(&in_time_model).await;
            let first = messages(requests.first().expect("a model request"));
            let tail = first.last().expect("a message");
            assert!(
                tail.contains(MEMORY_TAG) && tail.contains("teal"),
                "the pack is the request's tail: {first:#?}"
            );
            let sent = first.join("\n");
            let question_at = sent.find(QUESTION).expect("the question was sent");
            assert!(
                question_at < sent.find(MEMORY_TAG).expect("the note was sent"),
                "the note follows the user's message: {first:#?}"
            );
            let transcript = store
                .for_agent("in-time")
                .transcripts
                .root_for_thread(&reply.session_id)
                .expect("the thread is in the agent's store")
                .read_session()
                .expect("readable")
                .expect("written");
            assert!(
                transcript
                    .messages
                    .iter()
                    .any(|message| message.content.contains(QUESTION)),
                "the turn was persisted"
            );
            assert!(
                transcript.messages.iter().all(|message| {
                    !message.content.contains(MEMORY_TAG) && !message.content.contains("teal")
                }),
                "the note is never persisted: {:#?}",
                transcript.messages
            );

            // ── not asked: a first turn still gets no note ─────────────────
            let plain_model = provider("noted").await;
            let plain = agent_on(
                "plain",
                &plain_model,
                MemoryBinding::new("plain").root(ROOT),
            );
            ask(plain.clone()).await;
            let requests = chat_requests(&plain_model).await;
            let first = messages(requests.first().expect("a model request")).join("\n");
            assert!(
                !first.contains(MEMORY_TAG),
                "without recall_each_turn a cold first turn is unchanged: {first}"
            );

            // ── late: the turn goes on without a note ──────────────────────
            engine.hold_reads.store(true, Ordering::SeqCst);
            let late_model = provider("noted").await;
            let late = agent_on(
                "late",
                &late_model,
                MemoryBinding::new("late")
                    .root(ROOT)
                    .recall_each_turn(Duration::from_millis(300)),
            );
            let reply = ask(late.clone()).await;
            assert_eq!(reply.reply, "noted");
            let requests = chat_requests(&late_model).await;
            let first = messages(requests.first().expect("a model request")).join("\n");
            assert!(
                !first.contains(MEMORY_TAG),
                "a late recall adds no note: {first}"
            );

            // Removing awaits each agent's MCP close, so the runtime drops clean.
            for agent in ["in-time", "plain", "late"] {
                runtime
                    .remove_agent(agent)
                    .await
                    .unwrap_or_else(|error| panic!("{agent} is removed: {error}"));
            }
            drop((in_time, plain, late));
            drop(runtime);
        })
        .await
        .expect("the test task");
    });
}
