//! End-to-end proof that `Harness` runs a real agent turn as a library call.
//!
//! This is the acceptance test for the `openhuman_core::core::runtime`
//! composition (`CoreBuilder` + `DomainSet` + `ServiceSet::none()`): build with
//! no transport and no background services, run one turn, and assert nothing
//! was bound.
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

mod common;

use common::{chat_completion, offline_config, runtime};
use openhuman_embed::{Access, Harness, Provider, Workspace};
use serde_json::json;
use wiremock::matchers::{any, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const REPLY: &str = "harness-embed-ok";

/// How long the mock provider holds every reply.
///
/// This is the test's clock. 100 turns taken one after another cannot finish in
/// under `100 * PROVIDER_DELAY`, so any budget below that figure is what makes
/// serialization observable.
const PROVIDER_DELAY: std::time::Duration = std::time::Duration::from_secs(2);

/// Wall-clock allowance for the 100 overlapping turns below.
///
/// Overlapped, they cost one `PROVIDER_DELAY` plus per-agent construction and
/// session persistence — and it is that overhead, not the delay, that an
/// instrumented build inflates. A 250 ms delay used to put the serialization
/// floor at 25 s against a 20 s budget: 20% of headroom, which `cargo llvm-cov`
/// on a shared hosted runner spent, failing a run that had not serialized at
/// all (#6758). The floor is now 200 s, so this budget can absorb an
/// instrumented run and still catch a serialized one by a factor of two.
const CONCURRENT_BUDGET: std::time::Duration = std::time::Duration::from_secs(90);

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
                        .set_delay(PROVIDER_DELAY)
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
            // Regression: the harness agent's action directory must be the
            // resolved workspace's own action dir — `<root>/action`, a
            // sibling of `<root>/workspace` — not `agent::build`'s per-agent
            // default of `<root>/agents/harness/action`. The latter is right
            // for a multi-agent `Runtime::agent` caller narrowing its own
            // subdirectory, but a `Harness` (one runtime, one agent) must
            // keep writing to the directory `ResolvedWorkspace::resolve`
            // already created, or a caller reading `Workspace::Dir`'s sibling
            // `action/` directly would see nothing the agent ever wrote to.
            let root_dir = workspace_dir
                .parent()
                .expect("workspace_dir has a root parent");
            assert_eq!(
                harness.action_dir(),
                root_dir.join("action"),
                "harness action_dir must be the resolved workspace's own action dir"
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
            let requests = common::chat_requests(&provider_server).await;
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
            // `100 * PROVIDER_DELAY` before agent construction and persistence
            // overhead, which is well past `CONCURRENT_BUDGET`. All futures are
            // created together and each receives a distinct session, matching a
            // host such as OpenCompany running independent agents.
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
            let outcomes = tokio::time::timeout(CONCURRENT_BUDGET, async {
                let mut outcomes = Vec::with_capacity(100);
                while let Some(outcome) = turns.join_next().await {
                    outcomes.push(outcome.expect("concurrent turn task did not panic"));
                }
                outcomes
            })
            .await
            .unwrap_or_else(|_| {
                panic!("100 concurrent turns did not settle within {CONCURRENT_BUDGET:?}")
            });
            let elapsed = started.elapsed();
            eprintln!("100 concurrent library turns completed in {elapsed:?}");
            assert!(
                elapsed < CONCURRENT_BUDGET,
                "100 turns serialized instead of overlapping: {elapsed:?} is past \
                 the {CONCURRENT_BUDGET:?} budget"
            );
            let mut session_ids = std::collections::HashSet::new();
            for outcome in outcomes {
                let outcome = outcome.expect("concurrent turn runs");
                assert!(outcome.reply.contains(REPLY));
                assert!(session_ids.insert(outcome.session_id));
            }
            assert_eq!(session_ids.len(), 100);

            let requests = common::chat_requests(&provider_server).await;
            assert_eq!(requests.len(), 102, "two serial + 100 concurrent turns");

            // The first session's transcript landed in the harness agent's
            // own store under the workspace (the path-based transcript files
            // are gone). A harness session id is its thread id.
            let transcripts =
                tinyagents_session::port::WorkspaceStores::open(&workspace_dir, "harness")
                    .expect("the harness agent's workspace store")
                    .transcripts();
            assert!(
                transcripts.root_for_thread(&first.session_id).is_some(),
                "the first session's transcript was not persisted"
            );

            // The harness is one runtime plus one agent, and says so.
            assert_eq!(harness.agent().id(), "harness");
            assert_eq!(harness.runtime().workspace_dir(), harness.workspace_dir());
            assert_eq!(harness.runtime().agent_ids(), vec!["harness".to_string()]);

            // A second harness — a second runtime — in this process must be
            // refused rather than silently sharing process-global core state.
            let err = Harness::builder()
                .workspace(Workspace::Ephemeral)
                .build()
                .await
                .expect_err("a second harness must be refused");
            assert!(
                matches!(err, openhuman_embed::HarnessError::AlreadyRunning),
                "got {err:?}"
            );
            let err = openhuman_embed::Runtime::builder()
                .workspace(Workspace::Ephemeral)
                .build()
                .await
                .expect_err("a second runtime must be refused");
            assert!(
                matches!(err, openhuman_embed::RuntimeError::AlreadyRunning),
                "got {err:?}"
            );

            // Dropping the agent only schedules its MCP close, and the deferred
            // close keeps the ephemeral workspace until it is acknowledged;
            // removing the agent awaits the close first.
            harness
                .runtime()
                .remove_agent("harness")
                .await
                .expect("the harness agent is removed and its cleanup acknowledged");
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
