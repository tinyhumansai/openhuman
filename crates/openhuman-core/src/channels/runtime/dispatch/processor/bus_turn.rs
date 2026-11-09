//! The orchestrator path: an unbound channel's turn goes over the native bus
//! to the `agent.run_turn` handler, with the orchestrator's tool scoping.

use crate::agent::bus::{AgentTurnRequest, AgentTurnResponse, AGENT_RUN_TURN_METHOD};
use crate::channels::context::{ChannelRouteSelection, ChannelRuntimeContext};
use crate::core::bus::BUS;
use crate::core::runtime::turn_origin::AgentTurnOrigin;
use crate::progress::AgentProgress;
use std::sync::Arc;
use tinyagents_session::transcript::TranscriptMessage;
use tinybus::NativeRequestError;
use tinychannels_bus as traits;

use super::super::routing::resolve_target_agent;

/// Run `msg`'s turn as the orchestrator over the bus.
pub(super) async fn dispatch_bus_turn(
    ctx: &ChannelRuntimeContext,
    msg: &traits::ChannelMessage,
    route: &ChannelRouteSelection,
    history: Vec<TranscriptMessage>,
    active_turn_model_source: Option<crate::agent::tinyagents::TurnModelSource>,
    progress_tx: Option<tokio::sync::mpsc::Sender<AgentProgress>>,
    turn_origin: AgentTurnOrigin,
    approval: crate::security::approval::ApprovalTurnContext,
) -> anyhow::Result<AgentTurnResponse> {
    // Dispatch the agentic turn through the native event bus instead of
    // calling `run_tool_call_loop` directly. The agent domain registers
    // an `agent.run_turn` handler at startup (see
    // `crate::agent::bus::register_agent_handlers`); this keeps
    // the channel layer free of direct harness imports and makes the
    // agent side mockable in unit tests via a handler override.
    //
    // The agent handler owns the history vector, taken here by value.
    //
    // Pick the active agent for this turn (always orchestrator) and
    // synthesise its delegation tool surface. Fresh disk read of
    // `Config::onboarding_completed` happens inside `resolve_target_agent`.
    let scoping = resolve_target_agent(&msg.channel).await;

    // A channel's explicitly-registered `tools_registry` tools are always visible
    // to the model. The resolved agent's visible-tool scope is meant to filter the
    // ambient/builtin tool surface, not to hide tools the channel deliberately
    // handed in for this turn. Without this, a channel that provides a tool
    // outside the resolved agent's `Named` scope (e.g. a test mock, or a custom
    // channel-specific tool) would be filtered out and surfaced to the model as
    // "unknown tool". When the scope is `Wildcard` (`None`), no filter applies.
    let visible_tool_names = scoping.visible_tool_names.map(|mut set| {
        for tool in ctx.tools_registry.iter() {
            set.insert(tool.name().to_string());
        }
        set
    });

    let turn_request = AgentTurnRequest {
        // Crate-native channel turn models (Phase 3 P3-B): when the runtime carries
        // the full config, build crate `ChatModel`s from `("chat", route.provider,
        // config)` — `route.provider` is the effective provider string. Tests (no
        // `config`) stay on an injected model source.
        turn_model_source: match &ctx.config {
            Some(cfg) => crate::agent::tinyagents::TurnModelSource::new_crate_native_from_string(
                &ctx.dispatch_ctx(),
                "chat",
                route.provider.clone(),
                cfg.clone(),
            ),
            None => active_turn_model_source
                .expect("test channel context must inject a turn model source"),
        },
        history,
        tools_registry: Arc::clone(&ctx.tools_registry),
        provider_name: route.provider.clone(),
        model: route.model.clone(),
        temperature: ctx.temperature,
        silent: true,
        channel_name: msg.channel.clone(),
        multimodal: ctx.multimodal.clone(),
        // Channel-sourced text is untrusted (Slack / Discord / Telegram
        // / WhatsApp / etc. — anyone who can DM the bot can put bytes
        // here). Operator-supplied defaults at `config.multimodal_files`
        // would otherwise let a remote sender smuggle a marker like
        // `[FILE:/etc/passwd]`, `[FILE:/home/<user>/.ssh/id_rsa]`, or
        // `[FILE:.env]` into the agent prompt — `read_local_file`
        // resolves the path with no workspace confinement, so absolute
        // paths exfiltrate server-local files via a follow-up question.
        //
        // Hard-disable file-marker resolution on this path regardless of
        // operator config; the desktop / web-chat path (where the user
        // owns the local filesystem) goes through a different turn
        // builder and keeps the operator default. Mirrors the triage-arm
        // hardening in `agent::triage::evaluator`.
        multimodal_files: crate::config::MultimodalFileConfig::for_untrusted_channel_input(),
        max_tool_iterations: ctx.max_tool_iterations,
        on_delta: None, // on_progress handles text deltas now
        target_agent_id: scoping.target_agent_id,
        visible_tool_names,
        extra_tools: scoping.extra_tools,
        on_progress: progress_tx,
        origin: turn_origin,
        approval,
    };
    tracing::debug!(
        channel = %msg.channel,
        provider = %route.provider,
        model = %route.model,
        "[channels::dispatch] dispatching {AGENT_RUN_TURN_METHOD} via native bus"
    );
    BUS.native()
        .request::<AgentTurnRequest, AgentTurnResponse>(AGENT_RUN_TURN_METHOD, turn_request)
        .await
        .map_err(|err| match err {
            // Unwrap handler-returned errors so the underlying
            // message (e.g. "Agent exceeded maximum tool iterations")
            // flows through without being wrapped in bus-transport
            // layer prose. The error-formatting path downstream
            // treats this `anyhow::Error` the same way it did before
            // the bus migration.
            NativeRequestError::HandlerFailed { message, .. } => {
                anyhow::anyhow!(message)
            }
            // Bus-level errors (UnregisteredHandler / TypeMismatch /
            // NotInitialized) surface with their full Display so
            // startup wiring bugs are immediately obvious in logs.
            other => anyhow::anyhow!("[agent.run_turn dispatch] {other}"),
        })
}

/// Keep the injected orchestrator graph available to processor unit fixtures
/// without leaving a second production execution path behind.
#[cfg(test)]
pub(super) async fn dispatch_test_turn(
    ctx: &ChannelRuntimeContext,
    msg: &traits::ChannelMessage,
    route: &ChannelRouteSelection,
    message: &str,
    active_turn_model_source: Option<crate::agent::tinyagents::TurnModelSource>,
    progress_tx: Option<tokio::sync::mpsc::Sender<AgentProgress>>,
    turn_origin: AgentTurnOrigin,
    approval: crate::security::approval::ApprovalTurnContext,
) -> anyhow::Result<AgentTurnResponse> {
    let history = vec![
        TranscriptMessage::system(ctx.system_prompt.current().as_str()),
        TranscriptMessage::user(message),
    ];
    dispatch_bus_turn(
        ctx,
        msg,
        route,
        history,
        active_turn_model_source,
        progress_tx,
        turn_origin,
        approval,
    )
    .await
}
