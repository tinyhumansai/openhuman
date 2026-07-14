//! Native event-bus handlers exposed by the agent domain.
//!
//! The agent domain publishes one native request handler, `agent.run_turn`,
//! which executes a single end-to-end agentic turn (LLM call → tool calls →
//! loop until final text) on the tinyagents harness via
//! [`run_channel_turn_via_graph`](crate::openhuman::agent::harness::run_channel_turn_via_graph)
//! (issue #4249; the legacy `run_tool_call_loop` was removed).
//!
//! Consumers call it via [`crate::core::event_bus::request_native_global`]
//! with an [`AgentTurnRequest`] and receive an [`AgentTurnResponse`]. The
//! point is to keep the request payload as **owned Rust types** (including
//! trait objects and streaming channels) so no serialization happens and
//! consumers don't import the harness directly.
//!
//! See [`crate::openhuman::channels::runtime::dispatch`] for the primary
//! caller.

use std::collections::HashSet;
use std::sync::Arc;

use tokio::sync::mpsc;

use crate::core::event_bus::register_native_global;
use crate::openhuman::agent::progress::AgentProgress;
use crate::openhuman::agent::turn_origin::{self, AgentTurnOrigin};
use crate::openhuman::config::MultimodalConfig;
use crate::openhuman::inference::provider::{
    current_resolved_provider_route, with_resolved_provider_route_scope, ChatMessage,
};
use crate::openhuman::prompt_injection::{
    enforce_prompt_input, PromptEnforcementAction, PromptEnforcementContext,
};
use crate::openhuman::tools::Tool;

use super::harness::definition::{AgentDefinitionRegistry, SandboxMode};
use super::harness::{run_channel_turn_via_graph, with_current_sandbox_mode};
use crate::openhuman::file_state::with_file_state_agent_id;

/// Method name used to dispatch an agentic turn through the native bus.
pub const AGENT_RUN_TURN_METHOD: &str = "agent.run_turn";

/// Full owned payload for a single agentic turn executed through the bus.
///
/// All fields are either owned values, [`Arc`]s, or channel handles — the
/// bus carries them by value without touching serialization. Consumers can
/// therefore pass trait objects (`Arc<dyn Provider>`, tool trait-object
/// registries) and streaming senders (`on_delta`) through unchanged.
pub struct AgentTurnRequest {
    /// The turn's model source — the seam handle that builds this turn's tiered
    /// crate `ChatModel` set (issue #4249, Phase 3 / Motion A). Replaces the raw
    /// `Arc<dyn Provider>`: the bus/harness path names crate model types only,
    /// and the `Provider` stays confined to the inference factory + seam.
    pub turn_model_source: crate::openhuman::tinyagents::TurnModelSource,

    /// Full conversation history including system prompt and the incoming
    /// user message. The handler mutates an internal clone of this during
    /// the tool-call loop; callers should rebuild their per-session cache
    /// from their own records, not from this vector.
    pub history: Vec<ChatMessage>,

    /// Registered tool implementations available to this turn.
    /// These are provided as trait objects to avoid tight coupling with tool implementations.
    pub tools_registry: Arc<Vec<Box<dyn Tool>>>,

    /// Provider name token (e.g. `"openai"`) — routed to the loop as-is for logging and tracking.
    pub provider_name: String,

    /// Model identifier (e.g. `"gpt-4"`) — routed to the loop as-is.
    pub model: String,

    /// Sampling temperature. Higher values (e.g., 0.7) are more creative,
    /// lower (e.g., 0.0) are more deterministic.
    pub temperature: f64,

    /// When `true`, suppresses stdout during the tool loop (always set by
    /// channel callers to prevent cluttering the main console).
    pub silent: bool,

    /// Channel name this turn belongs to (e.g. `"telegram"`, `"cli"`).
    /// Used for context and telemetry.
    pub channel_name: String,

    /// Multimodal feature configuration (image inlining rules, payload
    /// size caps).
    pub multimodal: MultimodalConfig,

