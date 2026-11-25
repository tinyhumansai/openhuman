//! `Agent::compact_thread` compacts a stored conversation with no model turn.
//!
//! A runtime over an [`InMemorySessionStores`] runs an agent against a
//! `wiremock` provider that answers chat turns with "ok" and the configured
//! summarizer model (`[context] summarizer_model`) with a fixed summary. The
//! test pins the whole contract on one thread:
//!
//! - below `min_tokens` nothing is written;
//! - above it the head generation is sealed (still readable) and the next
//!   generation opens with the summary right after the system prompt;
//! - the next turn resends the summary instead of the full history, with the
//!   static system prompt and tool declarations byte-identical and a memory
//!   block freshly rendered for the new generation;
//! - a turn started while the compaction holds the session waits for it.
//!
//! One `#[test]` because a runtime claims a process-wide slot.

mod common;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::{chat_completion, offline_config, runtime, stub_backend};
use openhuman_embed::memory::api::conformance::ReferenceEngine;
use openhuman_embed::memory::LearnParams;
use openhuman_embed::SessionStoreProvider as _;
use openhuman_embed::{
    Access, AgentDefinitionSpec, AgentSpec, CompactOptions, CompactOutcome, MemoryBinding,
    MemoryBlockSpec, Provider, Runtime, Workspace,
};
use tinyagents_session::port::InMemorySessionStores;
use tinyagents_session::transcript::SessionRef;
use wiremock::{Mock, Request, Respond, ResponseTemplate};

/// The model id the compaction summarizer calls.
const SUMMARIZER_MODEL: &str = "summarizer-model";
/// What the summarizer answers.
const SUMMARY: &str = "## Goal\nPlan the trip to Goa. SUMMARY-MARKER";
/// How the memory block opens (`MEMORY_BLOCK_MARKER`).
const MEMORY_MARKER: &str = "<memory>\n";
/// The memory tenant root the agent reads.
const ROOT: &str = "team:acme";
/// How long the summarizer takes to answer, so a turn can race it.
const SLOW_SUMMARY: Duration = Duration::from_millis(1500);

/// One chat request the provider received, and when.
#[derive(Clone)]
struct Seen {
    body: serde_json::Value,
    at: Instant,
}

/// Answers the summarizer slowly with [`SUMMARY`] and every chat turn with
/// "ok", recording each request.
#[derive(Clone, Default)]
struct ScriptedProvider(Arc<Mutex<Vec<Seen>>>);

impl ScriptedProvider {
    fn seen(&self) -> Vec<Seen> {
        self.0.lock().unwrap().clone()
    }

    fn turns(&self) -> Vec<Seen> {
        self.seen()
            .into_iter()
            .filter(|seen| !is_summary_request(&seen.body))
            .collect()
    }

    fn summaries(&self) -> Vec<Seen> {
        self.seen()
            .into_iter()
            .filter(|seen| is_summary_request(&seen.body))
            .collect()
    }
}

fn is_summary_request(body: &serde_json::Value) -> bool {
    body.get("model").and_then(|model| model.as_str()) == Some(SUMMARIZER_MODEL)
}

impl Respond for ScriptedProvider {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
        let summary = is_summary_request(&body);
        self.0.lock().unwrap().push(Seen {
            body,
            at: Instant::now(),
        });
        if summary {
            ResponseTemplate::new(200)
                .set_body_json(chat_completion(SUMMARY))
                .set_delay(SLOW_SUMMARY)
        } else {
            ResponseTemplate::new(200).set_body_json(chat_completion("ok"))
        }
    }
}

