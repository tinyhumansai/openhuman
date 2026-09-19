//! Single-arm execution: dispatch one `agent.run_turn` through the
//! native bus, parse the reply, and classify any failure so the
//! fallback chain can decide what to do next.

use std::sync::Arc;
use std::time::Instant;

use anyhow::anyhow;

use crate::agent::bus::{AgentTurnRequest, AgentTurnResponse, AGENT_RUN_TURN_METHOD};
use crate::agent::harness::AgentDefinitionRegistry;
use crate::agent::messages::ChatMessage;
use crate::config::MultimodalConfig;
use crate::core::bus::BUS;
use crate::inference::provider::error_classify::{
    is_rate_limited, is_upstream_unhealthy, parse_retry_after_ms,
};
use tinybus::NativeRequestError;

use super::super::decision::parse_triage_decision;
use super::super::envelope::TriggerEnvelope;
use super::super::routing::ResolvedProvider;
use super::outcome::{TriageResolutionPath, TriageRun};
use super::prompt::{extract_inline_prompt, format_parse_error, render_user_message};

/// Agent definition id for the built-in triage classifier.
pub const TRIGGER_TRIAGE_AGENT_ID: &str = "trigger_triage";

/// Single-arm execution result. `Retryable` lets the orchestrator
/// decide whether to sleep + retry on the same arm (cloud) or to fall
/// through (local). `Fatal` short-circuits the whole chain.
pub(crate) enum ArmError {
    /// 429 / 5xx / timeout / connection — the kind of failure where
    /// trying again later might help.
    Retryable {
        retry_after_ms: Option<u64>,
        source: anyhow::Error,
    },
    /// Auth failure, missing model, prompt parse error, registry
    /// missing, etc. — retry / fallback would not change the result.
    Fatal(anyhow::Error),
    /// Cloud upstream rejected the call because the user is out of
    /// budget / credits. Retrying the cloud arm would just burn the
    /// same wall, but the local arm has no upstream cost — so we
    /// skip cloud retry, try local, and defer if local also fails.
    /// This is **not** a fatal error: the user takes an explicit
    /// action (top up) to fix it, so it must not page Sentry.
    BudgetExhausted(anyhow::Error),
    /// Our prompt-injection guard (`agent::bus`'s `enforce_prompt_input`,
    /// also used by `agent::session_host::runtime`) flagged the
    /// incoming content as adversarial / unsafe and refused to dispatch
    /// the turn. The guard runs *before* either model is contacted, so
    /// trying the same prompt again — on cloud or local — produces the
    /// same verdict. This is **not** a fatal error: the guard is doing
    /// its job (OPENHUMAN-TAURI-X regression: an adversarial Gmail
    /// message reliably trips the guard, and every fire paged Sentry).
    /// Route the same way as `BudgetExhausted` so the local-arm fallthrough
    /// lands in `TriageOutcome::Deferred` rather than `Err(_)`.
    SafetyFlagged(anyhow::Error),
}

