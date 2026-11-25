//! The steps a turn runs before its first model request are reported to the
//! host observer as `TurnObservation::Setup`, in the order they ran, so a
//! host can split its setup time into what each step cost.
//!
//! One `#[test]` because a runtime claims a process-wide slot, and the host
//! engine it installs is process-wide too.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{offline_config, provider, runtime, stub_backend};
use openhuman_embed::memory::api::conformance::ReferenceEngine;
use openhuman_embed::memory::LearnParams;
use openhuman_embed::observe::{observe_turn, SetupPhase, TraceContent, TurnObservation};
use openhuman_embed::{
    Access, AgentDefinitionSpec, AgentSpec, MemoryBinding, MemoryBlockSpec, Provider, Runtime,
    Workspace,
};
use tinyagents_session::port::InMemorySessionStores;

const ROOT: &str = "team:acme";
const TURN_BUDGET: Duration = Duration::from_secs(120);

#[derive(Default)]
struct Records {
    events: Mutex<Vec<TurnObservation>>,
}

impl openhuman_embed::observe::TurnObserver for Records {
    fn on_event(&self, event: &TurnObservation) {
        self.events.lock().unwrap().push(event.clone());
    }
    fn on_turn(&self, _: &openhuman_embed::observe::TurnTrace<'_>) {}
}

/// What each observation was, in order: a setup phase by its stable name, or
/// `model` for a model call. Tool and dispatch events are not under test.
fn timeline(records: &Records) -> Vec<(&'static str, bool, bool)> {
    records
        .events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|event| match event {
            TurnObservation::Setup {
                phase,
                failed,
                skipped,
                ..
            } => Some((phase.as_str(), *failed, *skipped)),
            TurnObservation::Model { .. } => Some(("model", false, false)),
            _ => None,
        })
        .collect()
}

#[test]
fn setup_phases_are_reported_in_order() {
    runtime().block_on(async {
        tokio::spawn(async move {
            let backend = stub_backend().await;
            let mut config = offline_config();
            config.modules.allow_download = false;
            let runtime = Runtime::builder()
                .config(config)
                .workspace(Workspace::stateless())
                .backend_url(backend.uri())
                .session_store(Arc::new(InMemorySessionStores::new()))
                .memory_engine(Arc::new(ReferenceEngine::new()))
                .build()
                .await
                .expect("a stateless runtime over the reference engine");
            let params: LearnParams =
                serde_json::from_value(serde_json::json!({"text": "A fact worth a block"}))
                    .expect("learn params");
            runtime
                .memory(ROOT)
                .expect("the tenant's memory")
                .learn(params)
                .await
                .expect("a learning");

            let model = provider("ok").await;
            let agent = runtime
                .agent(
                    AgentSpec::new("setup-reporter")
                        .provider(
                            Provider::openai_compatible(format!("{}/v1", model.uri()), "sk-test")
                                .model("setup-model"),
                        )
                        .access(Access::readonly())
                        .definition(
                            AgentDefinitionSpec::new()
                                .bare_prompt("You report setup.")
                                .refresh_prompt_on_resume(true)
                                .memory_block(MemoryBlockSpec { max_tokens: 400 }),
                        )
                        .memory(MemoryBinding::new("setup-reporter").root(ROOT)),
                )
                .expect("the agent instantiates");

            // A cold turn on a new thread opens the agent, resumes nothing,
            // renders the prompt and then the memory block, all before the
            // model is called.
            let cold = Arc::new(Records::default());
            tokio::time::timeout(
                TURN_BUDGET,
                observe_turn(
                    cold.clone(),
                    TraceContent::MetadataOnly,
                    "setup-thread",
                    "hello",
                    agent.turn("hello").session("setup-thread").send(),
                ),
            )
            .await
            .expect("the turn finishes")
            .expect("the turn succeeds");
            assert_eq!(
                timeline(&cold),
                vec![
                    ("agent_open", false, false),
                    ("transcript_resume", false, false),
                    ("prompt_refresh", false, false),
                    ("memory_block", false, false),
                    ("model", false, false),
                ]
            );
            assert!(
                cold.events.lock().unwrap().iter().any(|event| matches!(
                    event,
                    TurnObservation::Setup {
                        phase: SetupPhase::MemoryBlock,
                        ..
                    }
                )),
                "hosts match on the phase enum the facade re-exports"
            );

            runtime
                .remove_agent("setup-reporter")
                .await
                .expect("removed");
            drop(agent);
            drop(runtime);
        })
        .await
        .expect("the test task");
    });
}
