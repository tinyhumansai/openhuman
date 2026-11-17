//! Dropping a stream cancels the native turn instead of detaching it forever.
mod common;
use async_trait::async_trait;
use openhuman_embed::providers::{ChatModel, ModelRequest, ModelResponse};
use openhuman_embed::{Access, AgentDefinitionSpec, AgentSpec, Provider, Runtime, ToolScopeSpec};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

struct BlockingFirstCall {
    first: AtomicBool,
    started: tokio::sync::mpsc::UnboundedSender<()>,
    released: tokio::sync::mpsc::UnboundedSender<()>,
}
struct InFlight(tokio::sync::mpsc::UnboundedSender<()>);
impl Drop for InFlight {
    fn drop(&mut self) {
        let _ = self.0.send(());
    }
}
#[async_trait]
impl ChatModel<()> for BlockingFirstCall {
    async fn invoke(
        &self,
        _: &(),
        _: ModelRequest,
    ) -> openhuman_embed::providers::Result<ModelResponse> {
        if self.first.swap(false, Ordering::SeqCst) {
            let _guard = InFlight(self.released.clone());
            self.started.send(()).unwrap();
            std::future::pending::<()>().await;
        }
        Ok(ModelResponse::assistant("after cancellation"))
    }
}

struct FloodModel {
    buffered: tokio::sync::mpsc::UnboundedSender<()>,
    released: tokio::sync::mpsc::UnboundedSender<()>,
}
struct FloodStream {
    emitted: usize,
    buffered: tokio::sync::mpsc::UnboundedSender<()>,
    _guard: InFlight,
}
impl futures_core::Stream for FloodStream {
    type Item = openhuman_embed::providers::ModelStreamItem;
    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.emitted += 1;
        if self.emitted == 128 {
            self.buffered.send(()).unwrap();
        }
        std::task::Poll::Ready(Some(Self::Item::MessageDelta(
            openhuman_embed::providers::MessageDelta {
                text: "delta".into(),
                reasoning: String::new(),
                tool_call: None,
            },
        )))
    }
}
#[async_trait]
impl ChatModel<()> for FloodModel {
    fn profile(&self) -> Option<&openhuman_embed::providers::ModelProfile> {
        static PROFILE: std::sync::LazyLock<openhuman_embed::providers::ModelProfile> =
            std::sync::LazyLock::new(|| openhuman_embed::providers::ModelProfile {
                streaming: true,
                ..Default::default()
            });
        Some(&PROFILE)
    }
    async fn invoke(
        &self,
        _: &(),
        _: ModelRequest,
    ) -> openhuman_embed::providers::Result<ModelResponse> {
        panic!("the streaming fixture must stream");
    }
    async fn stream(
        &self,
        _: &(),
        _: ModelRequest,
    ) -> openhuman_embed::providers::Result<openhuman_embed::providers::ModelStream> {
        Ok(openhuman_embed::providers::ModelStream::new(Box::pin(
            FloodStream {
                emitted: 0,
                buffered: self.buffered.clone(),
                _guard: InFlight(self.released.clone()),
            },
        )))
    }
}

