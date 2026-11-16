//! A call's per-turn overrides (model, temperature, inference route,
//! reasoning effort) applied to its own in-memory `Config` copy before the
//! session is built.

use super::guards::normalize_model_override;
use crate::config::Config;

/// Applies a call's model, temperature and inference route to `config`, in
/// memory, before its session is built.
///
/// `config` is the call's own copy (the controller loads it per call), so
/// nothing here outlives the turn or reaches `config.toml`.
///
/// # Model precedence for the turn
///
/// 1. `route` (`inference_url`): pins the agent-turn roles to
///    `ephemeral-route:<model>`, the model being the one resolved below.
/// 2. `model_override`: replaces `default_model` **and** the model of every
///    agent-turn role that is pinned (chat, reasoning, agentic, coding — the
///    roles [`ephemeral_route::apply`](crate::config::schema::ephemeral_route::apply)
///    pins). A pinned role route outranks `default_model` in
///    [`provider_for_role`](crate::inference::provider::provider_for_role),
///    so writing only `default_model` left a host that pins its roles
///    (`chat_provider = "local-openai:A"`) asking the endpoint for `A`
///    whatever the caller asked for. See [`pin_turn_roles_to_model`].
///    Background roles (memory, embeddings, vision) keep their pins: they run
///    tier-specific models a chat model cannot stand in for.
/// 3. The configured role pins, then `default_model`, as without overrides.
///
/// Config-level agent pins (a hosted profile's fixed model,
/// `orchestrator.model`, `[teams.*]`) are applied later by the session
/// builder and still name the session's model; an agent definition's own
/// `model` steers the sub-agents it is spawned as, not a top-level turn.
pub(crate) fn apply_turn_overrides(
    config: &mut Config,
    model_override: Option<String>,
    temperature: Option<f64>,
    route: Option<crate::config::schema::EphemeralRoute>,
) {
    // TAURI-RUST-RS: an upstream caller (frontend, JSON-RPC client) can pass
    // `model_override: Some("")`. See `normalize_model_override` for the
    // rationale — an empty / whitespace-only override collapses to `None`.
    if let Some(model) = normalize_model_override(model_override) {
        pin_turn_roles_to_model(config, &model);
        config.default_model = Some(model);
    }
    if let Some(temp) = temperature {
        config.default_temperature = temp;
    }
    // After the model override, because the route pins its roles to
    // `"<slug>:<model>"` and the model it uses is the one this call resolved.
    if let Some(route) = route {
        crate::config::schema::ephemeral_route::apply(config, route);
    }
}

/// Points the pinned agent-turn roles of this call's `config` at `model`.
///
/// A `hint:<role>` override selects a workload (the session builder's
/// `provider_role_for`) rather than a model, so the pins are left alone for
/// it. Unpinned roles need nothing: they already fall back to
/// `default_model`.
fn pin_turn_roles_to_model(config: &mut Config, model: &str) {
    if model.trim().starts_with("hint:") {
        log::debug!("[inference] agent_chat model_override is a workload hint; role pins kept");
        return;
    }
    let pins = [
        config.chat_provider.as_deref(),
        config.reasoning_provider.as_deref(),
        config.agentic_provider.as_deref(),
        config.coding_provider.as_deref(),
    ]
    .map(|pinned| {
        pinned
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(|pinned| {
                (
                    pinned.to_string(),
                    crate::inference::provider::factory::route_for_turn_model(
                        pinned, model, config,
                    ),
                )
            })
    });
    let slots = [
        ("chat", &mut config.chat_provider),
        ("reasoning", &mut config.reasoning_provider),
        ("agentic", &mut config.agentic_provider),
        ("coding", &mut config.coding_provider),
    ];
    for ((role, slot), pin) in slots.into_iter().zip(pins) {
        let Some((pinned, routed)) = pin else {
            continue;
        };
        if routed != pinned {
            log::debug!(
                "[inference] agent_chat model_override re-pins role={role} from={pinned} to={routed}"
            );
            *slot = Some(routed);
        }
    }
}

/// Applies a call's `inference_reasoning_effort` when it has no
/// `inference_url` route to carry it.
///
/// The effort rides [`EphemeralRoute`](crate::config::schema::EphemeralRoute),
/// which exists only when the call names an endpoint and a key, so on a host
/// that routes through its configured providers (a pinned `local-openai`
/// route) the parameter was dropped. It is set here as the turn model's own
/// level (`runtime.reasoning_effort_by_model`, keyed by the model the turn
/// resolves: `model_override`, else `default_model`), which
/// [`reasoning_for_config`](crate::inference::reasoning) ranks above the
/// configured levels; with no model it becomes this call's
/// `runtime.reasoning_effort`. Blank collapses to `None` (no change).
pub(crate) fn apply_turn_reasoning_effort(
    config: &mut Config,
    model_override: Option<&str>,
    effort: Option<String>,
) {
    let Some(effort) = effort
        .map(|effort| effort.trim().to_string())
        .filter(|effort| !effort.is_empty())
    else {
        return;
    };
    let model = normalize_model_override(model_override.map(str::to_string)).or_else(|| {
        config
            .default_model
            .as_deref()
            .map(str::trim)
            .filter(|model| !model.is_empty())
            .map(str::to_string)
    });
    log::debug!(
        "[inference] agent_chat reasoning effort={effort} for model={:?} (no inference_url)",
        model
    );
    match model {
        Some(model) => {
            config
                .runtime
                .reasoning_effort_by_model
                .insert(model, effort);
        }
        None => config.runtime.reasoning_effort = Some(effort),
    }
}

#[cfg(test)]
#[path = "turn_overrides_tests.rs"]
mod tests;
