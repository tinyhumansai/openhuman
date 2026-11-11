//! Static validation for chat-provider references, model construction, and temperatures.

use std::sync::Arc;

use async_trait::async_trait;

use crate::config::Config;
use crate::core::runtime::DispatchCtx;
use crate::doctor::{DoctorCheck, DoctorCtx, DoctorTier, Finding};
use crate::inference::provider::factory::create_chat_model_with_model_id_in_for_inspection;

const PROVIDER_REF_ID: &str = "inference/provider-ref";
const CONSTRUCT_ID: &str = "inference/construct";
const TEMPERATURE_ID: &str = "inference/temperature";

const CHAT_ROLE_PROVIDER_FIELDS: [(&str, &str); 6] = [
    ("chat", "chat_provider"),
    ("reasoning", "reasoning_provider"),
    ("agentic", "agentic_provider"),
    ("coding", "coding_provider"),
    ("vision", "vision_provider"),
    ("memory", "memory_provider"),
];

pub(super) struct ProviderRef;
pub(super) struct Construct;
pub(super) struct Temperature;

#[async_trait]
impl DoctorCheck for ProviderRef {
    fn id(&self) -> &str {
        PROVIDER_REF_ID
    }

    fn category(&self) -> &str {
        "inference configuration"
    }

    fn tier(&self) -> DoctorTier {
        DoctorTier::Static
    }

    async fn detect(&self, ctx: &DoctorCtx) -> Vec<Finding> {
        provider_ref(ctx.config())
    }
}

#[async_trait]
impl DoctorCheck for Construct {
    fn id(&self) -> &str {
        CONSTRUCT_ID
    }

    fn category(&self) -> &str {
        "inference configuration"
    }

    fn tier(&self) -> DoctorTier {
        DoctorTier::Static
    }

    async fn detect(&self, ctx: &DoctorCtx) -> Vec<Finding> {
        construct(ctx.config()).await
    }
}

#[async_trait]
impl DoctorCheck for Temperature {
    fn id(&self) -> &str {
        TEMPERATURE_ID
    }

    fn category(&self) -> &str {
        "inference configuration"
    }

    fn tier(&self) -> DoctorTier {
        DoctorTier::Static
    }

    async fn detect(&self, ctx: &DoctorCtx) -> Vec<Finding> {
        temperature(ctx.config())
    }
}

pub(super) fn checks() -> Vec<Arc<dyn DoctorCheck>> {
    vec![
        Arc::new(ProviderRef),
        Arc::new(Construct),
        Arc::new(Temperature),
    ]
}