/// The messages a request sent.
fn messages(seen: &Seen) -> Vec<serde_json::Value> {
    seen.body["messages"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

/// The text of a request message, whether a string or content parts.
fn text(message: &serde_json::Value) -> String {
    match &message["content"] {
        serde_json::Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

#[test]
fn compact_thread_seals_the_head_and_the_next_turn_resends_the_summary() {
    let _ = env_logger::builder().is_test(true).try_init();
    runtime().block_on(async {
        tokio::spawn(async move {
            let backend = stub_backend().await;
            let server = wiremock::MockServer::start().await;
            let provider = ScriptedProvider::default();
            Mock::given(wiremock::matchers::method("POST"))
                .and(wiremock::matchers::path("/v1/chat/completions"))
                .respond_with(provider.clone())
                .mount(&server)
                .await;
            let store = Arc::new(InMemorySessionStores::new());
            let mut config = offline_config();
            config.modules.allow_download = false;
            config.context.compaction_strategy =
                openhuman_core::config::CompactionStrategy::Summary;
            config.context.summarizer_model = Some(SUMMARIZER_MODEL.into());
            let runtime = Runtime::builder()
                .config(config)
                .workspace(Workspace::stateless())
                .backend_url(backend.uri())
                .session_store(store.clone())
                .memory_engine(Arc::new(ReferenceEngine::new()))
                .build()
                .await
                .expect("the runtime builds");
            let memory = runtime.memory(ROOT).expect("the tenant's memory");
            let learn = |text: &'static str| {
                let memory = memory.clone();
                async move {
                    let params: LearnParams =
                        serde_json::from_value(serde_json::json!({ "text": text }))
                            .expect("learn params");
                    memory.learn(params).await.expect("a learning");
                }
            };
            learn("The user's favourite colour is teal").await;
            let agent = runtime
                .agent(
                    AgentSpec::new("asha")
                        .provider(
                            Provider::openai_compatible(format!("{}/v1", server.uri()), "sk-test")
                                .model("chat-model"),
                        )
                        .access(Access::readonly())
                        .definition(
                            AgentDefinitionSpec::new()
                                .memory_block(MemoryBlockSpec { max_tokens: 400 }),
                        )
                        .memory(MemoryBinding::new("asha").root(ROOT)),
                )
                .expect("asha instantiates");

            // ── a long conversation on one thread ─────────────────────────
            let first = agent
                .turn(format!(
                    "we are planning a trip to Goa. {}",
                    "detail ".repeat(300)
                ))
                .send()
                .await
                .expect("first turn");
            let thread = first.session_id.clone();
            for round in 0..5 {
                agent
                    .turn(format!("round {round}: {}", "more context ".repeat(300)))
                    .session(&thread)
                    .send()
                    .await
                    .expect("a turn");
            }
            let before = provider.turns().last().cloned().expect("a turn request");

            let transcripts = store.for_agent("asha").transcripts;
            let session = SessionRef::scoped(thread.clone(), "asha");
            let head_text = |session: &SessionRef| {
                let read = transcripts
                    .read_session_transcript(session)
                    .expect("the generation exists")
                    .read_session()
                    .expect("readable")
                    .expect("written");
                serde_json::to_string(&(read.messages, read.tools, read.meta.updated)).unwrap()
            };
            assert_eq!(transcripts.session_chain(&session).len(), 1);
            let original = head_text(&session);

            // ── below the threshold: nothing is written ───────────────────
            let skipped = agent
                .compact_thread(
                    &thread,
                    CompactOptions {
                        min_tokens: 10_000_000,
                        keep_recent_tokens: None,
                    },
                )
                .await
                .expect("a skipped compaction succeeds");
            let CompactOutcome::Skipped { tokens } = skipped else {
                panic!("expected Skipped, got {skipped:?}");
            };
            assert!(tokens > 1_000, "the thread's tokens are reported: {tokens}");
            assert!(provider.summaries().is_empty(), "no summary was bought");
            assert_eq!(transcripts.session_chain(&session).len(), 1);
            assert_eq!(head_text(&session), original, "no transcript write");

            // ── an unknown thread is empty ────────────────────────────────
            let empty = agent
                .compact_thread("never-used", CompactOptions::default())
                .await
                .expect("an empty thread compacts to nothing");
            assert_eq!(empty, CompactOutcome::Empty);

            // ── memory changes while the user is away ─────────────────────
            learn("The user lives in Lisbon").await;

            // ── above it, racing a turn ───────────────────────────────────
            let compacting = {
                let agent = agent.clone();
                let thread = thread.clone();
                tokio::spawn(async move {
                    agent
                        .compact_thread(
                            &thread,
                            CompactOptions {
                                min_tokens: 500,
                                keep_recent_tokens: Some(200),
                            },
                        )
                        .await
                })
            };
            let summary_asked = common::eventually("the summary request", || {
                provider.summaries().first().map(|seen| seen.at)
            })
            .await;
            let turn = agent.turn("so, what was the plan?").session(&thread).send();
            let (outcome, reply) = tokio::join!(compacting, turn);
            let outcome = outcome
                .expect("the task ran")
                .expect("the compaction succeeds");
            reply.expect("the racing turn");
            let CompactOutcome::Compacted {
                tokens_before,
                tokens_after,
            } = outcome
            else {
                panic!("expected Compacted, got {outcome:?}");
            };
            assert!(
                tokens_after < tokens_before,
                "{tokens_after} < {tokens_before}"
            );

            // The summary request went to the summarizer model alone.
            assert_eq!(provider.summaries().len(), 1);

            // The turn waited for the compaction: its request came after the
            // summary was answered, and it resent the summary.
            let after = provider
                .turns()
                .last()
                .cloned()
                .expect("the racing turn's request");
            assert!(
                after.at >= summary_asked + SLOW_SUMMARY,
                "the turn ran after the compaction released the session"
            );
            let before_messages = messages(&before);
            let after_messages = messages(&after);
            let system_prompt = |messages: &[serde_json::Value]| -> Vec<serde_json::Value> {
                messages
                    .iter()
                    .take_while(|message| message["role"] == "system")
                    .cloned()
                    .collect()
            };
            let is_block = |message: &serde_json::Value| text(message).starts_with(MEMORY_MARKER);
            let static_prompt = |prompt: &[serde_json::Value]| -> Vec<serde_json::Value> {
                prompt
                    .iter()
                    .filter(|message| !is_block(message))
                    .cloned()
                    .collect()
            };
            let prompt = system_prompt(&after_messages);
            let earlier = system_prompt(&before_messages);
            assert!(
                !prompt.is_empty(),
                "the request opens with the system prompt"
            );
            assert_eq!(
                static_prompt(&prompt),
                static_prompt(&earlier),
                "the static system prompt is byte-identical"
            );
            let old_block = earlier.last().map(text).unwrap_or_default();
            assert!(
                old_block.starts_with(MEMORY_MARKER) && !old_block.contains("Lisbon"),
                "the thread opened with the block of its time: {old_block}"
            );
            let new_block = prompt.last().map(text).unwrap_or_default();
            assert!(
                new_block.starts_with(MEMORY_MARKER) && new_block.contains("Lisbon"),
                "the new generation carries a fresh memory block: {new_block}"
            );
            assert_eq!(
                after.body.get("tools"),
                before.body.get("tools"),
                "the tool declarations are byte-identical"
            );
            let summary = &after_messages[prompt.len()];
            assert_eq!(
                summary["role"], "user",
                "the summary follows the system prompt"
            );
            assert!(text(summary).contains("SUMMARY-MARKER"));
            assert!(
                after.body.to_string().len() < before.body.to_string().len(),
                "the compacted request is smaller"
            );
            assert!(
                !after.body.to_string().contains("we are planning a trip"),
                "the folded opening turn is not resent"
            );

            // ── the sealed generation is still readable ───────────────────
            let chain = transcripts.session_chain(&session);
            assert_eq!(chain.len(), 2, "the head was sealed and the next opened");
            assert_eq!(
                head_text(&chain[0]),
                original,
                "the sealed one is untouched"
            );

            runtime.remove_agent("asha").await.expect("asha is removed");
            drop(agent);
            drop(runtime);
        })
        .await
        .expect("the test task");
    });
}
