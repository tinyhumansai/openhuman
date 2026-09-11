//! End-to-end proof that `Harness` runs a real agent turn as a library call.
//!
//! This is the acceptance test `docs/plans/pluggable-core/phase-1-corebuilder.md`
//! specified and never got: build with no transport and no background services,
//! run one turn, and assert nothing was bound.
//!
//! # Why one test does all of it
//!
//! A `Harness` claims a process-wide slot, because the core's keyring, event bus
//! and `Once`-guarded subscribers are process-scoped. Splitting these assertions
//! into separate `#[test]` functions would either serialize them behind a mutex
//! (same thing, more code) or race. So the process builds exactly one harness
//! and checks everything against it.
//!
//! No live LLM call is made: `wiremock` stands in for the provider, which is
//! also what makes the routing assertion possible — if the turn had gone
//! anywhere else, the mock would have recorded no request.

use openhuman_core::core::runtime::{AGENT_WORKER_STACK_BYTES, MAX_BLOCKING_THREADS};
use openhuman_core::openhuman::config::Config;
use openhuman_core::{Access, Harness, Provider, Workspace};
use serde_json::json;
use wiremock::matchers::{any, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const REPLY: &str = "harness-embed-ok";

/// An OpenAI-compatible chat completion carrying `content`.
fn chat_completion(content: &str) -> serde_json::Value {
    json!({
        "id": "chatcmpl-harness-embed",
        "object": "chat.completion",
        "created": 1_700_000_000_u64,
        "model": "harness-embed-model",
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": content },
            "finish_reason": "stop"
        }],
        "usage": { "prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2 }
    })
}

/// A config that keeps the turn offline: no local runtimes, no spaCy, no
/// embeddings endpoint. Mirrors `src/bin/library_profile/harness.rs::fixture()`,
/// which is the recipe already proven against real turns.
fn offline_config() -> Config {
    let mut config = Config::default();
    config.local_ai.runtime_enabled = false;
    config.runtime_python.enabled = false;
    config.memory_tree.spacy_enabled = false;
    config.memory_tree.embedding_endpoint = None;
    config.memory_tree.embedding_model = None;
    config.memory_tree.embedding_strict = false;
    config.default_temperature = 0.0;
    config
}

/// The tuned runtime the harness documents as the caller's responsibility.
///
/// A default 2 MiB worker stack overflows on a turn that delegates to a
/// sub-agent and aborts the whole process, so building it the documented way is
/// both what the test needs and a check that the documented way works.
fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(AGENT_WORKER_STACK_BYTES)
        .max_blocking_threads(MAX_BLOCKING_THREADS)
        .build()
        .expect("tokio runtime")
}

