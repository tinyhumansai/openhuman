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
//! No live LLM call is made: a local mock stands in for the provider, which is
//! also what makes the routing assertion possible — if the turn had gone
//! anywhere else, the mock would have recorded no request.

mod common;

use common::gated_provider::GatedProvider;
use common::{offline_config, runtime};
use openhuman_embed::{Access, Harness, Provider, SessionStoreProvider, Workspace};
use serde_json::json;
use tinyagents_session::InMemorySessionStores;
use wiremock::matchers::any;
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A runtime whose drop waits at most `RUNTIME_SHUTDOWN` for blocking work.
struct BoundedRuntime(Option<tokio::runtime::Runtime>);

impl std::ops::Deref for BoundedRuntime {
    type Target = tokio::runtime::Runtime;
    fn deref(&self) -> &Self::Target {
        self.0.as_ref().expect("runtime is alive until drop")
    }
}

impl Drop for BoundedRuntime {
    fn drop(&mut self) {
        if let Some(runtime) = self.0.take() {
            runtime.shutdown_timeout(RUNTIME_SHUTDOWN);
        }
    }
}

const REPLY: &str = "harness-embed-ok";

/// How many independent turns the overlap proof runs at once.
const CONCURRENT_TURNS: usize = 100;

/// Upper bound for any single step that has no clock of its own (building the
/// harness, one turn, removing the agent).
///
/// None of them has a deadline in the library, so a stuck one used to leave
/// the test parked until the runner's 10-minute kill, with no output at all
/// (EX63 run 39292268284). Bounding each makes a hang fail in minutes and name
/// the step.
const STEP_BUDGET: std::time::Duration = std::time::Duration::from_secs(180);

/// How long dropping the runtime may wait for in-flight blocking work.
///
/// `Runtime::drop` joins every `spawn_blocking` task without a limit. After a
/// failed assertion, 100 agents' unfinished work then holds the panic back for
/// as long as the slowest of them runs, which is how a missed wall-clock budget
/// once turned into a silent timeout.
const RUNTIME_SHUTDOWN: std::time::Duration = std::time::Duration::from_secs(10);

/// Panics with `message`, printing it first.
///
/// The linked tinyflows module installs TinyBus's process-wide panic hook,
/// which writes only `module panicked at <file>:<line>` and drops the payload,
/// for the host's panics too. Without this line a failure in CI names a line
/// number and nothing else.
fn fail(message: String) -> ! {
    eprintln!("harness_embed failure: {message}");
    panic!("{message}")
}

/// Runs `step` for at most `STEP_BUDGET`, failing with its `name` otherwise.
async fn bounded<T>(name: &str, step: impl std::future::IntoFuture<Output = T>) -> T {
    tokio::time::timeout(STEP_BUDGET, step.into_future())
        .await
        .unwrap_or_else(|_| fail(format!("{name} did not finish within {STEP_BUDGET:?}")))
}

/// Deadlock guard for the overlapping turns below. Not a speed budget.
///
/// Overlap is proven structurally: the provider holds every reply until all
/// `CONCURRENT_TURNS` requests are in flight together, which turns run one
/// after another can never reach. A wall-clock budget used to stand in for
/// that proof, and an instrumented build on a loaded 4-vCPU runner kept
/// spending it on agent construction alone (#6758, EX63 run 39357699152), so a
/// run that had not serialized failed anyway. This bound only stops a
/// serialized (or capped) host from waiting at the gate until the runner's
/// 10-minute kill; a correct run opens the gate as soon as its last turn
/// reaches the provider, however slow the machine.
const CONCURRENT_HANG_GUARD: std::time::Duration = std::time::Duration::from_secs(300);

