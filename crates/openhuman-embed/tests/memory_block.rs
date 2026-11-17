//! The memory block (`AgentDefinitionSpec::memory_block`), end to end: real
//! turns on a stateless runtime over TinyMemory's reference engine.
//!
//! - A new thread's first request ends its system prompt with the block.
//! - Memory that changes between two ordinary turns leaves the second
//!   request's system prompt byte-identical, with no instructions note.
//! - A replaced prompt (`refresh_prompt_on_resume`) starts a generation: the
//!   block is rendered again from current memory, and the note still opens
//!   the user's message.
//! - Nothing remembered sends no block, and a memory read error sends none
//!   while the turn still completes.
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
    Access, AgentDefinitionSpec, AgentSpec, MemoryBinding, MemoryBlockSpec, Provider, Runtime,
    Workspace,
};
use tinyagents_session::port::InMemorySessionStores;

/// How the block opens (`MEMORY_BLOCK_MARKER`).
const MARKER: &str = "<memory>\n";
/// How a replaced prompt's note opens (`PROMPT_UPDATE_NOTE`).
const NOTE: &str = "[instructions update]";
const ROOT: &str = "team:acme";
const TURN_BUDGET: Duration = Duration::from_secs(120);

/// The reference engine, whose reads fail on demand.
struct FailableEngine {
    inner: ReferenceEngine,
    fail_reads: AtomicBool,
}

