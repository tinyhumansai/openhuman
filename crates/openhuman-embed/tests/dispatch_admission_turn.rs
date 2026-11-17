//! Host dispatch refusal remains typed through a structured agent turn.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use openhuman_embed::budget::{
    DispatchRefusal, DispatchRefusalReason, PhysicalDispatchAdmission, PhysicalDispatchPermit,
};
use openhuman_embed::complete::ResponseFormat;
use openhuman_embed::observe::{TurnObservation, TurnObserver};
use openhuman_embed::providers::{ChatModel, ModelRequest, ModelResponse, Result};
use openhuman_embed::{
    Access, AgentDefinitionSpec, AgentSpec, CoreError, Provider, Runtime, Workspace,
};
use tinyinference_llm::usage::Usage;

struct ScriptedModel {
    calls: Arc<AtomicUsize>,
    response: &'static str,
}

#[async_trait]
impl ChatModel<()> for ScriptedModel {
    async fn invoke(&self, _: &(), _: ModelRequest) -> Result<ModelResponse> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let mut response = ModelResponse::assistant(self.response);
        response.usage = Some(Usage::new(8, 3));
        Ok(response)
    }
}

struct LimitOne(AtomicUsize);

struct Permit;

#[async_trait]
impl PhysicalDispatchAdmission for LimitOne {
    async fn acquire(
        &self,
    ) -> std::result::Result<Arc<dyn PhysicalDispatchPermit>, DispatchRefusal> {
        if self.0.fetch_add(1, Ordering::SeqCst) == 0 {
            Ok(Arc::new(Permit))
        } else {
            Err(DispatchRefusal::new(DispatchRefusalReason::CallLimit))
        }
    }
}

impl PhysicalDispatchPermit for Permit {
    fn mark_dispatched(&self) -> std::result::Result<(), DispatchRefusal> {
        Ok(())
    }
}

#[derive(Default)]
struct DispatchEvents(AtomicUsize);

impl TurnObserver for DispatchEvents {
    fn on_event(&self, event: &TurnObservation) {
        if matches!(event, TurnObservation::ModelDispatch { .. }) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
}

#[test]
fn repair_refusal_is_typed_and_does_not_reach_fallback_or_emit_a_phantom_dispatch() {
    common::runtime().block_on(async {
        tokio::spawn(async {
            let backend = common::stub_backend().await;
            let primary_calls = Arc::new(AtomicUsize::new(0));
            let fallback_calls = Arc::new(AtomicUsize::new(0));
            let primary: Arc<dyn ChatModel<()>> = Arc::new(ScriptedModel {
                calls: primary_calls.clone(),
                response: "not json",
            });
            let fallback: Arc<dyn ChatModel<()>> = Arc::new(ScriptedModel {
                calls: fallback_calls.clone(),
                response: "{\"ok\":true}",
            });
            let runtime = Runtime::builder()
                .config(common::offline_config())
                .backend_url(backend.uri())
                .workspace(Workspace::Ephemeral)
                .access(Access::readonly())
                .build()
                .await
                .expect("runtime builds");
            let agent = runtime
                .agent(
                    AgentSpec::new("dispatch-admission")
                        .provider(Provider::custom_with_fallback(primary, vec![fallback]))
                        .definition(
                            AgentDefinitionSpec::new()
                                .bare_prompt("Return a JSON object with an `ok` boolean."),
                        ),
                )
                .expect("agent builds");
            let admission = Arc::new(LimitOne(AtomicUsize::new(0)));
            let observer = Arc::new(DispatchEvents::default());

            let error = agent
                .turn("return the answer")
                .response_format(ResponseFormat::JsonObject)
                .structured_retries(1)
                .dispatch_admission(admission.clone())
                .observer(observer.clone())
                .send()
                .await
                .expect_err("the repair call is refused by host policy");

            match error {
                CoreError::DispatchAdmission { source, usage, .. } => {
                    assert_eq!(source.reason(), DispatchRefusalReason::CallLimit);
                    let usage = usage.expect("the admitted malformed answer remains accounted");
                    assert_eq!(usage.input_tokens, 8);
                    assert_eq!(usage.output_tokens, 3);
                }
                other => panic!("expected typed physical refusal, got {other:?}"),
            }
            assert_eq!(primary_calls.load(Ordering::SeqCst), 1);
            assert_eq!(fallback_calls.load(Ordering::SeqCst), 0);
            assert_eq!(admission.0.load(Ordering::SeqCst), 2);
            assert_eq!(observer.0.load(Ordering::SeqCst), 1);
        })
        .await
        .unwrap();
    });
}
