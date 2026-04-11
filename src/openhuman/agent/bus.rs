//! Native event-bus handlers exposed by the agent domain.
//!
//! The agent domain publishes one native request handler, `agent.run_turn`,
//! which executes a single end-to-end agentic turn (LLM call → tool calls →
//! loop until final text) using the full `run_tool_call_loop` machinery.
//!
//! Consumers call it via [`crate::core::event_bus::request_native_global`]
//! with an [`AgentTurnRequest`] and receive an [`AgentTurnResponse`]. The
//! point is to keep the request payload as **owned Rust types** (including
//! trait objects and streaming channels) so no serialization happens and
//! consumers don't import the harness directly.
//!
//! See [`crate::openhuman::channels::runtime::dispatch`] for the primary
//! caller.

use std::sync::Arc;

use tokio::sync::mpsc;

use crate::core::event_bus::register_native_global;
use crate::openhuman::config::MultimodalConfig;
use crate::openhuman::providers::{ChatMessage, Provider};
use crate::openhuman::tools::Tool;

use super::harness::run_tool_call_loop;

/// Method name used to dispatch an agentic turn through the native bus.
pub const AGENT_RUN_TURN_METHOD: &str = "agent.run_turn";

/// Full owned payload for a single agentic turn executed through the bus.
///
/// All fields are either owned values, [`Arc`]s, or channel handles — the
/// bus carries them by value without touching serialization. Consumers can
/// therefore pass trait objects (`Arc<dyn Provider>`, tool trait-object
/// registries) and streaming senders (`on_delta`) through unchanged.
pub struct AgentTurnRequest {
    /// LLM provider, already constructed and warmed up by the caller.
    pub provider: Arc<dyn Provider>,
    /// Full conversation history including system prompt and the incoming
    /// user message. The handler mutates an internal clone of this during
    /// the tool-call loop; callers should rebuild their per-session cache
    /// from their own records, not from this vector.
    pub history: Vec<ChatMessage>,
    /// Registered tool implementations available to this turn.
    pub tools_registry: Arc<Vec<Box<dyn Tool>>>,
    /// Provider name token (e.g. `"openai"`) — routed to the loop as-is.
    pub provider_name: String,
    /// Model identifier (e.g. `"gpt-4"`) — routed to the loop as-is.
    pub model: String,
    /// Sampling temperature.
    pub temperature: f64,
    /// When `true`, suppresses stdout during the tool loop (always set by
    /// channel callers).
    pub silent: bool,
    /// Channel name this turn belongs to (e.g. `"telegram"`, `"cli"`).
    pub channel_name: String,
    /// Multimodal feature configuration (image inlining rules, payload
    /// size caps).
    pub multimodal: MultimodalConfig,
    /// Maximum number of LLM↔tool round-trips before bailing out.
    pub max_tool_iterations: usize,
    /// Optional streaming sender — the loop forwards partial LLM text
    /// chunks here so channel providers can update "draft" messages in
    /// real time. `None` disables streaming for this turn.
    pub on_delta: Option<mpsc::Sender<String>>,
}

/// Final response from an agentic turn.
pub struct AgentTurnResponse {
    /// Final assistant text after all tool calls resolved.
    pub text: String,
}

/// Register the agent domain's native request handlers on the global
/// registry. Safe to call multiple times — the last registration wins.
///
/// Called from the canonical bus wiring in
/// `src/core/jsonrpc.rs::register_domain_subscribers`.
pub fn register_agent_handlers() {
    register_native_global::<AgentTurnRequest, AgentTurnResponse, _, _>(
        AGENT_RUN_TURN_METHOD,
        |req| async move {
            let AgentTurnRequest {
                provider,
                mut history,
                tools_registry,
                provider_name,
                model,
                temperature,
                silent,
                channel_name,
                multimodal,
                max_tool_iterations,
                on_delta,
            } = req;

            tracing::debug!(
                channel = %channel_name,
                provider = %provider_name,
                model = %model,
                history_len = history.len(),
                tool_count = tools_registry.len(),
                streaming = on_delta.is_some(),
                "[agent::bus] dispatching {AGENT_RUN_TURN_METHOD}"
            );

            let text = run_tool_call_loop(
                provider.as_ref(),
                &mut history,
                tools_registry.as_ref(),
                &provider_name,
                &model,
                temperature,
                silent,
                // Approval is not wired into the channel path today; if
                // CLI migrates to the bus later, extend AgentTurnRequest
                // with `approval: Option<Arc<ApprovalManager>>` and pass
                // it through here.
                None,
                &channel_name,
                &multimodal,
                max_tool_iterations,
                on_delta,
            )
            .await
            .map_err(|e| e.to_string())?;

            tracing::debug!(
                channel = %channel_name,
                text_chars = text.chars().count(),
                "[agent::bus] {AGENT_RUN_TURN_METHOD} completed"
            );

            Ok(AgentTurnResponse { text })
        },
    );
    tracing::debug!("[agent::bus] registered native handler `{AGENT_RUN_TURN_METHOD}`");
}