#[test]
fn a_harness_runs_a_turn_against_the_provider_it_was_given() {
    let _ = env_logger::builder().is_test(true).try_init();

    let runtime = runtime();
    runtime.block_on(async {
        // `Runtime::block_on` polls its root future on this test thread, whose
        // default stack is much smaller than the tuned worker stacks. Put the
        // agent host itself on a worker so the documented stack setting
        // actually applies to the large turn futures.
        tokio::spawn(async move {
            // A stub backend keeps incidental non-inference calls local. The
            // caller-supplied provider itself requires no app login in library
            // mode.
            let backend = MockServer::start().await;
            Mock::given(any())
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "success": true,
                    "data": { "id": "harness-embed-test", "email": "local@openhuman.local" }
                })))
                .mount(&backend)
                .await;

            let provider_server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/v1/chat/completions"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_delay(std::time::Duration::from_millis(250))
                        .set_body_json(chat_completion(REPLY)),
                )
                .mount(&provider_server)
                .await;

            let harness = std::sync::Arc::new(
                Harness::builder()
                    .config(offline_config())
                    .workspace(Workspace::Ephemeral)
                    .backend_url(backend.uri())
                    .provider(
                        Provider::openai_compatible(
                            format!("{}/v1", provider_server.uri()),
                            "sk-test",
                        )
                        .model("harness-embed-model"),
                    )
                    // Read-only: the turn has no business acting, and this keeps the
                    // test from depending on the approval gate's timing.
                    .access(Access::readonly())
                    .build()
                    .await
                    .expect("harness builds"),
            );

            // The workspace is the harness's own, not the operator's.
            let workspace_dir = harness.workspace_dir().to_path_buf();
            assert!(workspace_dir.is_dir(), "workspace was not created");
            assert!(
                !harness.action_dir().starts_with(&workspace_dir),
                "action_dir must not sit inside the workspace, or every agent write \
             is blocked by is_workspace_internal_path"
            );

            // No listener was bound: `ServiceSet` selects nothing that binds, and
            // `serve()` was never called.
            assert!(
                std::env::var("OPENHUMAN_CORE_RPC_URL").is_err(),
                "a library harness must not bind an RPC listener"
            );

            let first = harness.run("Say the magic word.").await.expect("turn runs");
            assert!(
                first.reply.contains(REPLY),
                "reply {:?} does not carry the provider's response",
                first.reply
            );
            assert!(
                !first.session_id.is_empty(),
                "the harness must mint a session id — the core returns none, so \
             without this a caller cannot continue a conversation at all"
            );

            // The turn went to the endpoint we named, not to the account's route.
            let requests = provider_server
                .received_requests()
                .await
                .expect("mock recorded requests");
            assert!(
                !requests.is_empty(),
                "the provider endpoint received nothing — the per-call route was ignored"
            );

            // Continuing a conversation reuses the caller's session id verbatim.
            let second = harness
                .turn("And again.")
                .session(&first.session_id)
                .send()
                .await
                .expect("second turn runs");
            assert_eq!(second.session_id, first.session_id);

            // One core must support many live agents. The delayed provider makes
            // serialization observable: sequential execution would take at least
            // 25 seconds before agent construction and persistence overhead. All
            // futures are created together and each receives a distinct session,
            // matching a host such as OpenCompany running independent agents.
            let started = std::time::Instant::now();
            let mut turns = tokio::task::JoinSet::new();
            for index in 0..100 {
                let harness = std::sync::Arc::clone(&harness);
                turns.spawn(async move {
                    harness
                        .turn(format!("Concurrent agent {index}"))
                        .session(format!("concurrent-agent-{index}"))
                        .send()
                        .await
                });
            }
            let outcomes = tokio::time::timeout(std::time::Duration::from_secs(20), async {
                let mut outcomes = Vec::with_capacity(100);
                while let Some(outcome) = turns.join_next().await {
                    outcomes.push(outcome.expect("concurrent turn task did not panic"));
                }
                outcomes
            })
            .await
            .expect("100 concurrent turns did not settle within 20 seconds");
            let elapsed = started.elapsed();
            eprintln!("100 concurrent library turns completed in {elapsed:?}");
            assert!(
                elapsed < std::time::Duration::from_secs(20),
                "100 turns serialized instead of overlapping: {elapsed:?}"
            );
            let mut session_ids = std::collections::HashSet::new();
            for outcome in outcomes {
                let outcome = outcome.expect("concurrent turn runs");
                assert!(outcome.reply.contains(REPLY));
                assert!(session_ids.insert(outcome.session_id));
            }
            assert_eq!(session_ids.len(), 100);

            let requests = provider_server
                .received_requests()
                .await
                .expect("mock recorded concurrent requests");
            assert_eq!(requests.len(), 102, "two serial + 100 concurrent turns");

            // The session database landed in the harness's workspace.
            assert!(
                workspace_dir.join("session_db/sessions.db").exists(),
                "sessions were not persisted under the harness workspace"
            );

            // A second harness in this process must be refused rather than silently
            // sharing process-global core state with the first.
            let err = Harness::builder()
                .workspace(Workspace::Ephemeral)
                .build()
                .await
                .expect_err("a second harness must be refused");
            assert!(
                matches!(err, openhuman_core::HarnessError::AlreadyRunning),
                "got {err:?}"
            );

            drop(harness);
            assert!(
                !workspace_dir.exists(),
                "an ephemeral workspace must be removed with its harness"
            );
        })
        .await
        .expect("library host task did not panic");
    });
}