fn provider_ref(config: &Config) -> Vec<Finding> {
    let mut findings = Vec::new();

    for (role, path) in CHAT_ROLE_PROVIDER_FIELDS {
        let Some(provider) = role_provider(config, role)
            .map(str::trim)
            .filter(|provider| !provider.is_empty())
        else {
            continue;
        };
        let resolves = chat_provider_resolves(config, provider);
        if !resolves {
            findings.push(unresolved_provider(path));
        }
    }

    if let Some(provider) = config
        .embeddings_provider
        .as_deref()
        .map(str::trim)
        .filter(|provider| !provider.is_empty() && *provider != "none")
    {
        if !embedding_provider_resolves(config, provider) {
            findings.push(unresolved_provider("embeddings_provider"));
        }
    }

    if let Some(primary) = config
        .primary_cloud
        .as_deref()
        .map(str::trim)
        .filter(|primary| !primary.is_empty())
    {
        let configured = config
            .cloud_providers
            .iter()
            .any(|provider| provider.id == primary);
        let registered = crate::inference::registry::chat_model(primary).is_some()
            || crate::inference::registry::catalog(primary).is_some();
        if !configured && !registered {
            findings.push(unresolved_provider("primary_cloud"));
        }
    }

    for (index, route) in config.model_routes.iter().enumerate() {
        if let Some(provider) = provider_slug_from_model_route(&route.model) {
            if !chat_provider_slug_resolves(config, provider) {
                findings.push(unresolved_provider(&format!("model_routes[{index}].model")));
            }
        }
    }

    for (index, route) in config.embedding_routes.iter().enumerate() {
        if !embedding_provider_resolves(config, &route.provider) {
            findings.push(unresolved_provider(&format!(
                "embedding_routes[{index}].provider"
            )));
        }
    }

    for (provider, path) in [
        (config.stt_provider.as_deref(), "stt_provider"),
        (config.tts_provider.as_deref(), "tts_provider"),
    ] {
        if let Some(provider) = provider.map(str::trim).filter(|value| !value.is_empty()) {
            if !voice_provider_resolves(config, provider, path == "tts_provider") {
                findings.push(unresolved_provider(path));
            }
        }
    }

    for (provider, path, is_tts) in [
        (
            config.local_ai.stt_provider.as_str(),
            "local_ai.stt_provider",
            false,
        ),
        (
            config.local_ai.tts_provider.as_str(),
            "local_ai.tts_provider",
            true,
        ),
    ] {
        let provider = provider.trim();
        if !provider.is_empty()
            && !matches!(provider, "cloud" | "openhuman" | "backend")
            && !voice_provider_resolves(config, provider, is_tts)
        {
            findings.push(unresolved_provider(path));
        }
    }

    let (stt_provider, stt_path) = effective_stt_provider(config);
    if !voice_provider_resolves(config, &stt_provider, false) {
        findings.push(unresolved_provider(stt_path));
    }

    let (tts_provider, tts_path) = effective_tts_provider(config);
    if !voice_provider_resolves(config, &tts_provider, true) {
        findings.push(unresolved_provider(tts_path));
    }

    let mut seen_paths = std::collections::HashSet::new();
    findings.retain(|finding| {
        finding
            .config_path
            .as_ref()
            .is_none_or(|path| seen_paths.insert(path.clone()))
    });
    findings
}

fn role_provider<'a>(config: &'a Config, role: &str) -> Option<&'a str> {
    match role {
        "chat" => config.chat_provider.as_deref(),
        "reasoning" => config.reasoning_provider.as_deref(),
        "agentic" => config.agentic_provider.as_deref(),
        "coding" => config.coding_provider.as_deref(),
        "vision" => config.vision_provider.as_deref(),
        "memory" => config.memory_provider.as_deref(),
        "embeddings" => config.embeddings_provider.as_deref(),
        _ => None,
    }
}

fn provider_slug_from_model_route(route: &str) -> Option<&str> {
    let route = route.trim();
    let (slug, model) = route.split_once(':')?;
    if slug.trim().is_empty() || model.trim().is_empty() {
        return None;
    }
    Some(slug.trim())
}

fn chat_provider_resolves(config: &Config, provider: &str) -> bool {
    if provider.is_empty()
        || matches!(provider, "cloud" | "openhuman" | "managed")
        || provider == "claude_agent_sdk"
        || provider.starts_with("claude_agent_sdk:")
        || provider.starts_with(tinyagents_harness::providers::claude_code::PROVIDER_PREFIX)
        || tinyinference_local::profile::is_local_provider_string(provider)
        || matches!(
            provider,
            "ollama" | "lmstudio" | "mlx" | "omlx" | "local-openai"
        )
    {
        return true;
    }
    let slug = provider
        .split_once(':')
        .map_or(provider, |(slug, _)| slug.trim());
    chat_provider_slug_resolves(config, slug)
}

fn chat_provider_slug_resolves(config: &Config, slug: &str) -> bool {
    config
        .cloud_providers
        .iter()
        .any(|provider| provider.slug == slug)
        || crate::inference::registry::chat_model(slug).is_some()
        || crate::inference::registry::catalog(slug).is_some()
}