// ── Shared test helpers ──────────────────────────────────────────────────
//
// Any test in `openhuman_core` that needs to stub or exercise the real
// `agent.run_turn` native handler should use these helpers rather than
// touching `register_native_global`, `register_agent_handlers`, or the
// shared `BUS_HANDLER_LOCK` directly. That keeps bus-stubbing consistent
// and panic-safe across the whole workspace — including tests outside the
// `channels` module that previously couldn't easily mock the agent turn.

/// Install a typed stub for `agent.run_turn` on the global native bus,
/// returning an RAII guard that restores the production handler on drop.
///
/// This is the canonical entry point for any test that wants to verify
/// dispatch routed through the bus OR inject a canned agent response
/// without spinning up `run_tool_call_loop`. The returned guard holds
/// [`crate::core::event_bus::testing::BUS_HANDLER_LOCK`] so other
/// dispatch tests will block until this one finishes.
///
/// # Example
///
/// ```ignore
/// use crate::openhuman::agent::bus::{mock_agent_run_turn, AgentTurnResponse};
/// use std::sync::atomic::{AtomicUsize, Ordering};
/// use std::sync::Arc;
///
/// #[tokio::test]
/// async fn channel_dispatch_hits_bus_once() {
///     let calls = Arc::new(AtomicUsize::new(0));
///     let calls_for_stub = Arc::clone(&calls);
///     let _guard = mock_agent_run_turn(move |req| {
///         let calls = Arc::clone(&calls_for_stub);
///         async move {
///             calls.fetch_add(1, Ordering::SeqCst);
///             assert_eq!(req.channel_name, "discord");
///             Ok(AgentTurnResponse { text: "CANNED".into() })
///         }
///     })
///     .await;
///
///     // ... drive the code under test ...
///     assert_eq!(calls.load(Ordering::SeqCst), 1);
///     // _guard drops → `register_agent_handlers()` runs automatically.
/// }
/// ```
#[cfg(test)]
pub async fn mock_agent_run_turn<F, Fut>(
    handler: F,
) -> crate::core::event_bus::testing::MockBusGuard
where
    F: Fn(AgentTurnRequest) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = Result<AgentTurnResponse, String>> + Send + 'static,
{
    crate::core::event_bus::testing::mock_bus_stub::<
        AgentTurnRequest,
        AgentTurnResponse,
        F,
        Fut,
        _,
    >(AGENT_RUN_TURN_METHOD, handler, || register_agent_handlers())
    .await
}