    /// File-attachment feature configuration (file marker count caps,
    /// per-file size budget, extracted-text limits, MIME allowlist).
    pub multimodal_files: crate::openhuman::config::MultimodalFileConfig,

    /// Maximum number of LLM↔tool round-trips before bailing out.
    /// Prevents infinite loops if a model gets "stuck" calling the same tool.
    pub max_tool_iterations: usize,

    /// Optional streaming sender — the loop forwards partial LLM text
    /// chunks here so channel providers can update "draft" messages in
    /// real time. `None` disables streaming for this turn.
    pub on_delta: Option<mpsc::Sender<String>>,

    // ── Per-agent scoping (issues #525 / #526) ────────────────────────
    /// Identifier of the agent definition this turn represents (e.g.
    /// `"orchestrator"`, `"welcome"`). Used for structured tracing and
    /// downstream bookkeeping; the actual filtering is driven by
    /// [`Self::visible_tool_names`] and [`Self::extra_tools`] below.
    /// `None` preserves the legacy "generic unfiltered turn" behaviour.
    pub target_agent_id: Option<String>,

    /// Whitelist of tool names visible to the LLM this turn. When
    /// `Some(set)`, the bus handler filters both the function-calling
    /// schema and the tool-execution lookup to names in the set.
    /// Pre-built on the dispatch side from the target agent's
    /// definition (its `[tools] named` list unioned with the names of
    /// any per-turn synthesised delegation tools). `None` means no
    /// filter — every tool in `tools_registry` plus `extra_tools` is
    /// visible.
    pub visible_tool_names: Option<HashSet<String>>,

    /// Per-turn synthesised tools to splice alongside `tools_registry`.
    /// The dispatch path uses this to carry `ArchetypeDelegationTool` /
    /// `SkillDelegationTool` instances built fresh each turn from the
    /// active agent's `subagents` field and the current Composio
    /// integrations — tools that don't exist in the global startup
    /// registry because they depend on per-user runtime state.
    /// Empty vec for agents that don't delegate.
    pub extra_tools: Vec<Box<dyn Tool>>,

    /// Optional sink for per-turn [`AgentProgress`] events — lets
    /// external channel adapters (Telegram, Slack, …) subscribe to
    /// fine-grained tool-call / text-delta / thinking-delta events and
    /// progressively edit outbound messages. `None` disables streaming
    /// status updates for this turn.
    pub on_progress: Option<mpsc::Sender<AgentProgress>>,

    /// Trust/routing label for this turn. The bus handler scopes
    /// [`AGENT_TURN_ORIGIN`](crate::openhuman::agent::turn_origin::AGENT_TURN_ORIGIN)
    /// around the tool loop so the approval gate (and any other
    /// origin-aware policy) sees the same value the caller intended.
    ///
    /// Native-bus dispatch crosses a `tokio::spawn` boundary inside the
    /// registry, so callers cannot rely on `AGENT_TURN_ORIGIN` propagating
    /// implicitly — the origin must travel as an owned field.
    ///
    /// Defaults to [`AgentTurnOrigin::Unknown`] so any caller that fails
    /// to populate this fails closed at the gate rather than silently
    /// inheriting trusted-automation semantics.
    pub origin: AgentTurnOrigin,
}

/// Final response from an agentic turn.
pub struct AgentTurnResponse {
    /// Final assistant text after all tool calls resolved and the loop terminated.
    pub text: String,
    /// Provider that actually produced the final response, after any routing,
    /// retry, or fallback layer. `None` means the provider stack did not expose
    /// resolved-route metadata and callers should fall back to the requested
    /// provider.
    pub resolved_provider: Option<String>,
    /// Model that actually produced the final response, after any routing,
    /// retry, or fallback layer. `None` means callers should fall back to the
    /// requested model.
    pub resolved_model: Option<String>,
}

impl AgentTurnResponse {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            resolved_provider: None,
            resolved_model: None,
        }
    }

    pub fn with_resolved_route(
        text: impl Into<String>,
        provider: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            text: text.into(),
            resolved_provider: Some(provider.into()),
            resolved_model: Some(model.into()),
        }
    }
}