fn embedding_provider_resolves(config: &Config, provider: &str) -> bool {
    let provider = provider.trim();
    if matches!(provider, "cloud" | "managed" | "openhuman" | "none")
        || matches!(provider, "ollama" | "openai" | "voyage" | "cohere")
        || tinyinference_local::profile::is_local_provider_string(provider)
    {
        return true;
    }
    if provider == "custom" || provider.starts_with("custom:") {
        return custom_embedding_endpoint_is_configured(config, provider);
    }
    let slug = provider
        .split_once(':')
        .map_or(provider, |(slug, _)| slug.trim());
    tinyinference_embeddings::catalog::find_provider(normalize_embedding_provider(slug)).is_some()
}

fn normalize_embedding_provider(provider: &str) -> &str {
    match provider {
        "cloud" | "openhuman" => "managed",
        other => other,
    }
}

fn custom_embedding_endpoint_is_configured(config: &Config, provider: &str) -> bool {
    provider
        .strip_prefix("custom:")
        .is_some_and(|endpoint| !endpoint.trim().is_empty())
        || config
            .custom_embeddings
            .as_ref()
            .is_some_and(|custom| !custom.endpoint.trim().is_empty())
}

fn is_voice_deferred(provider: &str) -> bool {
    matches!(
        provider.trim().to_ascii_lowercase().as_str(),
        "" | "cloud" | "openhuman" | "backend"
    )
}

fn effective_stt_provider(config: &Config) -> (String, &'static str) {
    if let Some(provider) = config
        .stt_provider
        .as_deref()
        .map(str::trim)
        .filter(|provider| !is_voice_deferred(provider))
    {
        return (provider.to_owned(), "stt_provider");
    }
    if !is_voice_deferred(&config.local_ai.stt_provider) {
        return (
            config.local_ai.stt_provider.trim().to_owned(),
            "local_ai.stt_provider",
        );
    }
    (
        config.voice_server.stt_engine.provider_string().to_owned(),
        "voice_server.stt_engine",
    )
}

fn effective_tts_provider(config: &Config) -> (String, &'static str) {
    if let Some(provider) = config
        .tts_provider
        .as_deref()
        .map(str::trim)
        .filter(|provider| !provider.is_empty())
    {
        return (provider.to_owned(), "tts_provider");
    }
    if !config.local_ai.tts_provider.trim().is_empty() {
        return (
            config.local_ai.tts_provider.trim().to_owned(),
            "local_ai.tts_provider",
        );
    }
    ("cloud".to_owned(), "tts_provider")
}

fn voice_provider_resolves(config: &Config, provider: &str, is_tts: bool) -> bool {
    let provider = provider.trim();
    if is_voice_deferred(provider) {
        return true;
    }
    if provider == "piper" {
        return is_tts;
    }
    let slug = provider
        .split_once(':')
        .map_or(provider, |(slug, _)| slug.trim());
    let configured = config.voice_providers.iter().any(|entry| {
        entry.slug == slug
            && if is_tts {
                entry.capability.supports_tts()
            } else {
                entry.capability.supports_stt()
            }
    });
    configured || registered_voice_provider(slug, is_tts)
}

#[cfg(feature = "voice")]
fn registered_voice_provider(slug: &str, is_tts: bool) -> bool {
    if is_tts {
        crate::voice::factory::registry::has_tts_provider(slug)
    } else {
        crate::voice::factory::registry::has_stt_provider(slug)
    }
}

#[cfg(not(feature = "voice"))]
fn registered_voice_provider(_slug: &str, _is_tts: bool) -> bool {
    false
}

fn unresolved_provider(path: &str) -> Finding {
    let mut finding = Finding::error(
        PROVIDER_REF_ID,
        "A configured provider reference does not resolve to an available provider.",
        "Select a configured provider or add the matching provider entry, then rerun doctor.",
    );
    finding.config_path = Some(path.to_owned());
    finding
}