/// Run a single arm: dispatch the agent turn through the native bus
/// and parse the reply. Classifies any error so the caller can decide
/// what to do next.
pub(super) async fn try_arm(
    resolved: &ResolvedProvider,
    envelope: &TriggerEnvelope,
    intended_path: TriageResolutionPath,
) -> Result<TriageRun, ArmError> {
    let started = Instant::now();

    tracing::debug!(
        source = %envelope.source.slug(),
        label = %envelope.display_label,
        external_id = %envelope.external_id,
        provider = %resolved.provider_name,
        used_local = resolved.used_local,
        path = intended_path.as_str(),
        "[triage::evaluator] starting triage turn"
    );

    let registry = AgentDefinitionRegistry::global().ok_or_else(|| {
        ArmError::Fatal(anyhow!(
            "AgentDefinitionRegistry not initialised — did startup wiring \
             skip `init_global`?"
        ))
    })?;
    let definition = registry.get(TRIGGER_TRIAGE_AGENT_ID).ok_or_else(|| {
        ArmError::Fatal(anyhow!(
            "built-in `{TRIGGER_TRIAGE_AGENT_ID}` definition missing from registry"
        ))
    })?;

    let system_prompt = extract_inline_prompt(definition).ok_or_else(|| {
        ArmError::Fatal(anyhow!(
            "trigger_triage agent definition must ship an inline prompt body"
        ))
    })?;
    let user_message = render_user_message(envelope);
    let history = vec![
        ChatMessage::system(&system_prompt),
        ChatMessage::user(&user_message),
    ];

    let request = AgentTurnRequest {
        turn_model_source: resolved.turn_model_source.clone(),
        history,
        tools_registry: Arc::new(Vec::new()),
        provider_name: resolved.provider_name.clone(),
        model: resolved.model.clone(),
        temperature: definition.temperature,
        silent: true,
        channel_name: "triage".to_string(),
        multimodal: MultimodalConfig::default(),
        // Triage receives untrusted text from third-party channel
        // payloads (Slack/Telegram/Discord/WhatsApp). Disable
        // file-marker resolution outright so an attacker can't smuggle
        // `[FILE:/etc/passwd]` (or any other local-path marker) into
        // an inbound message and have triage exfiltrate the contents
        // into an LLM call. The hardened constructor sets max_files=0,
        // which `prepare_messages_for_provider` short-circuits before
        // any disk read happens. The same constructor is used at the
        // main channel-dispatch site in `channels::runtime::dispatch`.
        multimodal_files: crate::config::MultimodalFileConfig::for_untrusted_channel_input(),
        max_tool_iterations: 1,
        on_delta: None,
        target_agent_id: Some("trigger_triage".to_string()),
        visible_tool_names: None,
        extra_tools: Vec::new(),
        on_progress: None,
        // Triage processes untrusted inbound channel text. Label it as
        // ExternalChannel so the approval gate treats any external_effect
        // tool call originating from this turn as remote-attacker input
        // (the triage agent doesn't usually invoke such tools — it
        // classifies and routes — but label correctly for defense in depth).
        origin: crate::agent::turn_origin::AgentTurnOrigin::ExternalChannel {
            channel: envelope.source.slug().to_string(),
            // Triage runs over an upstream envelope (composio / webhook /
            // cron / external caller) that doesn't carry a per-user sender
            // at this layer. Leave it unset and let the gate apply the
            // strict per-channel TTL-deny default.
            sender: None,
            reply_target: envelope.display_label.clone(),
            message_id: envelope.external_id.clone(),
        },
    };

    let response = match BUS
        .native()
        .request::<AgentTurnRequest, AgentTurnResponse>(AGENT_RUN_TURN_METHOD, request)
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
                path = intended_path.as_str(),
                "[triage::evaluator] agent turn dispatch failed"
            );
            return Err(classify_error(message));
        }
    };

    let decision = match parse_triage_decision(&response.text) {
        Ok(d) => d,
        Err(parse_err) => {
            tracing::warn!(
                error = %parse_err,
                reply_chars = response.text.chars().count(),
                path = intended_path.as_str(),
                "[triage::evaluator] classifier reply did not parse"
            );
            // A parse failure means the model produced unusable
            // output. Retrying the same arm with the same prompt
            // won't usually help, but on the cloud arm one retry is
            // cheap enough because hosted models can be
            // non-deterministic across calls. If the cloud retry also
            // returns malformed output, let the outer chain fall
            // through to local/Deferred instead of surfacing Err to
            // background callers like Composio trigger triage.
            return Err(match intended_path {
                TriageResolutionPath::Cloud | TriageResolutionPath::CloudAfterRetry => {
                    ArmError::Retryable {
                        retry_after_ms: None,
                        source: anyhow!(
                            "classifier reply did not parse on {} arm: {}",
                            intended_path.as_str(),
                            format_parse_error(&parse_err)
                        ),
                    }
                }
                TriageResolutionPath::LocalFallback => ArmError::Fatal(anyhow!(
                    "classifier reply did not parse on {} arm: {}",
                    intended_path.as_str(),
                    format_parse_error(&parse_err)
                )),
            });
        }
    };

    let latency_ms = started.elapsed().as_millis() as u64;
    let used_local = matches!(intended_path, TriageResolutionPath::LocalFallback);
    tracing::info!(
        source = %envelope.source.slug(),
        action = %decision.action.as_str(),
        path = intended_path.as_str(),
        latency_ms = latency_ms,
        "[triage::evaluator] classifier decision produced"
    );

    Ok(TriageRun {
        decision,
        used_local,
        latency_ms,
        resolution_path: intended_path,
    })
}

