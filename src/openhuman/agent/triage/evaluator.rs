//! Build the turn, dispatch `agent.run_turn`, parse the reply.
//!
//! This is the core of the triage pipeline. It:
//!
//! 1. Resolves a provider via [`super::routing::resolve_provider`]
//!    (commit 1 = always remote; commit 2 = local-or-remote with probe
//!    + cache).
//! 2. Looks up the `trigger_triage` built-in agent definition from
//!    the global [`AgentDefinitionRegistry`].
//! 3. Builds a [`ChatMessage`] history: the definition's system
//!    prompt body + a user message summarising the envelope.
//! 4. Dispatches the turn through the existing
//!    [`crate::openhuman::agent::bus::AGENT_RUN_TURN_METHOD`] native
//!    request so tests can override the handler via
//!    [`crate::openhuman::agent::bus::mock_agent_run_turn`].
//! 5. Parses the reply with
//!    [`super::decision::parse_triage_decision`] — tolerant enough to
//!    accept whatever 1B-parameter output looks like today.
//!
//! ## Why `run_tool_call_loop` doesn't care about `tools_registry = []`
//!
//! The triage agent has `named = []` in its TOML (zero tools). The
//! `run_tool_call_loop` implementation in
//! `src/openhuman/agent/harness/tool_loop.rs` handles an empty registry
//! by just doing a plain `chat_with_history` under the hood — no tool
//! schemas are sent to the backend. That's exactly what we want: one
//! LLM round-trip, no chained tool calls.

use std::sync::Arc;
use std::time::Instant;

use anyhow::{anyhow, Context};

use crate::core::event_bus::{request_native_global, NativeRequestError};
use crate::openhuman::agent::bus::{AgentTurnRequest, AgentTurnResponse, AGENT_RUN_TURN_METHOD};
use crate::openhuman::agent::harness::definition::{AgentDefinition, PromptSource};
use crate::openhuman::agent::harness::AgentDefinitionRegistry;
use crate::openhuman::config::MultimodalConfig;
use crate::openhuman::providers::ChatMessage;

use crate::openhuman::config::Config;

use super::decision::{parse_triage_decision, ParseError, TriageDecision};
use super::envelope::TriggerEnvelope;
use super::events;
use super::routing::{self, resolve_provider_with_config, ResolvedProvider};

/// Agent definition id for the built-in triage classifier. Hard-coded
/// so a rogue workspace TOML can't override it to point at a different
/// agent — the triage pipeline is tightly coupled to the prompt
/// contract in `trigger_triage/prompt.md`.
pub const TRIGGER_TRIAGE_AGENT_ID: &str = "trigger_triage";

/// How much of the raw payload we inline into the user message. Picked
/// so a huge Gmail body cannot blow the local model's context window.
/// The classifier only needs the gist — downstream agents (orchestrator,
/// trigger_reactor) can re-read the full payload if they need it.
const PAYLOAD_INLINE_LIMIT_BYTES: usize = 8 * 1024;

/// Final output of a single triage run — the parsed decision plus
/// bookkeeping fields published on the domain event.
#[derive(Debug, Clone)]
pub struct TriageRun {
    pub decision: TriageDecision,
    pub used_local: bool,
    pub latency_ms: u64,
}