#[test]
fn dropping_a_stream_releases_the_inflight_model_and_the_agent_can_run_again() {
    common::runtime().block_on(async {
        tokio::spawn(async {
            let backend = common::stub_backend().await;
            let (started, mut start_rx) = tokio::sync::mpsc::unbounded_channel();
            let (released, mut release_rx) = tokio::sync::mpsc::unbounded_channel();
            let model = Arc::new(BlockingFirstCall {
                first: AtomicBool::new(true),
                started,
                released,
            });
            let runtime = Runtime::builder()
                .config(common::offline_config())
                .backend_url(backend.uri())
                .access(Access::readonly())
                .provider(Provider::custom(model.clone()).model("cancel-model"))
                .build()
                .await
                .unwrap();
            let agent = runtime
                .agent(
                    AgentSpec::new("stream-cancel").definition(
                        AgentDefinitionSpec::new()
                            .bare_prompt("Reply.")
                            .tools(ToolScopeSpec::Named(Vec::new())),
                    ),
                )
                .unwrap();
            let stream = agent.stream("wait forever");
            let token = stream.cancellation_token();
            tokio::time::timeout(std::time::Duration::from_secs(10), start_rx.recv())
                .await
                .unwrap()
                .unwrap();
            drop(stream);
            assert!(token.is_cancelled());
            tokio::time::timeout(std::time::Duration::from_secs(10), release_rx.recv())
                .await
                .unwrap()
                .unwrap();
            let reply =
                tokio::time::timeout(std::time::Duration::from_secs(10), agent.run("next turn"))
                    .await
                    .unwrap()
                    .unwrap();
            assert_eq!(reply.reply, "after cancellation");
            model.first.store(true, Ordering::SeqCst);
            let external = openhuman_embed::CancellationToken::new();
            let mut stream = agent
                .turn("cancel externally")
                .cancellation(external.clone())
                .stream();
            tokio::time::timeout(std::time::Duration::from_secs(10), start_rx.recv())
                .await
                .unwrap()
                .unwrap();
            external.cancel();
            assert!(stream.cancellation_token().is_cancelled());
            tokio::time::timeout(std::time::Duration::from_secs(10), release_rx.recv())
                .await
                .unwrap()
                .unwrap();
            while let Some(event) =
                tokio::time::timeout(std::time::Duration::from_secs(10), stream.recv())
                    .await
                    .unwrap()
            {
                if matches!(event, openhuman_embed::StreamEvent::Finished(_)) {
                    break;
                }
            }
            drop(stream);
            // Ordinary send shares the same native child scope. Cancelling
            // its acknowledgement handle must leave the caller's parent usable.
            model.first.store(true, Ordering::SeqCst);
            let shared = openhuman_embed::CancellationToken::new();
            let mut configured = agent
                .turn("cancel ordinary send")
                .cancellation(shared.clone());
            let handle = configured.cancellation_handle();
            let running = tokio::spawn(configured.send());
            tokio::time::timeout(std::time::Duration::from_secs(10), start_rx.recv())
                .await
                .unwrap()
                .unwrap();
            tokio::time::timeout(std::time::Duration::from_secs(10), handle.cancel())
                .await
                .expect("ordinary send acknowledges native cleanup");
            assert!(running.await.unwrap().is_err());
            tokio::time::timeout(std::time::Duration::from_secs(10), release_rx.recv())
                .await
                .unwrap()
                .unwrap();
            assert!(
                !shared.is_cancelled(),
                "turn-local cancellation must not cancel its parent"
            );
            assert_eq!(
                agent
                    .turn("reuse caller parent")
                    .cancellation(shared)
                    .send()
                    .await
                    .unwrap()
                    .reply,
                "after cancellation"
            );
            let (buffered, mut buffered_rx) = tokio::sync::mpsc::unbounded_channel();
            let (released, mut released_rx) = tokio::sync::mpsc::unbounded_channel();
            let flood = runtime
                .agent(
                    AgentSpec::new("backpressured")
                        .provider(
                            Provider::custom(Arc::new(FloodModel { buffered, released }))
                                .model("flood"),
                        )
                        .definition(
                            AgentDefinitionSpec::new()
                                .bare_prompt("Reply.")
                                .tools(ToolScopeSpec::Named(Vec::new())),
                        ),
                )
                .unwrap();
            let external = openhuman_embed::CancellationToken::new();
            let unread = flood
                .turn("fill the bounded buffers")
                .cancellation(external.clone())
                .stream();
            tokio::time::timeout(std::time::Duration::from_secs(10), buffered_rx.recv())
                .await
                .unwrap()
                .unwrap();
            external.cancel();
            // Keep the receiver alive and unread: cancellation must interrupt a
            // blocked progress send rather than waiting for the consumer to drain.
            tokio::time::timeout(std::time::Duration::from_secs(10), released_rx.recv())
                .await
                .unwrap()
                .unwrap();
            drop(unread);
            // The acknowledgement handle and deadline also release a model
            // while the stream receiver remains alive and backpressured.
            let mut configured = flood.turn("cancel with the acknowledgement handle");
            let handle = configured.cancellation_handle();
            let mut unread = configured.stream();
            tokio::time::timeout(std::time::Duration::from_secs(10), buffered_rx.recv())
                .await
                .unwrap()
                .unwrap();
            tokio::time::timeout(std::time::Duration::from_secs(10), handle.cancel())
                .await
                .expect("acknowledged cancellation under backpressure");
            tokio::time::timeout(std::time::Duration::from_secs(10), released_rx.recv())
                .await
                .unwrap()
                .unwrap();
            while let Some(event) = unread.recv().await {
                if let openhuman_embed::StreamEvent::Finished(result) = event {
                    assert!(matches!(
                        result,
                        Err(openhuman_embed::CoreError::TurnCancelled { .. })
                    ));
                    break;
                }
            }
            drop(unread);
            let mut unread = flood
                .turn("deadline with an unread stream")
                .timeout(std::time::Duration::from_secs(2))
                .stream();
            tokio::time::timeout(std::time::Duration::from_secs(10), buffered_rx.recv())
                .await
                .unwrap()
                .unwrap();
            tokio::time::timeout(std::time::Duration::from_secs(10), released_rx.recv())
                .await
                .expect("deadline interrupts backpressure")
                .unwrap();
            while let Some(event) = unread.recv().await {
                if let openhuman_embed::StreamEvent::Finished(result) = event {
                    assert!(matches!(
                        result,
                        Err(openhuman_embed::CoreError::DeadlineExceeded { .. })
                    ));
                    break;
                }
            }
            drop(unread);
            drop(flood);
            drop(agent);
            drop(runtime);
        })
        .await
        .expect("scenario runs on tuned worker");
    });
}
