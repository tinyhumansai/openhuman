//! Implementation of process-local inference overrides supplied by the standalone CLI.
//!
//! These values deliberately live outside the serialized [`Config`]. A CLI
//! launch may choose a provider/model without changing what the desktop app or
//! the next invocation uses.

use std::sync::{OnceLock, RwLock};

use super::super::Config;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct CliInferenceOverrides {
    provider: Option<String>,
    model: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct InferenceFields {
    default_model: Option<String>,
    chat_provider: Option<String>,
    reasoning_provider: Option<String>,
    agentic_provider: Option<String>,
    coding_provider: Option<String>,
}

impl InferenceFields {
    fn from_config(config: &Config) -> Self {
        Self {
            default_model: config.default_model.clone(),
            chat_provider: config.chat_provider.clone(),
            reasoning_provider: config.reasoning_provider.clone(),
            agentic_provider: config.agentic_provider.clone(),
            coding_provider: config.coding_provider.clone(),
        }
    }

    fn restore_matching(&self, config: &mut Config, applied: &Self) {
        restore_if_applied(
            &mut config.default_model,
            &applied.default_model,
            &self.default_model,
        );
        restore_if_applied(
            &mut config.chat_provider,
            &applied.chat_provider,
            &self.chat_provider,
        );
        restore_if_applied(
            &mut config.reasoning_provider,
            &applied.reasoning_provider,
            &self.reasoning_provider,
        );
        restore_if_applied(
            &mut config.agentic_provider,
            &applied.agentic_provider,
            &self.agentic_provider,
        );
        restore_if_applied(
            &mut config.coding_provider,
            &applied.coding_provider,
            &self.coding_provider,
        );
    }
}

fn restore_if_applied<T: Clone + PartialEq>(current: &mut T, applied: &T, baseline: &T) {
    if current == applied {
        *current = baseline.clone();
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[doc(hidden)]
pub struct AppliedInferenceOverride {
    baseline: InferenceFields,
    applied: InferenceFields,
}

#[derive(Debug, Default)]
struct CliInferenceState {
    overrides: CliInferenceOverrides,
}

static CLI_INFERENCE_STATE: OnceLock<RwLock<CliInferenceState>> = OnceLock::new();

fn state() -> &'static RwLock<CliInferenceState> {
    CLI_INFERENCE_STATE.get_or_init(|| RwLock::new(CliInferenceState::default()))
}

pub(crate) fn set_cli_inference_overrides(provider: Option<&str>, model: Option<&str>) {
    let overrides = CliInferenceOverrides {
        provider: nonempty(provider),
        model: nonempty(model),
    };
    let mut state = state()
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    state.overrides = overrides;
}

pub(crate) fn apply_cli_inference_overrides(config: &mut Config) {
    let overrides = state()
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .overrides
        .clone();
    let baseline = InferenceFields::from_config(config);
    apply_overrides(config, &overrides);
    if overrides.provider.is_some() || overrides.model.is_some() {
        config.cli_inference_snapshot = Some(AppliedInferenceOverride {
            baseline,
            applied: InferenceFields::from_config(config),
        });
    }
}

/// Remove process-local launch values from a clone immediately before it is
/// serialized. A handler mutation that replaced an overridden field is kept;
/// only values still equal to the transient overlay are restored.
pub(crate) fn restore_persisted_inference_fields(config: &mut Config) {
    if let Some(applied) = config.cli_inference_snapshot.take() {
        applied.baseline.restore_matching(config, &applied.applied);
    }
}

fn nonempty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn apply_overrides(config: &mut Config, overrides: &CliInferenceOverrides) {
    if overrides.provider.is_none() && overrides.model.is_none() {
        return;
    }

    let current_chat = config.chat_provider.as_deref().unwrap_or("").trim();
    let requested_provider = overrides.provider.as_deref().unwrap_or(current_chat);
    let (requested_key, embedded_model) = split_provider_route(requested_provider);
    let provider_key = resolve_provider_key(config, requested_key);

    let current_model = if overrides.provider.is_none()
        || resolve_provider_key(config, split_provider_route(current_chat).0) == provider_key
    {
        split_provider_route(current_chat).1
    } else {
        None
    };

    let model = overrides
        .model
        .clone()
        .or_else(|| embedded_model.map(str::to_string))
        .or_else(|| current_model.map(str::to_string))
        .or_else(|| provider_default_model(config, &provider_key).map(str::to_string));

    if provider_key == "openhuman" {
        if let Some(model) = model.as_ref() {
            config.default_model = Some(model.clone());
        }
    }

    let route = match (provider_key.as_str(), model.as_deref()) {
        ("" | "openhuman", _) => "openhuman".to_string(),
        (provider, Some(model)) => format!("{provider}:{model}"),
        (provider, None) => provider.to_string(),
    };

    config.chat_provider = Some(route.clone());
    config.reasoning_provider = Some(route.clone());
    config.agentic_provider = Some(route.clone());
    config.coding_provider = Some(route);
}

fn split_provider_route(route: &str) -> (&str, Option<&str>) {
    let route = route.trim();
    match route.split_once(':') {
        Some((provider, model)) if !model.trim().is_empty() => {
            (provider.trim(), Some(model.trim()))
        }
        Some((provider, _)) => (provider.trim(), None),
        None => (route, None),
    }
}

/// Accept either the user-facing provider slug or the opaque provider id
/// stored in config. The routing factory consumes slugs, so ids are resolved
/// before the transient route is installed.
fn resolve_provider_key(config: &Config, requested: &str) -> String {
    let requested = requested.trim();
    if requested.is_empty() || requested == "cloud" {
        return config
            .primary_cloud
            .as_deref()
            .and_then(|id| config.cloud_providers.iter().find(|entry| entry.id == id))
            .map(|entry| entry.slug.trim())
            .filter(|slug| !slug.is_empty())
            .unwrap_or("openhuman")
            .to_string();
    }

    let requested = match requested {
        "lm_studio" | "lm-studio" => "lmstudio",
        other => other,
    };

    config
        .cloud_providers
        .iter()
        .find(|entry| entry.id == requested || entry.slug == requested)
        .map(|entry| entry.slug.trim())
        .filter(|slug| !slug.is_empty())
        .unwrap_or(requested)
        .to_string()
}

fn provider_default_model<'a>(config: &'a Config, provider: &str) -> Option<&'a str> {
    if matches!(provider, "ollama" | "lmstudio" | "omlx") {
        return nonempty_str(&config.local_ai.chat_model_id);
    }
    if matches!(provider, "openhuman" | "cloud") {
        return config.default_model.as_deref().and_then(nonempty_str);
    }
    config
        .cloud_providers
        .iter()
        .find(|entry| entry.slug == provider || entry.id == provider)
        .and_then(|entry| entry.default_model.as_deref())
        .and_then(nonempty_str)
}

fn nonempty_str(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
