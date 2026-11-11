//! Isolation and lifecycle checks for custom inference and runtime observation.
mod common;
use async_trait::async_trait;
use openhuman_embed::providers::{ChatModel, ModelRequest, ModelResponse};
use openhuman_embed::seams::{LearningSink, TurnSummary};
use openhuman_embed::{
    Access, AgentDefinitionSpec, AgentSpec, ModelDefaults, Provider, Runtime, RuntimeEventKind,
    SessionStoreProvider, StreamEvent,
};
use std::sync::Arc;
use tinyagents_session::port::InMemorySessionStores;

struct Model {
    label: &'static str,
    seen: tokio::sync::mpsc::UnboundedSender<ModelRequest>,
}
#[async_trait]
impl ChatModel<()> for Model {
    async fn invoke(
        &self,
        _: &(),
        request: ModelRequest,
    ) -> openhuman_embed::providers::Result<ModelResponse> {
        self.seen.send(request).unwrap();
        Ok(ModelResponse::assistant(self.label))
    }
}
struct Hook {
    seen: tokio::sync::mpsc::UnboundedSender<String>,
}
#[async_trait]
impl LearningSink for Hook {
    async fn on_turn_complete(&self, ctx: &TurnSummary) -> tinyagents_harness::error::Result<()> {
        self.seen.send(ctx.agent_id.clone()).unwrap();
        Ok(())
    }
}

#[test]
fn custom_models_local_hooks_session_stores_and_events_stay_scoped() {
    common::runtime().block_on(async {
        tokio::spawn(async {
            let backend = common::stub_backend().await;
            let (model_tx, mut model_rx) = tokio::sync::mpsc::unbounded_channel();
            let (hook_tx, mut hook_rx) = tokio::sync::mpsc::unbounded_channel();
            let (global_tx, mut global_rx) = tokio::sync::mpsc::unbounded_channel();
            let runtime = Runtime::builder()
                .config(common::offline_config())
                .backend_url(backend.uri())
                .access(Access::readonly())
                .provider(
                    Provider::custom(Arc::new(Model {
                        label: "answer-a",
                        seen: model_tx.clone(),
                    }))
                    .model("custom-a"),
                )
                .model_defaults(ModelDefaults {
                    max_tokens: Some(40),
                    top_p: Some(0.8),
                    ..Default::default()
                })
                .build()
                .await
                .unwrap();
            runtime.post_turn_hook("global", Some(Arc::new(Hook { seen: global_tx })));
            let mut events = runtime.events();
            let store_a = Arc::new(InMemorySessionStores::new());
            let store_b = Arc::new(InMemorySessionStores::new());
            let definition = || {
                AgentDefinitionSpec::new()
                    .bare_prompt("Reply briefly.")
                    .tools(openhuman_embed::ToolScopeSpec::Named(Vec::new()))
            };
            let a = runtime
                .agent(
                    AgentSpec::new("a")
                        .definition(definition())
                        .session_store(store_a.clone()),
                )
                .unwrap();
            a.post_turn_hook("local", Some(Arc::new(Hook { seen: hook_tx })));
            let b = runtime
                .agent(
                    AgentSpec::new("b")
                        .definition(definition())
                        .session_store(store_b.clone())
                        .provider(
                            Provider::custom(Arc::new(Model {
                                label: "answer-b",
                                seen: model_tx,
                            }))
                            .model("custom-b"),
                        ),
                )
                .unwrap();
            assert!(matches!(
                events.recv().await.unwrap().kind,
                RuntimeEventKind::AgentAdded
            ));
            assert!(matches!(
                events.recv().await.unwrap().kind,
                RuntimeEventKind::AgentAdded
            ));
            let first = a
                .turn("private input a")
                .session("thread-a")
                .top_p(0.3)
                .max_tokens(20)
                .send()
                .await
                .unwrap();
            assert_eq!(first.reply, "answer-a");
            let request = model_rx.recv().await.unwrap();
            assert_eq!(request.max_tokens, Some(20));
            assert_eq!(request.top_p, Some(0.3));
            assert_eq!(
                tokio::time::timeout(std::time::Duration::from_secs(5), hook_rx.recv())
                    .await
                    .unwrap()
                    .unwrap(),
                "a"
            );
            assert_eq!(
                tokio::time::timeout(std::time::Duration::from_secs(5), global_rx.recv())
                    .await
                    .unwrap()
                    .unwrap(),
                "a"
            );
            let mut stream = b.turn("private input b").session("thread-b").stream();
            let mut progress_before_final = false;
            let outcome = loop {
                match stream.recv().await.unwrap() {
                    StreamEvent::Progress(_) => progress_before_final = true,
                    StreamEvent::Finished(outcome) => break outcome.unwrap(),
                }
            };
            assert!(progress_before_final);
            assert_eq!(outcome.reply, "answer-b");
            assert!(stream.recv().await.is_none());
            let request = model_rx.recv().await.unwrap();
            assert_eq!(request.max_tokens, Some(40));
            assert_eq!(request.top_p, Some(0.8));
            assert_eq!(
                tokio::time::timeout(std::time::Duration::from_secs(5), global_rx.recv())
                    .await
                    .unwrap()
                    .unwrap(),
                "b"
            );
            assert!(hook_rx.try_recv().is_err(), "A's hook never observes B");
            runtime.post_turn_hook("global", None);
            a.post_turn_hook("local", None);
            assert_eq!(
                a.turn("private input after removing hooks")
                    .session("thread-a")
                    .send()
                    .await
                    .unwrap()
                    .reply,
                "answer-a"
            );
            let _ = model_rx.recv().await.unwrap();
            assert!(
                hook_rx.try_recv().is_err(),
                "removed local hook stays removed on resume"
            );
            assert!(
                global_rx.try_recv().is_err(),
                "removed builder hook stays removed on resume"
            );
            let stored = store_a
                .for_agent("a")
                .transcripts
                .root_for_thread("thread-a")
                .unwrap()
                .read_session()
                .unwrap()
                .unwrap();
            assert!(stored
                .messages
                .iter()
                .any(|message| message.content.contains("private input a")));
            assert!(store_b
                .for_agent("a")
                .transcripts
                .root_for_thread("thread-a")
                .is_none());
            assert!(store_a
                .for_agent("b")
                .transcripts
                .root_for_thread("thread-b")
                .is_none());
            // The session stores are inspected through their public destination
            // identity; transcript format/identity remains TinyAgents-owned.
            assert!(store_b
                .for_agent("b")
                .transcripts
                .root_for_thread("thread-b")
                .is_some());
            let mut ends = 0;
            while ends < 3 {
                let event = events.recv().await.unwrap();
                let json = serde_json::to_string(&event).unwrap();
                assert!(!json.contains("private input"));
                if matches!(event.kind, RuntimeEventKind::TurnEnded { success: true }) {
                    ends += 1;
                }
            }
            drop(stream);
            drop(a);
            drop(b);
            drop(runtime);
        })
        .await
        .expect("scenario runs on tuned worker");
    });
}