/// Run the triage classifier against a trigger envelope.
///
/// This is the main entry point for trigger classification. It performs the following:
/// 1. Resolves an appropriate provider (preferring local LLMs for speed).
/// 2. Dispatches a single LLM turn using the `trigger_triage` archetype.
/// 3. Parses the resulting JSON decision.
/// 4. If the local attempt fails or produces garbage, automatically retries on a
///    remote provider for maximum reliability.
///
/// On success returns a [`TriageRun`] containing the decision and performance metrics.
pub async fn run_triage(envelope: &TriggerEnvelope) -> anyhow::Result<TriageRun> {
    // Load the config once and reuse it for both the first attempt and
    // any retry that falls back to remote. `Config::load_or_init` is
    // relatively heavy (disk + env merge) so paying it twice would
    // double the tail latency of a degraded-local trigger.
    let config = Config::load_or_init()
        .await
        .context("loading config for triage turn")?;
    let resolved = resolve_provider_with_config(&config)
        .await
        .context("resolving provider for triage turn")?;

    // First attempt. On success, publish latency + return.
    match run_triage_with_resolved(resolved, envelope).await {
        Ok(run) => Ok(run),
        Err(first_err)
            if first_err
                .downcast_ref::<TurnOutcomeFailure>()
                .is_some_and(|f| f.used_local) =>
        {
            // Local turn failed — mark cache degraded, rebuild a remote
            // provider from the SAME config, and retry once. If that
            // also fails, publish `TriggerEscalationFailed` and return.
            tracing::warn!(
                error = %first_err,
                "[triage::evaluator] local attempt failed — retrying on remote"
            );
            routing::mark_degraded().await;
            let remote = resolve_provider_with_config(&config)
                .await
                .context("rebuilding remote provider for triage retry")?;
            debug_assert!(!remote.used_local, "mark_degraded must force remote");
            match run_triage_with_resolved(remote, envelope).await {
                Ok(run) => {
                    tracing::info!(
                        label = %envelope.display_label,
                        "[triage::evaluator] remote retry succeeded after local failure"
                    );
                    Ok(run)
                }
                Err(second_err) => {
                    let reason =
                        format!("local then remote both failed: {first_err} / {second_err}");
                    events::publish_failed(envelope, &reason);
                    Err(anyhow!(reason))
                }
            }
        }
        Err(err) => {
            // Remote attempt failed, or local attempt failed in a way
            // that isn't eligible for retry (e.g. registry missing
            // built-in — rebuilding won't help). Publish failure.
            let reason = format!("{err}");
            events::publish_failed(envelope, &reason);
            Err(err)
        }
    }
}

/// Sentinel error wrapper the retry path looks for. We attach it to
/// recoverable failures (handler error on local, parse failure on
/// local) so `run_triage` can decide whether a second attempt is worth
/// running. Unrecoverable failures (missing registry, missing built-in
/// definition, etc.) surface as plain `anyhow::Error` and skip the
/// retry loop.
#[derive(Debug)]
struct TurnOutcomeFailure {
    used_local: bool,
    kind: &'static str,
    message: String,
}

impl std::fmt::Display for TurnOutcomeFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "triage {} ({}): {}",
            self.kind,
            if self.used_local { "local" } else { "remote" },
            self.message
        )
    }
}

impl std::error::Error for TurnOutcomeFailure {}