/// Classify a handler-failure message string from the agent bus into
/// either a retryable (sleep + try again) or fatal (give up) error.
pub(crate) fn classify_error(message: String) -> ArmError {
    let err = anyhow!("{message}");
    if is_rate_limited(&err) {
        return ArmError::Retryable {
            retry_after_ms: parse_retry_after_ms(&err),
            source: err,
        };
    }
    if is_upstream_unhealthy(&err) || is_transient_string(&message) {
        return ArmError::Retryable {
            retry_after_ms: None,
            source: err,
        };
    }
    // Budget-exceeded is technically a 400 (not 5xx/429), so the
    // generic transient checks above won't catch it — but it is a
    // user-actionable upstream blocker, not a code bug, so we route
    // it through `BudgetExhausted` to avoid Sentry pages.
    if is_inference_budget_exceeded(&message) {
        return ArmError::BudgetExhausted(err);
    }
    // Prompt-guard rejection (`agent::bus::enforce_prompt_input` →
    // `ReviewBlocked` / `Blocked`). The guard fires *before* either
    // arm contacts a model, so the verdict is identical on cloud and
    // local — no point retrying. Treat as Deferred-eligible so the
    // chain ends in `TriageOutcome::Deferred` rather than Fatal,
    // which was paging Sentry for adversarial-email triage attempts
    // (OPENHUMAN-TAURI-X regression).
    if is_prompt_guard_rejection(&message) {
        return ArmError::SafetyFlagged(err);
    }
    ArmError::Fatal(err)
}

/// Returns `true` when `message` is the verbatim string our prompt-injection
/// guard returns when it rejects a turn before dispatch.
///
/// Canonical sources:
/// - `crates/openhuman-core/src/agent/bus.rs` — `Blocked` / `ReviewBlocked` arms of the
///   `enforce_prompt_input` decision (the path the triage evaluator hits via
///   `agent.run_turn`).
/// - `crates/openhuman-core/src/agent/session_host/runtime.rs` — same strings in the
///   tool-call loop, kept identical so this classifier covers both.
/// - `crates/openhuman-core/src/inference/local/ops.rs` — user-facing variants with the
///   `"Please rephrase clearly."` suffix; we match the leading phrase so
///   either form classifies.
///
/// Kept narrow on purpose: the guard's full output strings are private to
/// our code, so a substring match against the leading phrase will not collide
/// with anything coming back from upstream providers.
fn is_prompt_guard_rejection(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("prompt flagged for security review")
        || lower.contains("prompt blocked by security policy")
}

/// Returns `true` when `message` signals that the upstream rejected the
/// call because the user's inference budget or credit balance is empty —
/// meaning a retry would hit the same wall.
///
/// The vocabulary matches the OpenHuman backend's error copy and common
/// third-party provider phrasing. It does **not** mirror the
/// *semantics* of `web_chat/` (a different code path);
/// it is an independent, conservative allowlist evaluated inline so the
/// triage evaluator carries no cross-domain import.
///
/// Kept conservative on purpose: a false positive would silently
/// reclassify a real `Fatal` error as `BudgetExhausted`, hiding it from
/// Sentry.
fn is_inference_budget_exceeded(message: &str) -> bool {
    // Normalize: lowercase, replace non-alphanumeric with spaces, then
    // split into whitespace-separated tokens. This lets us do
    // whole-word matching: a raw `contains("top up")` against the
    // normalized text would also fire on "stop updating" (which
    // contains the substring "top up" across word boundaries).
    let normalized: String = message
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { ' ' })
        .collect();
    let words: Vec<&str> = normalized.split_whitespace().collect();
    const NEEDLES: &[&str] = &[
        "budget exceeded",
        "budget exceeds",
        "top up",
        "add credits",
        "out of credits",
        "no remaining credits",
    ];
    NEEDLES.iter().any(|needle| {
        let needle_tokens: Vec<&str> = needle.split_whitespace().collect();
        if needle_tokens.is_empty() || words.len() < needle_tokens.len() {
            return false;
        }
        words
            .windows(needle_tokens.len())
            .any(|window| window == needle_tokens.as_slice())
    })
}

/// Heuristic for transient cloud failures the provider stack didn't
/// already classify — connection resets, timeouts, generic 5xx text.
/// Mirrors the conservative match shape used by `is_upstream_unhealthy`.
fn is_transient_string(msg: &str) -> bool {
    let lower = msg.to_lowercase();
    let hints = [
        "timed out",
        "timeout",
        "connection",
        "connect error",
        "broken pipe",
        "reset by peer",
        "deadline exceeded",
        "temporarily unavailable",
    ];
    if hints.iter().any(|h| lower.contains(h)) {
        return true;
    }
    // Bare 5xx in the message body. Be careful not to match arbitrary
    // numerals — only treat 5xx as transient.
    for token in lower.split(|c: char| !c.is_ascii_digit()) {
        if let Ok(code) = token.parse::<u16>() {
            if (500..600).contains(&code) {
                return true;
            }
        }
    }
    false
}