#[test]
fn a_harness_runs_a_turn_against_the_provider_it_was_given() {
    let _ = env_logger::builder().is_test(true).try_init();

    let runtime = BoundedRuntime(Some(runtime()));
    // Every turn opens the MCP client, which asks TinyBox for the jail's
    // toolchain grants. Where the module is not already staged (a local run,
    // unlike the lanes' `OPENHUMAN_MODULE_PATH`) that downloads it from GitHub
    // with no timeout, so a test that is meant to be offline waited on the
    // network (24 s here) and could wait forever behind a firewall.
    let mut config = offline_config();
    config.modules.allow_download = false;
    runtime.block_on(async move {
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

            // Answers at once until the overlap proof arms its gate.
            let provider_server = GatedProvider::start(REPLY).await;
            let session_store = std::sync::Arc::new(InMemorySessionStores::new());

            let harness = std::sync::Arc::new(
                bounded(
                    "building the harness",
                    Harness::builder()
                        .config(config)
                        .workspace(Workspace::Stateless)
                        .backend_url(backend.uri())
                        .session_store(session_store.clone())
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
                        .build(),
                )
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

            let first = bounded("the first turn", harness.run("Say the magic word."))
                .await
                .expect("turn runs");
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
            assert!(
                provider_server.chat_requests() > 0,
                "the provider endpoint received nothing — the per-call route was ignored"
            );

            // Continuing a conversation reuses the caller's session id verbatim.
            let second = bounded(
                "the second turn",
                harness.turn("And again.").session(&first.session_id).send(),
            )
            .await
            .expect("second turn runs");
            assert_eq!(second.session_id, first.session_id);

            // One core must support many live agents. The gate makes overlap
            // observable without a clock: no reply leaves the provider until
            // all `CONCURRENT_TURNS` requests are in flight at once. Turns that
            // ran one after another would leave one request waiting forever;
            // a host capping concurrency below that would stall at its cap. All
            // futures are created together and each receives a distinct
            // session, matching a host such as OpenCompany running independent
            // agents.
            provider_server.hold_until(CONCURRENT_TURNS);
            let started = std::time::Instant::now();
            let mut turns = tokio::task::JoinSet::new();
            let completed = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
            for index in 0..CONCURRENT_TURNS {
                let harness = std::sync::Arc::clone(&harness);
                let completed = std::sync::Arc::clone(&completed);
                turns.spawn(async move {
                    let result = harness
                        .turn(format!("Concurrent agent {index}"))
                        .session(format!("concurrent-agent-{index}"))
                        .send()
                        .await;
                    completed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    (index, result)
                });
            }
            let outcomes = tokio::time::timeout(CONCURRENT_HANG_GUARD, async {
                let mut outcomes = Vec::with_capacity(CONCURRENT_TURNS);
                let mut report = tokio::time::interval(std::time::Duration::from_secs(10));
                report.tick().await;
                loop {
                    tokio::select! {
                        joined = turns.join_next() => match joined {
                            Some(Ok((index, Ok(outcome)))) => outcomes.push((index, outcome)),
                            Some(Ok((index, Err(error)))) => fail(format!(
                                "concurrent turn {index} failed after {:?}: {error:?}; \
                                 completed={}, provider requests={}, in_flight={}, peak={}",
                                started.elapsed(),
                                completed.load(std::sync::atomic::Ordering::Relaxed),
                                provider_server.chat_requests(),
                                provider_server.in_flight(),
                                provider_server.peak_in_flight(),
                            )),
                            Some(Err(error)) => fail(format!(
                                "concurrent turn task failed after {:?}: {error}; \
                                 completed={}, provider requests={}, in_flight={}, peak={}",
                                started.elapsed(),
                                completed.load(std::sync::atomic::Ordering::Relaxed),
                                provider_server.chat_requests(),
                                provider_server.in_flight(),
                                provider_server.peak_in_flight(),
                            )),
                            None => break,
                        },
                        _ = report.tick() => eprintln!(
                            "harness_embed concurrent progress: elapsed={:?}, completed={}, \
                             provider requests={}, in_flight={}, peak={}",
                            started.elapsed(),
                            completed.load(std::sync::atomic::Ordering::Relaxed),
                            provider_server.chat_requests(),
                            provider_server.in_flight(),
                            provider_server.peak_in_flight(),
                        ),
                    }
                }
                outcomes
            })
            .await
            .unwrap_or_else(|_| {
                fail(format!(
                    "{CONCURRENT_TURNS} turns did not overlap: the most ever in flight \
                     at the provider together was {} ({} waiting at the gate after \
                     {CONCURRENT_HANG_GUARD:?}); the gate opens only when all \
                     {CONCURRENT_TURNS} are, so turns serialized or are capped",
                    provider_server.peak_in_flight(),
                    provider_server.in_flight(),
                ))
            });
            // Informational only; nothing asserts on it.
            eprintln!(
                "{CONCURRENT_TURNS} concurrent library turns completed in {:?} \
                 (peak {} in flight)",
                started.elapsed(),
                provider_server.peak_in_flight()
            );
            assert!(provider_server.is_open(), "the overlap gate never opened");
            assert!(
                provider_server.peak_in_flight() >= CONCURRENT_TURNS,
                "only {} turns were in flight together, not {CONCURRENT_TURNS}",
                provider_server.peak_in_flight()
            );
            let mut session_ids = std::collections::HashSet::new();
            for (index, outcome) in outcomes {
                assert_eq!(outcome.session_id, format!("concurrent-agent-{index}"));
                assert!(outcome.reply.contains(REPLY));
                assert!(session_ids.insert(outcome.session_id));
            }
            assert_eq!(session_ids.len(), CONCURRENT_TURNS);

            assert_eq!(
                provider_server.chat_requests(),
                2 + CONCURRENT_TURNS,
                "two serial + {CONCURRENT_TURNS} concurrent turns"
            );

            // The first session's transcript landed in the harness agent's
            // own store under the workspace (the path-based transcript files
            // are gone). A harness session id is its thread id.
            let transcripts = session_store.for_agent("harness").transcripts;
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
            bounded(
                "removing the harness agent",
                harness.runtime().remove_agent("harness"),
            )
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