impl FailableEngine {
    fn read_gate(&self) -> memory_api::Result<()> {
        if self.fail_reads.load(Ordering::SeqCst) {
            return Err(memory_api::Error::Unavailable("the engine is down".into()));
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl MemoryEngine for FailableEngine {
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
        self.read_gate()?;
        self.inner.recall(req).await
    }

    async fn fetch(
        &self,
        req: memory_api::FetchRequest,
    ) -> memory_api::Result<memory_api::FetchPage> {
        self.read_gate()?;
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
        self.read_gate()?;
        self.inner.list(req).await
    }

    async fn get(&self, req: memory_api::GetRequest) -> memory_api::Result<Vec<memory_api::Hit>> {
        self.read_gate()?;
        self.inner.get(req).await
    }
}

/// `(role, content)` of every message in a recorded chat request.
fn messages(request: &wiremock::Request) -> Vec<(String, String)> {
    let body: serde_json::Value = serde_json::from_slice(&request.body).expect("a JSON request");
    body["messages"]
        .as_array()
        .expect("a messages array")
        .iter()
        .map(|message| {
            let content = match &message["content"] {
                serde_json::Value::String(text) => text.clone(),
                other => other.to_string(),
            };
            (message["role"].as_str().unwrap_or("").to_string(), content)
        })
        .collect()
}

/// The system messages of a recorded chat request, in order.
fn system_prompt(request: &wiremock::Request) -> Vec<String> {
    messages(request)
        .into_iter()
        .filter(|(role, _)| role == "system")
        .map(|(_, content)| content)
        .collect()
}

/// The last user message of a recorded chat request.
fn last_user(request: &wiremock::Request) -> String {
    messages(request)
        .into_iter()
        .rev()
        .find(|(role, _)| role == "user")
        .map(|(_, content)| content)
        .unwrap_or_default()
}

fn definition(prompt: &str) -> AgentDefinitionSpec {
    AgentDefinitionSpec::new()
        .bare_prompt(prompt)
        .refresh_prompt_on_resume(true)
        .memory_block(MemoryBlockSpec { max_tokens: 400 })
}

#[test]
fn the_memory_block_is_rendered_when_a_generation_starts_and_kept_otherwise() {
    let _ = env_logger::builder().is_test(true).try_init();
    runtime().block_on(async {
        tokio::spawn(async move {
            let engine = Arc::new(FailableEngine {
                inner: ReferenceEngine::new(),
                fail_reads: AtomicBool::new(false),
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
            let memory = runtime.memory(ROOT).expect("the tenant's memory");
            let learn = |text: &'static str| {
                let memory = memory.clone();
                async move {
                    let params: LearnParams = serde_json::from_value(serde_json::json!({
                        "text": text
                    }))
                    .expect("learn params");
                    memory.learn(params).await.expect("a learning");
                }
            };
            let agent_on = |id: &str, model: &wiremock::MockServer, prompt: &str| {
                runtime
                    .agent(
                        AgentSpec::new(id)
                            .provider(
                                Provider::openai_compatible(
                                    format!("{}/v1", model.uri()),
                                    "sk-test",
                                )
                                .model("block-model"),
                            )
                            .access(Access::readonly())
                            .definition(definition(prompt))
                            .memory(MemoryBinding::new(id).root(ROOT)),
                    )
                    .expect("the agent instantiates")
            };
            let ask = |agent: openhuman_embed::Agent, session: Option<String>| async move {
                let turn = agent.turn("hello");
                let turn = match session {
                    Some(session) => turn.session(session),
                    None => turn,
                };
                tokio::time::timeout(TURN_BUDGET, turn.send())
                    .await
                    .expect("the turn finishes")
                    .expect("the turn succeeds")
            };

            // ── nothing remembered yet: no block ───────────────────────────
            let empty_model = provider("ok").await;
            let empty = agent_on("empty", &empty_model, "You are blocky.");
            ask(empty.clone(), None).await;
            let requests = chat_requests(&empty_model).await;
            let prompt = system_prompt(requests.first().expect("a model request"));
            assert!(
                prompt.iter().all(|message| !message.starts_with(MARKER)),
                "nothing remembered sends no block: {prompt:#?}"
            );

            // ── a new thread: the block ends the system prompt ─────────────
            learn("The user's favourite colour is teal").await;
            let model = provider("ok").await;
            let blocky = agent_on("blocky", &model, "You are blocky.");
            let first = ask(blocky.clone(), None).await;
            let requests = chat_requests(&model).await;
            let opening = system_prompt(requests.first().expect("a model request"));
            let block = opening.last().expect("a system prompt");
            assert!(
                block.starts_with(MARKER) && block.contains("teal"),
                "the block is the prompt's tail: {opening:#?}"
            );
            assert!(
                opening[0].contains("You are blocky."),
                "the static prompt comes first: {opening:#?}"
            );

            // ── memory changes; an ordinary turn keeps the block verbatim ──
            learn("The user lives in Lisbon").await;
            ask(blocky.clone(), Some(first.session_id.clone())).await;
            let requests = chat_requests(&model).await;
            let second = requests.last().expect("a second request");
            assert_eq!(
                system_prompt(second),
                opening,
                "the system prompt is byte-identical"
            );
            assert!(
                !last_user(second).contains(NOTE),
                "a memory change adds no note"
            );

            // ── a replaced prompt starts a generation: a fresh block ───────
            runtime.remove_agent("blocky").await.expect("removed");
            drop(blocky);
            let refreshed_model = provider("ok").await;
            let blocky = agent_on("blocky", &refreshed_model, "You are blocky, v2.");
            ask(blocky.clone(), Some(first.session_id.clone())).await;
            let requests = chat_requests(&refreshed_model).await;
            let third = requests.first().expect("a third request");
            let prompt = system_prompt(third);
            assert!(prompt[0].contains("v2"), "the new prompt: {prompt:#?}");
            let block = prompt.last().expect("a system prompt");
            assert!(
                block.starts_with(MARKER) && block.contains("Lisbon"),
                "the new generation's block reads current memory: {prompt:#?}"
            );
            assert!(
                last_user(third).contains(NOTE),
                "a replaced prompt still says so"
            );

            // ── a read error: no block, and the turn completes ────────────
            engine.fail_reads.store(true, Ordering::SeqCst);
            let failing_model = provider("still here").await;
            let failing = agent_on("failing", &failing_model, "You are blocky.");
            let reply = ask(failing.clone(), None).await;
            assert_eq!(reply.reply, "still here");
            let requests = chat_requests(&failing_model).await;
            let prompt = system_prompt(requests.first().expect("a model request"));
            assert!(
                prompt.iter().all(|message| !message.starts_with(MARKER)),
                "a failed read sends no block: {prompt:#?}"
            );

            for agent in ["empty", "blocky", "failing"] {
                runtime
                    .remove_agent(agent)
                    .await
                    .unwrap_or_else(|error| panic!("{agent} is removed: {error}"));
            }
            drop((empty, blocky, failing));
            drop(runtime);
        })
        .await
        .expect("the test task");
    });
}