async fn handle_agent_run_turn(req: AgentTurnRequest) -> Result<AgentTurnResponse, String> {
    let AgentTurnRequest {
        turn_model_source,
        mut history,
        tools_registry,
        provider_name,
        model,
        temperature,
        silent,
        channel_name,
        multimodal,
        multimodal_files,
        max_tool_iterations,
        on_delta,
        target_agent_id,
        visible_tool_names,
        extra_tools,
        on_progress,
        origin,
    } = req;

    tracing::debug!(
        channel = %channel_name,
        target_agent = target_agent_id.as_deref().unwrap_or("<unset>"),
        provider = %provider_name,
        model = %model,
        history_len = history.len(),
        tool_count = tools_registry.len(),
        extra_tool_count = extra_tools.len(),
        visible_tool_count = visible_tool_names.as_ref().map(|s| s.len()).unwrap_or(0),
        filter_active = visible_tool_names.is_some(),
        streaming = on_delta.is_some(),
        progress_subscribed = on_progress.is_some(),
        "[agent::bus] dispatching {AGENT_RUN_TURN_METHOD}"
    );

    if let Some(user_prompt) = history
        .iter()
        .rev()
        .find(|msg| msg.role.eq_ignore_ascii_case("user"))
        .map(|msg| msg.content.as_str())
    {
        let decision = enforce_prompt_input(
            user_prompt,
            PromptEnforcementContext {
                source: "agent.bus.run_turn",
                request_id: None,
                user_id: Some(channel_name.as_str()),
                session_id: target_agent_id.as_deref(),
            },
        );
        if !matches!(decision.action, PromptEnforcementAction::Allow) {
            tracing::warn!(
                channel = %channel_name,
                target_agent = target_agent_id.as_deref().unwrap_or("<unset>"),
                action = match decision.action {
                    PromptEnforcementAction::Allow => "allow",
                    PromptEnforcementAction::Blocked => "block",
                    PromptEnforcementAction::ReviewBlocked => "review_blocked",
                },
                score = decision.score,
                reasons = %decision
                    .reasons
                    .iter()
                    .map(|r| r.code.as_str())
                    .collect::<Vec<_>>()
                    .join(","),
                prompt_hash = %decision.prompt_hash,
                prompt_chars = decision.prompt_chars,
                "[agent::bus] prompt rejected before run_tool_call_loop"
            );
            let msg = match decision.action {
                PromptEnforcementAction::Allow => "Message accepted.",
                PromptEnforcementAction::Blocked => "Prompt blocked by security policy.",
                PromptEnforcementAction::ReviewBlocked => {
                    "Prompt flagged for security review and was not processed."
                }
            };
            return Err(msg.to_string());
        }
    }

    // Resolve the target agent's declared sandbox mode so any
    // tool executed inside the loop can read it via the
    // `CURRENT_AGENT_SANDBOX_MODE` task-local. Falls back to
    // `SandboxMode::None` when the request doesn't pin an agent
    // id (legacy "generic unfiltered turn" path) or when the
    // global registry hasn't been initialised (tests that stub
    // the bus without bootstrapping definitions).
    let sandbox_mode = target_agent_id
        .as_deref()
        .and_then(|id| AgentDefinitionRegistry::global().and_then(|reg| reg.get(id)))
        .map(|def| def.sandbox_mode)
        .unwrap_or(SandboxMode::None);

    // Scope the caller-supplied origin around the tool loop so
    // the approval gate (and any other origin-aware policy) sees
    // the same trust label the entry point intended. Native-bus
    // dispatch crosses a `tokio::spawn` boundary inside the
    // registry, so re-scoping here is mandatory — the
    // task-local does NOT propagate across that boundary
    // implicitly.
    let file_state_id = format!(
        "bus:{}:{}",
        channel_name,
        target_agent_id.as_deref().unwrap_or("root")
    );
    let (text, resolved_route) = with_resolved_provider_route_scope(async {
        let text = turn_origin::with_origin(
            origin,
            with_file_state_agent_id(
                file_state_id,
                with_current_sandbox_mode(sandbox_mode, async {
                    // Channel/CLI turns run through the tinyagents harness
                    // (issue #4249); the legacy `run_tool_call_loop` is removed.
                    // `on_progress` mirrors the harness event stream (tool
                    // timeline, text deltas, cost footer) — production channel
                    // dispatch always supplies it and now expects it live.
                    // `on_delta` (raw Sender<String>) is superseded by
                    // `on_progress` text deltas, so it's intentionally unused.
                    let _ = (&provider_name, silent, &channel_name, on_delta);
                    run_channel_turn_via_graph(
                        turn_model_source.clone(),
                        &mut history,
                        tools_registry.clone(),
                        extra_tools,
                        visible_tool_names.as_ref(),
                        &model,
                        temperature,
                        max_tool_iterations,
                        multimodal.clone(),
                        multimodal_files.clone(),
                        on_progress,
                    )
                    .await
                }),
            ),
        )
        .await
        .map_err(|e| e.to_string())?;
        let resolved_route = current_resolved_provider_route();
        Ok::<_, String>((text, resolved_route))
    })
    .await?;

    tracing::debug!(
        channel = %channel_name,
        text_chars = text.chars().count(),
        "[agent::bus] {AGENT_RUN_TURN_METHOD} completed"
    );

    Ok(match resolved_route {
        Some(route) => AgentTurnResponse::with_resolved_route(text, route.provider, route.model),
        None => AgentTurnResponse::new(text),
    })
}