/// Inner half of [`run_triage`] that takes an already-resolved
/// [`ResolvedProvider`] instead of calling `routing::resolve_provider`.
///
/// Split out so unit tests can inject a stub provider without loading
/// real config from disk or constructing a real backend client. The
/// public [`run_triage`] path delegates here after resolving. Commit 2
/// will also call this from the "parse-failed → retry on remote" path
/// with a second resolved provider.
pub async fn run_triage_with_resolved(
    resolved: ResolvedProvider,
    envelope: &TriggerEnvelope,
) -> anyhow::Result<TriageRun> {
    let started = Instant::now();

    tracing::debug!(
        source = %envelope.source.slug(),
        label = %envelope.display_label,
        external_id = %envelope.external_id,
        provider = %resolved.provider_name,
        used_local = resolved.used_local,
        "[triage::evaluator] starting triage turn"
    );

    let ResolvedProvider {
        provider,
        provider_name,
        model,
        used_local,
    } = resolved;

    // ── Look up the built-in agent definition ──────────────────────
    let registry = AgentDefinitionRegistry::global().ok_or_else(|| {
        anyhow!(
            "AgentDefinitionRegistry not initialised — did startup wiring \
             skip `init_global`?"
        )
    })?;
    let definition = registry.get(TRIGGER_TRIAGE_AGENT_ID).ok_or_else(|| {
        anyhow!("built-in `{TRIGGER_TRIAGE_AGENT_ID}` definition missing from registry")
    })?;

    // ── Build the turn history ──────────────────────────────────────
    let system_prompt = extract_inline_prompt(definition).context(
        "trigger_triage agent definition must ship an inline prompt body \
         (bug: the built-in loader injects one at startup)",
    )?;
    let user_message = render_user_message(envelope);
    let history = vec![
        ChatMessage::system(&system_prompt),
        ChatMessage::user(&user_message),
    ];

    // ── Dispatch via the native bus so tests can stub it ────────────
    let request = AgentTurnRequest {
        provider: Arc::clone(&provider),
        history,
        tools_registry: Arc::new(Vec::new()),
        provider_name: provider_name.clone(),
        model: model.clone(),
        temperature: definition.temperature,
        silent: true,
        channel_name: "triage".to_string(),
        multimodal: MultimodalConfig::default(),
        // Single round-trip: the triage agent has zero tools so this
        // cap is only a safety net.
        max_tool_iterations: 1,
        on_delta: None,
        // The triage classifier runs against an empty tools registry
        // by design and emits a structured JSON decision rather than
        // calling tools — record the agent identity for tracing but
        // leave the visible-tool filter unset so the legacy unfiltered
        // behaviour is preserved.
        target_agent_id: Some("trigger_triage".to_string()),
        visible_tool_names: None,
        extra_tools: Vec::new(),
        on_progress: None,
    };
    tracing::debug!(
        provider = %provider_name,
        model = %model,
        used_local = used_local,
        "[triage::evaluator] dispatching {AGENT_RUN_TURN_METHOD}"
    );
    let response = match request_native_global::<AgentTurnRequest, AgentTurnResponse>(
        AGENT_RUN_TURN_METHOD,
        request,
    )
    .await
    {
        Ok(r) => r,
        Err(err) => {
            let message = match &err {
                NativeRequestError::HandlerFailed { message, .. } => message.clone(),
                other => format!("[agent.run_turn dispatch] {other}"),
            };
            tracing::warn!(
                error = %message,
                used_local = used_local,
                "[triage::evaluator] agent turn dispatch failed"
            );
            // Wrap in TurnOutcomeFailure so the outer `run_triage` can
            // decide whether to retry on remote. Only local failures
            // are retry-eligible.
            return Err(anyhow!(TurnOutcomeFailure {
                used_local,
                kind: "handler",
                message,
            }));
        }
    };

    // ── Parse the classifier's reply ────────────────────────────────
    let decision = match parse_triage_decision(&response.text) {
        Ok(d) => d,
        Err(parse_err) => {
            tracing::warn!(
                error = %parse_err,
                reply_chars = response.text.chars().count(),
                used_local = used_local,
                "[triage::evaluator] classifier reply did not parse"
            );
            return Err(anyhow!(TurnOutcomeFailure {
                used_local,
                kind: "parser",
                message: format_parse_error(&parse_err),
            }));
        }
    };

    let latency_ms = started.elapsed().as_millis() as u64;
    tracing::info!(
        source = %envelope.source.slug(),
        label = %envelope.display_label,
        action = %decision.action.as_str(),
        used_local = used_local,
        latency_ms = latency_ms,
        "[triage::evaluator] classifier decision produced"
    );

    Ok(TriageRun {
        decision,
        used_local,
        latency_ms,
    })
}

