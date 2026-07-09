//! Shared Subconscious provider routing + failure classification.
//!
//! Both subconscious worlds (the `memory` decision agent and the `tinyplace`
//! steering synthesis) run on the same **`subconscious`** provider route —
//! Connections → API keys → LLM "Subconscious" governs the cloud/local tick model.
//! The route resolution, the rate-cap circuit-breaker signature, and the two
//! permanent-error classifiers (tool-capability, per-minute token cap) are
//! therefore world-agnostic and live here, shared by the generic
//! [`super::instance`] runner and every [`super::profiles`] world.

use crate::openhuman::config::Config;
use crate::openhuman::credentials::{AuthService, APP_SESSION_PROVIDER};

/// Actionable reason surfaced (via `SubconsciousStatus.provider_unavailable_reason`)
/// when a subconscious tick fails because the configured chat model has no
/// tool-use endpoint. The memory decision turn is inherently tool-bearing (it
/// acts through tools), so a tool-incapable model can never satisfy such a tick
/// — this tells the user how to recover. See TAURI-RUST-ADC.
pub(crate) const TOOL_UNSUPPORTED_REASON: &str = "The selected chat model has no tool-use endpoint, so Subconscious can't run. Pick a tool-capable model in Connections → API keys → LLM.";

/// Surfaced in `SubconsciousStatus` when the circuit breaker has halted ticks
/// because the configured Subconscious model keeps rejecting requests with a
/// permanent per-minute token cap (413/TPM). Actionable: the fix is the user's
/// to make (a bigger model/tier), so the message points there.
pub(crate) const RATE_CAP_HALT_REASON: &str = "Subconscious is paused: the selected model rejected the request because it exceeds your provider's per-minute token limit. Pick a higher-tier model or provider for Subconscious in Connections → API keys → LLM.";

#[derive(Clone, Debug, Eq, PartialEq)]
enum SubconsciousProviderRoute {
    LocalOllama { model: String },
    OpenHumanCloud,
    Other(String),
}

/// Actionable reason the configured Subconscious provider can't run right now
/// (e.g. signed out of the OpenHuman cloud), or `None` when it is available.
/// Route resolution is shared by both worlds.
pub(crate) fn subconscious_provider_unavailable_reason(config: &Config) -> Option<String> {
    match resolve_subconscious_route(config) {
        SubconsciousProviderRoute::LocalOllama { .. } => None,
        SubconsciousProviderRoute::OpenHumanCloud => {
            if crate::openhuman::scheduler_gate::is_signed_out() {
                return Some(
                    "Sign in to use the OpenHuman cloud Subconscious provider.".to_string(),
                );
            }

            let state_dir = config
                .config_path
                .parent()
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| config.workspace_dir.clone());
            let auth = AuthService::new(&state_dir, config.secrets.encrypt);
            match auth.get_provider_bearer_token(APP_SESSION_PROVIDER, None) {
                Ok(Some(token)) if !token.trim().is_empty() => None,
                Ok(_) => Some(
                    "Sign in or configure a local Subconscious provider in Connections → API keys → LLM."
                        .to_string(),
                ),
                Err(e) => Some(format!("Unable to read the OpenHuman session: {e}")),
            }
        }
        SubconsciousProviderRoute::Other(_) => None,
    }
}

fn resolve_subconscious_route(config: &Config) -> SubconsciousProviderRoute {
    if let Some(model) = config.workload_local_model("subconscious") {
        return SubconsciousProviderRoute::LocalOllama { model };
    }

    let raw = config
        .subconscious_provider
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("cloud");
    let is_openhuman_cloud = raw.eq_ignore_ascii_case("cloud")
        || raw.eq_ignore_ascii_case("openhuman")
        || raw.to_ascii_lowercase().starts_with("openhuman:");
    if is_openhuman_cloud {
        SubconsciousProviderRoute::OpenHumanCloud
    } else {
        SubconsciousProviderRoute::Other(raw.to_string())
    }
}

/// Stable identity of the Subconscious provider routing — the exact knobs a
/// user changes in Connections → API keys → LLM to switch the tick model/provider.
/// The rate-cap circuit breaker keys its halt on this so a permanent per-minute
/// token-cap rejection stops re-firing while the SAME config is set, and
/// auto-clears the moment the user picks a different model/provider/tier.
///
/// The generic runner prefixes this with the instance id (`"memory|cloud"`) so
/// one world's halt never silences another.
pub(crate) fn subconscious_provider_signature(config: &Config) -> String {
    match resolve_subconscious_route(config) {
        SubconsciousProviderRoute::LocalOllama { model } => format!("local:{model}"),
        SubconsciousProviderRoute::OpenHumanCloud => "cloud".to_string(),
        SubconsciousProviderRoute::Other(raw) => format!("other:{raw}"),
    }
}

/// Outcome of comparing an active rate-cap halt against the live provider
/// signature at the start of a tick. Pure so it is unit-testable without
/// spinning an engine/agent.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RateCapHaltDecision {
    /// A halt is set for the same signature still in config — skip the run.
    Skip,
    /// A halt is set but the signature changed — clear it and resume ticking.
    Resume,
    /// No halt in effect — run the tick normally.
    Proceed,
}

/// Decide whether a tick should skip, resume, or proceed given the stored
/// rate-cap halt signature (if any) and the live provider signature.
pub(crate) fn evaluate_rate_cap_halt(
    halt_signature: Option<&str>,
    current: &str,
) -> RateCapHaltDecision {
    match halt_signature {
        Some(sig) if sig == current => RateCapHaltDecision::Skip,
        Some(_) => RateCapHaltDecision::Resume,
        None => RateCapHaltDecision::Proceed,
    }
}

/// True when an agent-run error is a permanent per-minute token-cap rejection
/// (413/TPM) — the request is larger than the provider account's per-minute
/// budget, so retrying the same tick can never succeed. Delegates to the shared
/// provider matcher (single source of truth with the Sentry classifier in
/// `core::observability`) so the wording can't drift. TAURI-RUST-HXF.
pub(crate) fn is_permanent_rate_cap_error(msg: &str) -> bool {
    crate::openhuman::inference::provider::is_provider_rate_cap_exceeded_message(msg)
}

/// True when an agent-run error means the configured chat model can't do tool
/// calls at all — a permanent, user-actionable condition (pick a tool-capable
/// model). Matches both the direct-provider body (`<model> does not support
/// tools`) and OpenRouter's router-level phrasing (`No endpoints found that
/// support tool use`, TAURI-RUST-ADC). Kept narrow to tool capability so an
/// unrelated provider error (auth, billing, rate-limit) is not misread as one.
pub(crate) fn is_tool_capability_error(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    lower.contains("no endpoints found that support tool use")
        || lower.contains("does not support tools")
}

#[cfg(test)]
#[path = "provider_tests.rs"]
mod tests;