#[cfg(test)]
async fn handle_agent_run_turn_on_large_stack(
    req: AgentTurnRequest,
) -> Result<AgentTurnResponse, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let handle = std::thread::Builder::new()
        .name("agent-run-turn-test".to_string())
        .stack_size(8 * 1024 * 1024)
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build agent run_turn test runtime");
            let result = runtime.block_on(handle_agent_run_turn(req));
            let _ = tx.send(result);
        })
        .map_err(|error| format!("failed to spawn large-stack agent handler: {error}"))?;

    let result = rx.await;
    handle
        .join()
        .map_err(|_| "large-stack agent handler panicked".to_string())?;
    result.map_err(|_| "large-stack agent handler exited without a result".to_string())?
}

/// Register the agent domain's native request handlers on the global
/// registry. Safe to call multiple times — the last registration wins.
///
/// This function wires the `agent.run_turn` method into the core event bus,
/// allowing any part of the system to request an agentic turn without
/// depending directly on the agent harness.
pub fn register_agent_handlers() {
    register_native_global::<AgentTurnRequest, AgentTurnResponse, _, _>(
        AGENT_RUN_TURN_METHOD,
        |req| {
            #[cfg(test)]
            {
                handle_agent_run_turn_on_large_stack(req)
            }
            #[cfg(not(test))]
            {
                handle_agent_run_turn(req)
            }
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
///             Ok(AgentTurnResponse::new("CANNED"))
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
    use crate::openhuman::inference::provider::Provider;
    use async_trait::async_trait;

    /// Minimal `Provider` implementation used only to build the
    /// [`TurnModelSource`] in [`AgentTurnRequest`]. The tests below
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
            turn_model_source: crate::openhuman::tinyagents::TurnModelSource::new(Arc::new(
                NoopProvider,
            )),
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
            multimodal_files: crate::openhuman::config::MultimodalFileConfig::default(),
            max_tool_iterations: 1,
            on_delta: None,
            target_agent_id: None,
            visible_tool_names: None,
            extra_tools: Vec::new(),
            on_progress: None,
            origin: AgentTurnOrigin::Cli,
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
                Ok(AgentTurnResponse::new(format!(
                    "handled({})",
                    req.history.len()
                )))
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
                Ok(AgentTurnResponse::new("streamed"))
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
