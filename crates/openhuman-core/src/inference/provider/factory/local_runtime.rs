//! Local OpenAI-compatible runtimes (Ollama / LM Studio / MLX / OMLX /
//! local-openai) as crate-native `ChatModel`s.

use super::*;
use crate::inference::provider::factory::access_gates::{
    enforce_local_only_endpoint, is_loopback_endpoint, is_ollama_cloud_model, LocalOnlyRequestGuard,
};
use tinyinference_llm::providers::openai::build_local_runtime_chat_model;
use tinyinference_llm::providers::openai::AuthStyle as CompatAuthStyle;

/// Local OpenAI-compatible runtimes (Ollama / LM Studio / MLX / OMLX /
/// local-openai) as a crate-native [`ChatModel`] (issue #4727).
///
/// Returns `None` when `role` does not resolve to a local runtime, allowing
/// [`create_chat_model_with_model_id`] to try cloud/BYOK/CLI constructors.
///
/// Endpoint/auth/`num_ctx` resolution uses the shared
/// `ollama_base_url_from_override` / `lm_studio_base_url` / profile helpers. It
/// runs the host access gates for custom/local providers —
/// [`enforce_local_only_inference`] (privacy mode) +
/// [`verify_provider_session`] (provider-specific authentication). Local runtimes
/// do not require an OpenHuman account. Temperature rides the per-call `ModelRequest` on
/// the crate path (parity with the managed-backend cutover; the `@<temp>` suffix
/// still bakes a fixed override).
///
pub(super) type ResolvedChatModel = (Arc<dyn ChatModel<()>>, String);

pub(super) type OptionalChatModelResult = Option<anyhow::Result<ResolvedChatModel>>;

pub(super) fn try_create_local_runtime_chat_model(
    role: &str,
    config: &Config,
    egress: &EgressLedger,
) -> OptionalChatModelResult {
    let resolved = provider_for_role(role, config);
    try_create_local_runtime_chat_model_from_string(role, &resolved, config, egress)
}

pub(super) fn try_create_local_runtime_chat_model_from_string(
    role: &str,
    provider: &str,
    config: &Config,
    egress: &EgressLedger,
) -> OptionalChatModelResult {
    try_create_local_runtime_chat_model_from_string_with_identity(
        role, provider, config, egress, false, None,
    )
}

pub(super) fn try_create_local_runtime_chat_model_for_inspection(
    role: &str,
    config: &Config,
) -> InspectionChatModelResult {
    let provider = provider_for_role(role, config);
    let mut identity = None;
    let result = try_create_local_runtime_chat_model_from_string_with_identity(
        role,
        &provider,
        config,
        &EgressLedger::none(),
        true,
        Some(&mut identity),
    )?;
    Some(result.and_then(|(model, model_id)| {
        let (endpoint, credential, wire_model) = identity.ok_or_else(|| {
            anyhow::anyhow!("inspection route did not expose its frozen wire identity")
        })?;
        if wire_model != model_id {
            anyhow::bail!("inspection route model identity did not match its constructed model");
        }
        Ok((model, model_id, endpoint, credential))
    }))
}

fn try_create_local_runtime_chat_model_from_string_with_identity(
    role: &str,
    provider: &str,
    config: &Config,
    egress: &EgressLedger,
    _inspection: bool,
    inspection_identity: Option<&mut Option<(String, String, String)>>,
) -> OptionalChatModelResult {
    use tinyinference_local::local_runtime::{
        resolve_local_runtime, LocalRuntimeAuth, LocalRuntimeInputs,
    };

    // Use the same classifier as privacy and session policy. Canonicalize only
    // the provider prefix: model IDs (including tags and temperature suffixes)
    // remain case-sensitive and must reach the runtime unchanged.
    let kind = tinyinference_local::profile::kind_from_provider_string(provider)?;
    let model = provider
        .trim()
        .split_once(':')
        .map_or("", |(_, model)| model);
    let p = format!("{}:{model}", kind.as_str());

    // Preserve privacy policy; local runtime authentication does not depend on
    // an OpenHuman backend session.
    if let Err(e) = enforce_local_only_inference(role, &p) {
        return Some(Err(e));
    }
    // These routes own independent local-runtime authentication; they do not
    // read or depend on the OpenHuman app-session profile.

    // Endpoint, auth, model and `@temp` resolution is the module's pure
    // `resolve_local_runtime`; the host only supplies config + env.
    let inputs = LocalRuntimeInputs {
        base_url: config.local_ai.base_url.as_deref(),
        api_key: config.local_ai.api_key.as_deref(),
        num_ctx: config.local_ai.num_ctx,
        env: host_env,
    };
    let resolved = match resolve_local_runtime(provider, &inputs)? {
        Ok(r) => r,
        Err(e) => return Some(Err(anyhow::anyhow!("{e}"))),
    };
    if let Err(e) = enforce_local_only_endpoint(role, &resolved.endpoint) {
        return Some(Err(e));
    }
    // Egress spine (privacy epic S2, #4436): the single local chokepoint for
    // every ChatModel/turn entry. A loopback runtime is disclosed as
    // non-external; another host, or an Ollama `-cloud` tag, is external.
    let external = !is_loopback_endpoint(&resolved.endpoint) || is_ollama_cloud_model(&p);
    let service = if resolved.model.is_empty() {
        role
    } else {
        &resolved.model
    };
    crate::security::egress::emit_external_transfer_in(
        egress,
        crate::security::egress::EgressDescriptor::inference(
            resolved.kind.as_str().to_string(),
            service.to_string(),
            external,
        ),
    );
    if let Some(identity) = inspection_identity {
        let credential = match &resolved.auth {
            LocalRuntimeAuth::None => String::new(),
            LocalRuntimeAuth::Bearer => resolved.api_key.clone(),
        };
        *identity = Some((
            resolved.endpoint.clone(),
            credential,
            resolved.model.clone(),
        ));
    }
    let auth = match resolved.auth {
        LocalRuntimeAuth::None => CompatAuthStyle::None,
        LocalRuntimeAuth::Bearer => CompatAuthStyle::Bearer,
    };
    log::debug!(
        "[chat-factory] local runtime resolved slug={}",
        resolved.kind.as_str()
    );
    let chat = build_local_runtime_chat_model(
        resolved.kind.as_str(),
        &resolved.endpoint,
        &resolved.api_key,
        auth,
        &resolved.model,
        &config.temperature_unsupported_models,
        resolved.temperature,
        resolved.num_ctx,
    );
    let chat: Arc<dyn ChatModel<()>> = Arc::new(LocalOnlyRequestGuard {
        inner: chat,
        role: role.to_string(),
        slug: resolved.kind.as_str().to_string(),
        endpoint: resolved.endpoint.clone(),
        model: resolved.model.clone(),
        egress: egress.clone(),
    });
    Some(Ok((chat, resolved.model)))
}

/// Environment lookup handed to the module's pure resolver.
fn host_env(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

/// Build a crate-native local-runtime model for setup/probe calls that run
/// before the desktop session gate is established.
pub(crate) fn create_local_chat_model_from_string(
    provider: &str,
    config: &Config,
) -> anyhow::Result<(Arc<dyn ChatModel<()>>, String)> {
    try_create_local_runtime_chat_model_from_string("chat", provider, config, &EgressLedger::none())
        .ok_or_else(|| anyhow::anyhow!("unsupported local provider string '{provider}'"))?
}