/// Acquire the shared bus handler lock and (re)register the real
/// `agent.run_turn` handler on the global native registry. Returns the
/// lock guard — callers should hold it for the duration of the test body
/// so no parallel stub-installing test can clobber the handler mid-dispatch.
///
/// Use this in tests that drive channel dispatch or otherwise depend on
/// the **real** agent turn path. For tests that want to override the
/// handler with a stub, use [`mock_agent_run_turn`] instead.
#[cfg(test)]
pub async fn use_real_agent_handler() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = crate::core::event_bus::testing::BUS_HANDLER_LOCK
        .lock()
        .await;
    register_agent_handlers();
    guard
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::event_bus::NativeRegistry;
    use async_trait::async_trait;

    /// Minimal `Provider` implementation used only to satisfy the
    /// `Arc<dyn Provider>` type in [`AgentTurnRequest`]. The tests below
    /// override the bus handler with a stub that never calls any
    /// provider methods, so this no-op is sufficient — the only required
    /// trait method is `chat_with_system`, everything else has a default.
    struct NoopProvider;

    #[async_trait]
    impl Provider for NoopProvider {
        async fn chat_with_system(
            &self,
            _system_prompt: Option<&str>,
            _message: &str,
            _model: &str,
            _temperature: f64,
        ) -> anyhow::Result<String> {
            anyhow::bail!(
                "NoopProvider::chat_with_system should not be invoked by tests that \
                 override the agent.run_turn handler"
            )
        }
    }

    /// Build a canonical test request. The bus handler is always stubbed
    /// in these tests, so the provider trait object is never actually
    /// invoked — it only needs to satisfy the type.
    fn test_request() -> AgentTurnRequest {
        AgentTurnRequest {
            provider: Arc::new(NoopProvider),
            history: vec![
                ChatMessage::system("you are a test bot"),
                ChatMessage::user("hello"),
            ],
            tools_registry: Arc::new(Vec::new()),
            provider_name: "fake-provider".into(),
            model: "fake-model".into(),
            temperature: 0.0,
            silent: true,
            channel_name: "test-channel".into(),
            multimodal: MultimodalConfig::default(),
            max_tool_iterations: 1,
            on_delta: None,
        }
    }

    #[tokio::test]
    async fn registry_override_routes_request_through_bus() {
        // Isolated local registry so this test doesn't fight the global one.
        let registry = NativeRegistry::new();
        registry.register::<AgentTurnRequest, AgentTurnResponse, _, _>(
            AGENT_RUN_TURN_METHOD,
            |req| async move {
                // Prove owned fields arrived intact across the bus boundary.
                assert_eq!(req.provider_name, "fake-provider");
                assert_eq!(req.channel_name, "test-channel");
                assert_eq!(req.history.len(), 2);
                Ok(AgentTurnResponse {
                    text: format!("handled({})", req.history.len()),
                })
            },
        );

        let resp = registry
            .request::<AgentTurnRequest, AgentTurnResponse>(AGENT_RUN_TURN_METHOD, test_request())
            .await
            .expect("dispatch should succeed");

        assert_eq!(resp.text, "handled(2)");
    }

    #[tokio::test]
    async fn streaming_delta_channel_survives_bus_roundtrip() {
        // Prove that `mpsc::Sender<String>` — a non-serializable type —
        // passes through the bus unchanged and the handler can write
        // through it. This is the whole reason native_request exists.
        let registry = NativeRegistry::new();
        registry.register::<AgentTurnRequest, AgentTurnResponse, _, _>(
            AGENT_RUN_TURN_METHOD,
            |req| async move {
                let tx = req
                    .on_delta
                    .expect("streaming test must supply an on_delta sender");
                tx.send("chunk1".into()).await.map_err(|e| e.to_string())?;
                tx.send("chunk2".into()).await.map_err(|e| e.to_string())?;
                Ok(AgentTurnResponse {
                    text: "streamed".into(),
                })
            },
        );

        let (tx, mut rx) = mpsc::channel::<String>(4);
        let collector = tokio::spawn(async move {
            let mut buf = Vec::new();
            while let Some(d) = rx.recv().await {
                buf.push(d);
            }
            buf
        });

        let mut req = test_request();
        req.on_delta = Some(tx);

        let resp = registry
            .request::<AgentTurnRequest, AgentTurnResponse>(AGENT_RUN_TURN_METHOD, req)
            .await
            .expect("dispatch should succeed");

        assert_eq!(resp.text, "streamed");

        let chunks = collector.await.unwrap();
        assert_eq!(chunks, vec!["chunk1".to_string(), "chunk2".to_string()]);
    }

    #[tokio::test]
    async fn register_agent_handlers_exposes_run_turn_on_global_registry() {
        // Read-only smoke test: prove the production registration path
        // actually puts `agent.run_turn` on the global registry. Does
        // NOT dispatch — dispatching from this test would race with any
        // other test that installs a handler override (e.g. the channel
        // dispatch integration tests in `runtime_dispatch.rs`).
        register_agent_handlers();
        let registry = crate::core::event_bus::native_registry()
            .expect("native registry should be initialized after register_agent_handlers");
        assert!(
            registry.is_registered(AGENT_RUN_TURN_METHOD),
            "`{AGENT_RUN_TURN_METHOD}` should be registered on the global native registry"
        );
    }
}