async fn construct(config: &Config) -> Vec<Finding> {
    let mut findings = Vec::new();
    let dispatch = DispatchCtx::ambient();

    for (role, path) in CHAT_ROLE_PROVIDER_FIELDS {
        let result = create_chat_model_with_model_id_in_for_inspection(
            &dispatch,
            role,
            config,
            config.default_temperature,
        )
        .await;
        let Err(error) = result else {
            continue;
        };
        let class = construction_error_class(&error.to_string());
        let mut finding = Finding::error(
            CONSTRUCT_ID,
            format!("The configured {role} model could not be constructed: {class}."),
            format!("Correct `{path}` and configure the required credential, then rerun doctor."),
        );
        finding.config_path = Some(construction_config_path(config, path));
        findings.push(finding);
    }

    findings
}

fn construction_config_path(config: &Config, role_path: &str) -> String {
    if role_provider(config, role_path.trim_end_matches("_provider"))
        .is_some_and(|provider| !provider.trim().is_empty())
    {
        role_path.to_owned()
    } else if config
        .primary_cloud
        .as_deref()
        .is_some_and(|provider| !provider.trim().is_empty())
    {
        "primary_cloud".to_owned()
    } else {
        "default_model".to_owned()
    }
}

fn construction_error_class(error: &str) -> &'static str {
    let error = error.to_ascii_lowercase();
    if error.contains("requires refresh") || error.contains("expired") {
        "the stored credential requires refresh"
    } else if error.contains("credential") || error.contains("api key") {
        "a required credential is missing or unavailable"
    } else if error.contains("local-only") {
        "the configured privacy mode blocks this provider"
    } else if error.contains("backend_unavailable") || error.contains("managed inference needs") {
        "the managed provider is not available in this host"
    } else if error.contains("no model configured") || error.contains("empty model") {
        "the configured model is missing"
    } else if error.contains("no cloud provider") || error.contains("unrecognised provider") {
        "the configured provider route is invalid"
    } else {
        "the provider configuration is invalid"
    }
}

fn temperature(config: &Config) -> Vec<Finding> {
    let mut findings = Vec::new();
    add_temperature_finding(
        &mut findings,
        "default_temperature",
        config.default_temperature,
    );

    for (field, provider) in [
        ("chat_provider", config.chat_provider.as_deref()),
        ("reasoning_provider", config.reasoning_provider.as_deref()),
        ("agentic_provider", config.agentic_provider.as_deref()),
        ("coding_provider", config.coding_provider.as_deref()),
        ("vision_provider", config.vision_provider.as_deref()),
        ("memory_provider", config.memory_provider.as_deref()),
        ("embeddings_provider", config.embeddings_provider.as_deref()),
    ] {
        if let Some(provider) = provider {
            if let Some(temperature) = route_temperature(provider) {
                add_temperature_finding(&mut findings, field, temperature);
            }
        }
    }

    if let Some(default_model) = config.default_model.as_deref() {
        if let Some(temperature) = route_temperature(default_model) {
            add_temperature_finding(&mut findings, "default_model", temperature);
        }
    }

    for (index, route) in config.model_routes.iter().enumerate() {
        if let Some(temperature) = route_temperature(&route.model) {
            add_temperature_finding(
                &mut findings,
                &format!("model_routes[{index}].model"),
                temperature,
            );
        }
    }

    findings
}

fn route_temperature(route: &str) -> Option<f64> {
    let (_, model) = route.trim().split_once(':')?;
    let (model, temperature) = model.rsplit_once('@')?;
    if model.trim().is_empty() {
        return None;
    }
    temperature.trim().parse::<f64>().ok()
}

fn add_temperature_finding(findings: &mut Vec<Finding>, path: &str, temperature: f64) {
    if temperature.is_finite() && (0.0..=2.0).contains(&temperature) {
        return;
    }
    let mut finding = Finding::error(
        TEMPERATURE_ID,
        "A configured inference temperature is outside the supported range 0.0 to 2.0.",
        format!("Set a supported temperature for `{path}` and rerun doctor."),
    );
    finding.config_path = Some(path.to_owned());
    findings.push(finding);
}