/// Pull the prompt body out of the definition.
///
/// Built-ins use [`PromptSource::Dynamic`] (function-driven) and
/// custom TOML definitions may use `Inline`. Only `Inline` and
/// `Dynamic` are handled here — `File`-backed sources fall into the
/// wildcard arm and return `None`. For `Dynamic`, the builder is
/// invoked with a minimal
/// [`crate::openhuman::agent::harness::definition::PromptContext`]
/// since the triage classifier does not need tool lists or memory
/// context. Returning an option here (rather than panicking) lets the
/// caller surface a clean error to downstream logging.
fn extract_inline_prompt(def: &AgentDefinition) -> Option<String> {
    match &def.system_prompt {
        PromptSource::Inline(body) if !body.is_empty() => Some(body.clone()),
        PromptSource::Dynamic(build) => {
            use crate::openhuman::context::prompt::{
                ConnectedIntegration, LearnedContextData, PromptContext, PromptTool, ToolCallFormat,
            };
            let empty_tools: Vec<PromptTool<'_>> = Vec::new();
            let empty_integrations: Vec<ConnectedIntegration> = Vec::new();
            let empty_visible: std::collections::HashSet<String> = std::collections::HashSet::new();
            let ctx = PromptContext {
                workspace_dir: std::path::Path::new("."),
                model_name: "",
                agent_id: &def.id,
                tools: &empty_tools,
                skills: &[],
                dispatcher_instructions: "",
                learned: LearnedContextData::default(),
                visible_tool_names: &empty_visible,
                tool_call_format: ToolCallFormat::PFormat,
                connected_integrations: &empty_integrations,
                connected_identities_md: String::new(),
                include_profile: false,
                include_memory_md: false,
            };
            match build(&ctx) {
                Ok(body) if !body.is_empty() => Some(body),
                Ok(_) => None,
                Err(e) => {
                    tracing::warn!(
                        agent_id = %def.id,
                        error = %e,
                        "[triage::evaluator] dynamic prompt builder failed"
                    );
                    None
                }
            }
        }
        _ => None,
    }
}

/// Render the user-side message that the classifier reads. The prompt
/// contract in `trigger_triage/prompt.md` describes the field layout;
/// we keep it trivially serialisable so a 1B model can reason over it.
fn render_user_message(envelope: &TriggerEnvelope) -> String {
    let payload_string = truncate_payload(&envelope.payload, PAYLOAD_INLINE_LIMIT_BYTES);
    format!(
        "SOURCE: {source}\n\
         DISPLAY_LABEL: {label}\n\
         EXTERNAL_ID: {eid}\n\
         PAYLOAD:\n{payload}",
        source = envelope.source.slug(),
        label = envelope.display_label,
        eid = envelope.external_id,
        payload = payload_string,
    )
}

/// Format a [`ParseError`] for inclusion in a [`TurnOutcomeFailure`]
/// message. Keeps the source-error wrapper out of the string so the
/// retry log stays readable.
fn format_parse_error(err: &ParseError) -> String {
    match err {
        ParseError::NoJsonObject => "classifier reply had no JSON object".to_string(),
        ParseError::InvalidJson(src) => format!("classifier JSON invalid: {src}"),
        ParseError::MissingTarget { action } => {
            format!("action `{action}` missing required target_agent/prompt")
        }
    }
}

/// Pretty-print `payload` as JSON, truncate if it exceeds `max_bytes`,
/// and leave a `[...truncated N bytes]` marker so the classifier
/// understands the input was abridged.
fn truncate_payload(payload: &serde_json::Value, max_bytes: usize) -> String {
    let pretty = serde_json::to_string_pretty(payload).unwrap_or_else(|_| payload.to_string());
    if pretty.len() <= max_bytes {
        return pretty;
    }
    let dropped = pretty.len() - max_bytes;
    // Split at a char boundary at or below `max_bytes` so we never
    // produce an invalid UTF-8 slice when the payload contains
    // multi-byte characters.
    let mut end = max_bytes;
    while end > 0 && !pretty.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n[...truncated {dropped} bytes]", &pretty[..end])
}

#[cfg(test)]
#[path = "evaluator_tests.rs"]
mod tests;
