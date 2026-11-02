//! `<slug>:<model>` BYOK cloud providers: shared slug resolution (model fallback,
//! abstract-tier remapping, credentials, codex routing) and the crate-native builders.

use super::{
    access_gates::enforce_local_only_inference,
    cloud_spec,
    credentials::{CredentialAccess, RuntimeCredentialSnapshot},
    local_runtime::OptionalChatModelResult,
    managed_backend::managed_chat_model_for,
    provider_for_role, redact_endpoint_origin, InspectionOutputBudgetCapability,
    BYOK_INCOMPLETE_SENTINEL, CLAUDE_AGENT_SDK_PREFIX, CLAUDE_AGENT_SDK_PROVIDER,
    PROVIDER_OPENHUMAN,
};
use crate::config::ops::resolve_primary_cloud_provider_string;
use crate::config::schema::cloud_providers::AuthStyle;
use crate::config::Config;
use crate::inference::provider::fallback_diagnostics;
use crate::inference::provider::openai_codex::resolve_openai_codex_routing_for_inspection;
use crate::security::egress::EgressLedger;
use std::sync::Arc;
use tinyinference_llm::model::ChatModel;
use tinyinference_llm::providers::anthropic::endpoint_is_anthropic_messages;
use tinyinference_llm::providers::openai::codex::{
    openai_codex_client_version, openai_codex_user_agent, OPENAI_CODEX_ACCOUNT_HEADER,
    OPENAI_CODEX_ORIGINATOR, OPENAI_CODEX_ORIGINATOR_HEADER,
};
use tinyinference_llm::providers::registry::resolve_slug_model;
use tinyinference_providers::route::split_model_and_temperature;

type CloudSlugInspectionChatModelResult = Option<
    anyhow::Result<(
        Arc<dyn ChatModel<()>>,
        String,
        String,
        String,
        InspectionOutputBudgetCapability,
    )>,
>;

/// Look up a `cloud_providers` entry by slug and build the provider.
/// The shared resolution for a `<slug>:<model>` cloud provider — the cloud
/// `cloud_providers` entry, the effective model id (with `default_model`
/// fallback + abstract-tier remapping), the resolved API key, and the OpenAI
/// codex-oauth routing shared by every cloud `ChatModel` constructor.
pub(crate) struct CloudSlugResolution<'a> {
    pub(crate) entry: &'a crate::config::schema::cloud_providers::CloudProviderCreds,
    pub(crate) effective_model: String,
    pub(crate) key: String,
    pub(crate) codex: tinyinference_llm::providers::openai::codex::OpenAiCodexRouting,
}

pub(super) fn inspection_output_budget_capability(
    codex: &tinyinference_llm::providers::openai::codex::OpenAiCodexRouting,
) -> InspectionOutputBudgetCapability {
    if codex.using_oauth {
        InspectionOutputBudgetCapability::Unsupported
    } else {
        InspectionOutputBudgetCapability::Bounded
    }
}

/// The user-facing error for a failed credential lookup: the guidance, plus
/// the expiry signal when the lookup reported one. The doctor's
/// refresh-required check and the Codex reconnect classifier key on those
/// fixed phrases; the rest of the lookup error can carry stored-credential
/// content or a refresh response's body, so it is dropped.
pub(super) fn lookup_failure(error: &anyhow::Error, guidance: String) -> anyhow::Error {
    let text = format!("{error:#}");
    let signal = if text.contains(super::credentials::OAUTH_REFRESH_REQUIRED) {
        Some(super::credentials::OAUTH_REFRESH_REQUIRED)
    } else if tinyinference_llm::failure::is_codex_token_expired_text(&text.to_ascii_lowercase()) {
        Some(CODEX_TOKEN_EXPIRED)
    } else {
        None
    };
    match signal {
        Some(signal) => anyhow::anyhow!("{guidance} (underlying error: {signal})"),
        None => anyhow::anyhow!("{guidance}"),
    }
}

/// The Codex OAuth expiry phrase (`openai_oauth::store`) the reconnect
/// classifier recognises.
const CODEX_TOKEN_EXPIRED: &str = "Codex authentication token is expired";

pub(super) fn resolve_cloud_slug_with_access<'a>(
    role: &str,
    slug: &str,
    model: &str,
    config: &'a Config,
    extensions: Option<&crate::core::slots::Extensions>,
    credential_access: CredentialAccess<'_>,
    inspection_credential: Option<&super::credentials::ReadOnlyCloudCredential>,
) -> anyhow::Result<CloudSlugResolution<'a>> {
    let entry = config.cloud_providers.iter().find(|e| e.slug == slug);

    let entry = entry.ok_or_else(|| {
        let known: Vec<&str> = config
            .cloud_providers
            .iter()
            .map(|e| e.slug.as_str())
            .collect();
        anyhow::anyhow!(
            "[chat-factory] no cloud provider configured for slug '{}' (role '{}') — \
             add an entry with that slug to cloud_providers in config.toml. \
             Configured slugs: [{}]",
            slug,
            role,
            known.join(", ")
        )
    })?;

    let hosted = crate::config::schema::cloud_providers::is_hosted_auth_style_for(
        extensions,
        entry.auth_style,
    );
    // Hosted routes derive their model from the managed provider and retain the
    // old permissive behavior. BYOK model fallback and hint classification are
    // shared with the provider owner.
    let effective_model = if hosted {
        if model.trim().is_empty() {
            entry.default_model.clone().unwrap_or_default()
        } else {
            model.to_string()
        }
    } else {
        match resolve_slug_model(model, entry.default_model.as_deref(), |candidate| {
            crate::inference::provider_model::is_abstract_model_for(
                extensions,
                PROVIDER_OPENHUMAN,
                candidate,
            )
        }) {
            Ok(model) => model,
            Err(tinyinference_llm::providers::registry::SlugModelError::MissingModel) => {
                log::warn!(
                    "[providers][chat-factory] role={} slug={} resolved to empty model — \
                     provider string must include a model id (e.g. '{}:<model-id>') or \
                     set default_model on the cloud_providers entry",
                    role,
                    slug,
                    slug,
                );
                anyhow::bail!(
                    "[chat-factory] no model configured: role '{}' resolved to an empty model id for slug '{}'. \
                     Include a model in the provider string (e.g. '{slug}:<model-id>') or \
                     set default_model on the cloud_providers entry for slug '{slug}'.",
                    role,
                    slug,
                );
            }
            Err(tinyinference_llm::providers::registry::SlugModelError::NoConcreteDefault) => {
                let requested_model = if model.trim().is_empty() {
                    entry.default_model.as_deref().unwrap_or_default()
                } else {
                    model
                };
                anyhow::bail!(
                    "[chat-factory] model '{}' is an abstract tier for role '{}', \
                     but cloud provider slug '{}' has no concrete default_model configured. \
                     Set cloud_providers[].default_model to a provider-native model id (e.g. deepseek-v4-pro).",
                    requested_model,
                    role,
                    slug
                );
            }
        }
    };

    if !hosted && effective_model != model && !model.trim().is_empty() {
        log::info!(
            "[providers][chat-factory] role={} slug={} remapping abstract model {} -> {}",
            role,
            slug,
            model,
            effective_model
        );
    }

    log::info!(
        "[providers][chat-factory] role={} slug={} model={} endpoint_host={}",
        role,
        slug,
        effective_model,
        redact_endpoint_origin(&entry.endpoint)
    );

    // #5146 §2.1: a raw "failed to read API key for slug 'anthropic'" is
    // baffling when the user never configured Anthropic — they set a local
    // Ollama model and this is a background role that fell back to the cloud.
    // Attach the role, and the local chat model that caused the fallback, so
    // the message explains itself and names a concrete remedy.
    // Only an *implicit* fallback is explained as one. A role with its own
    // explicit cloud route can fail key lookup here too, and telling that user
    // their local chat model caused it would be a lie.
    let fallback_diagnostic = super::fallback_diagnostics_for_role(role, config);
    let implicit_fallback = fallback_diagnostic.is_some();
    let local_chat = if implicit_fallback {
        config
            .chat_provider
            .as_deref()
            .filter(|chat| tinyinference_local::profile::is_local_provider_string(chat))
    } else {
        None
    };
    let missing_credentials = || {
        // Safe fields only: role, slug, and the routing shape. Never the
        // underlying error (it can echo a key) and never the key itself.
        log::warn!(
            "[providers][chat-factory] credential lookup failed role={} slug={} auth_style={} implicit_cloud_fallback={}",
            role,
            slug,
            entry.auth_style.as_str(),
            implicit_fallback
        );
        fallback_diagnostic.as_ref().map_or_else(
            || format!("No usable credentials for '{slug}'. Add a key in Connections → LLM."),
            |diagnostic| {
                fallback_diagnostics::missing_provider_credentials_message(
                    diagnostic, slug, local_chat,
                )
            },
        )
    };

    let (key, bearer_is_oauth, oauth_account_id) =
        if credential_access.is_inspection() && entry.auth_style == AuthStyle::OpenhumanJwt {
            // The managed model constructor receives its credential later, on
            // invoke. Its hosted construction callback is config-only and offline.
            (String::new(), false, None)
        } else if credential_access.is_inspection() {
            let credential = inspection_credential
                .ok_or_else(|| anyhow::anyhow!("inspection credential was not pre-resolved"))?;
            (
                credential.key.clone(),
                credential.is_oauth,
                credential.account_id.clone(),
            )
        } else {
            let key = credential_access
                .lookup_key_for_slug(slug)
                .map_err(|error| lookup_failure(&error, missing_credentials()))?;
            let is_oauth = slug == "openai" && credential_access.openai_bearer_is_oauth();
            (key, is_oauth, None)
        };

    // A readable auth profile with no key for this slug returns `Ok("")`, which
    // would otherwise build a client with an empty bearer and surface as a raw
    // 401 from the provider several layers later — exactly the baffling error
    // this diagnostic exists to replace.
    //
    // Runtime construction only rejects missing credentials on the implicit
    // fallback path: explicit routes may be constructed before a key is saved.
    // Inspection is stricter because doctor must report an unusable authenticated
    // route without invoking the model.
    //
    // Styles that carry no stored key (`OpenhumanJwt` injects a session JWT
    // downstream, `None` sends no auth header at all) are legitimately blank and
    // never trip this.
    if (implicit_fallback || credential_access.is_inspection())
        && key.trim().is_empty()
        && matches!(entry.auth_style, AuthStyle::Bearer | AuthStyle::Anthropic)
    {
        anyhow::bail!("{}", missing_credentials());
    }
    let codex = match credential_access {
        CredentialAccess::Runtime(snapshot) => snapshot
            .codex_routing(slug, &entry.endpoint, &key)
            .map_err(anyhow::Error::msg)?,
        CredentialAccess::Inspection => resolve_openai_codex_routing_for_inspection(
            slug,
            &entry.endpoint,
            bearer_is_oauth,
            oauth_account_id,
        ),
    };

    Ok(CloudSlugResolution {
        entry,
        effective_model,
        key,
        codex,
    })
}

/// A `<slug>:<model>` BYOK cloud provider as a crate-native [`ChatModel`] — the
/// Native model for every configured cloud auth style, including the managed
/// `OpenhumanJwt` entry (issue #4727 Phase 3).
///
/// Returns `None` unless the role resolves to a **configured** cloud slug. When
/// it does:
/// - `Anthropic` / `None` / plain `Bearer` → crate `OpenAiModel` Chat Completions;
/// - `Bearer` with OpenAI **Codex OAuth** → crate `OpenAiModel` on the Responses
///   API (`with_responses_api_primary`), with the codex account/originator
///   headers, user-agent, `client_version` query param, and `max_output_tokens`
///   omitted (the crate `/v1/responses` support, tinyagents#51);
/// - `OpenhumanJwt` → the crate-native managed backend model.
///
/// The legacy host's rare chat-completions-404 → `/v1/responses` **fallback** for
/// non-codex slugs is not replicated (the crate has responses-*primary*, not
/// fallback); chat completions is the primary path those slugs use in practice.
///
/// The resolution is shared via [`resolve_cloud_slug_for`], so slugs resolve
/// identically to the legacy path; only the wire client differs. The **same**
/// access gate the `Provider` path applies (`enforce_local_only_inference` +
/// `verify_session_active`) runs before building. Temperature rides the per-call
/// `ModelRequest` (managed/local parity; the `@<temp>` suffix still bakes a fixed
/// override).
#[cfg(test)]
pub(super) async fn try_create_cloud_slug_chat_model(
    role: &str,
    config: &Config,
    egress: &EgressLedger,
) -> OptionalChatModelResult {
    let extensions = crate::core::runtime::CoreContext::current_extensions();
    try_create_cloud_slug_chat_model_with_native_tools_and_access(
        role,
        config,
        true,
        egress,
        extensions.as_ref(),
        CredentialAccess::Runtime(&RuntimeCredentialSnapshot::default()),
        None,
        None,
    )
    .await
}

pub(super) async fn try_create_cloud_slug_chat_model_with_native_tools_for(
    role: &str,
    config: &Config,
    native_tool_calling: bool,
    egress: &EgressLedger,
    extensions: Option<&crate::core::slots::Extensions>,
) -> OptionalChatModelResult {
    try_create_cloud_slug_chat_model_with_native_tools_and_access(
        role,
        config,
        native_tool_calling,
        egress,
        extensions,
        CredentialAccess::Runtime(&RuntimeCredentialSnapshot::default()),
        None,
        None,
    )
    .await
}

pub(super) async fn try_create_cloud_slug_chat_model_for_inspection_with_identity(
    role: &str,
    config: &Config,
    extensions: Option<&crate::core::slots::Extensions>,
) -> CloudSlugInspectionChatModelResult {
    let mut identity = None;
    let mut resolved = provider_for_role(role, config);
    if resolved.trim().is_empty() || resolved.trim() == "cloud" {
        resolved = resolve_primary_cloud_provider_string(config);
    }
    let inspection_credential = if let Some((slug, _)) = resolved.split_once(':') {
        let entry = config
            .cloud_providers
            .iter()
            .find(|entry| entry.slug == slug.trim());
        if entry.is_some_and(|entry| entry.auth_style != AuthStyle::OpenhumanJwt) {
            match super::credentials::lookup_key_for_slug_read_only(slug.trim(), config).await {
                Ok(credential) => Some(credential),
                Err(error) => return Some(Err(error)),
            }
        } else {
            None
        }
    } else {
        None
    };
    let result = try_create_cloud_slug_chat_model_from_string_with_native_tools_and_access(
        role,
        &resolved,
        config,
        true,
        &EgressLedger::none(),
        extensions,
        CredentialAccess::Inspection,
        inspection_credential.as_ref(),
        Some(&mut identity),
    )
    .await?;
    Some(result.and_then(|(model, model_id)| {
        let (endpoint, credential, wire_model, output_budget) = identity.ok_or_else(|| {
            anyhow::anyhow!("inspection route did not expose its frozen wire identity")
        })?;
        if wire_model != model_id {
            anyhow::bail!("inspection route model identity did not match its constructed model");
        }
        Ok((model, model_id, endpoint, credential, output_budget))
    }))
}

pub(super) async fn try_create_cloud_slug_chat_model_with_native_tools_and_access(
    role: &str,
    config: &Config,
    native_tool_calling: bool,
    egress: &EgressLedger,
    extensions: Option<&crate::core::slots::Extensions>,
    credential_access: CredentialAccess<'_>,
    inspection_credential: Option<&super::credentials::ReadOnlyCloudCredential>,
    inspection_identity: Option<
        &mut Option<(String, String, String, InspectionOutputBudgetCapability)>,
    >,
) -> OptionalChatModelResult {
    // Resolve the role's provider string, expanding the empty / "cloud" sentinel
    // to the primary cloud target.
    let mut resolved = provider_for_role(role, config);
    if resolved.trim().is_empty() || resolved.trim() == "cloud" {
        resolved = resolve_primary_cloud_provider_string(config);
    }
    try_create_cloud_slug_chat_model_from_string_with_native_tools_and_access(
        role,
        &resolved,
        config,
        native_tool_calling,
        egress,
        extensions,
        credential_access,
        inspection_credential,
        inspection_identity,
    )
    .await
}

pub(super) async fn try_create_cloud_slug_chat_model_from_string(
    role: &str,
    provider: &str,
    config: &Config,
    egress: &EgressLedger,
) -> OptionalChatModelResult {
    try_create_cloud_slug_chat_model_from_string_with_native_tools(
        role, provider, config, true, egress,
    )
    .await
}

pub(super) async fn try_create_cloud_slug_chat_model_from_string_with_native_tools(
    role: &str,
    provider: &str,
    config: &Config,
    native_tool_calling: bool,
    egress: &EgressLedger,
) -> OptionalChatModelResult {
    let extensions = crate::core::runtime::CoreContext::current_extensions();
    try_create_cloud_slug_chat_model_from_string_with_native_tools_and_access(
        role,
        provider,
        config,
        native_tool_calling,
        egress,
        extensions.as_ref(),
        CredentialAccess::Runtime(&RuntimeCredentialSnapshot::default()),
        None,
        None,
    )
    .await
}

pub(super) async fn try_create_cloud_slug_chat_model_from_string_with_native_tools_for(
    role: &str,
    provider: &str,
    config: &Config,
    native_tool_calling: bool,
    egress: &EgressLedger,
    extensions: Option<&crate::core::slots::Extensions>,
) -> OptionalChatModelResult {
    try_create_cloud_slug_chat_model_from_string_with_native_tools_and_access(
        role,
        provider,
        config,
        native_tool_calling,
        egress,
        extensions,
        CredentialAccess::Runtime(&RuntimeCredentialSnapshot::default()),
        None,
        None,
    )
    .await
}

pub(super) async fn try_create_cloud_slug_chat_model_from_string_with_native_tools_and_access(
    role: &str,
    provider: &str,
    config: &Config,
    native_tool_calling: bool,
    egress: &EgressLedger,
    extensions: Option<&crate::core::slots::Extensions>,
    credential_access: CredentialAccess<'_>,
    inspection_credential: Option<&super::credentials::ReadOnlyCloudCredential>,
    mut inspection_identity: Option<
        &mut Option<(String, String, String, InspectionOutputBudgetCapability)>,
    >,
) -> OptionalChatModelResult {
    let p = provider.trim().to_string();

    // Only the "<slug>:<model>[@temp]" cloud form routes here. The managed
    // backend, BYOK-incomplete sentinel, and bespoke subprocess providers
    // (claude-code / claude_agent_sdk) are handled on the `Provider` path.
    if p == PROVIDER_OPENHUMAN
        || p == BYOK_INCOMPLETE_SENTINEL
        || p == CLAUDE_AGENT_SDK_PROVIDER
        || p.starts_with(CLAUDE_AGENT_SDK_PREFIX)
        || p.starts_with(tinyagents_harness::providers::claude_code::PROVIDER_PREFIX)
    {
        return None;
    }
    let colon = p.find(':')?;
    let slug = p[..colon].trim().to_string();
    if slug.is_empty() {
        return None;
    }
    let (raw_model, temperature_override) = split_model_and_temperature(&p[colon + 1..]);
    // Not a configured cloud slug → let the `Provider` path surface the precise
    // "no cloud provider configured" error rather than silently no-op'ing.
    if !config.cloud_providers.iter().any(|e| e.slug == slug) {
        return None;
    }

    // Preserve the `Provider` path's gate for custom/cloud providers.
    if let Err(e) = enforce_local_only_inference(role, &p) {
        return Some(Err(e));
    }
    if let CredentialAccess::Runtime(snapshot) = credential_access {
        #[cfg(not(test))]
        if let Err(e) = super::access_gates::verify_session_active_with_snapshot(config, snapshot) {
            return Some(Err(e));
        }
    }

    let CloudSlugResolution {
        entry,
        effective_model,
        key,
        codex,
    } = match resolve_cloud_slug_with_access(
        role,
        &slug,
        &raw_model,
        config,
        extensions,
        credential_access,
        inspection_credential,
    ) {
        Ok(r) => r,
        Err(e) => return Some(Err(e)),
    };

    // Every configured cloud slug builds a crate-native model through the
    // registry (`cloud_spec`). OpenhumanJwt delegates to the managed backend
    // model; Codex OAuth routes to the Responses API with its headers / UA /
    // query; every other Bearer/Anthropic/None slug uses Chat Completions (or
    // the Anthropic Messages adapter for a first-party native-tools endpoint).
    let mut extra_headers: Vec<(String, String)> = config
        .ephemeral_route
        .as_ref()
        .filter(|route| {
            slug == crate::config::schema::ephemeral_route::EPHEMERAL_ROUTE_SLUG
                && route.endpoint == entry.endpoint
        })
        .map(|route| route.headers.clone())
        .unwrap_or_default();
    let default_provider_options = config
        .ephemeral_route
        .as_ref()
        .filter(|route| {
            slug == crate::config::schema::ephemeral_route::EPHEMERAL_ROUTE_SLUG
                && route.endpoint == entry.endpoint
        })
        .and_then(|route| route.provider_options.clone());
    let mut extra_query_params: Vec<(String, String)> = Vec::new();
    let mut user_agent: Option<String> = None;
    let mut codex_route: Option<&tinyinference_llm::providers::openai::codex::OpenAiCodexRouting> =
        None;

    let anthropic_native = entry.auth_style == AuthStyle::Anthropic
        && native_tool_calling
        && endpoint_is_anthropic_messages(&entry.endpoint);
    match entry.auth_style {
        AuthStyle::Anthropic if !anthropic_native => {
            // Anthropic's OpenAI-compatibility endpoint does not support prompt
            // caching, so native tool calling routes to the Messages adapter
            // (which places `cache_control` breakpoints). Text mode
            // (prompt-guided tools) is only implemented on the Chat
            // Completions adapter, so that rare case keeps the compat path.
            log::debug!(
                "[providers][chat-factory] slug={slug} auth_style=anthropic native_tools=false → OpenAI-compatible text-mode path (no prompt caching)"
            );
        }
        AuthStyle::Anthropic | AuthStyle::None => {}
        style
            if crate::config::schema::cloud_providers::is_hosted_auth_style_for(
                extensions, style,
            ) =>
        {
            if let CredentialAccess::Runtime(snapshot) = credential_access {
                #[cfg(not(test))]
                if !snapshot.backend_session_active() {
                    return Some(Err(anyhow::anyhow!(
                        "SESSION_EXPIRED: no backend session — sign in to use OpenHuman"
                    )));
                }
            }
            let model_override =
                (!effective_model.trim().is_empty()).then_some(effective_model.as_str());
            return Some(match credential_access {
                CredentialAccess::Runtime(_) => managed_chat_model_for(
                    extensions,
                    role,
                    config,
                    model_override,
                    None,
                    native_tool_calling,
                    None,
                    egress,
                ),
                CredentialAccess::Inspection => {
                    match super::managed_backend::managed_chat_model_build_for_inspection(
                        extensions,
                        role,
                        config,
                        model_override,
                    )
                    .await
                    {
                        Ok(build) => {
                            if let Some(identity) = inspection_identity.as_deref_mut() {
                                *identity = Some((
                                    build.endpoint.clone(),
                                    build.credential.clone(),
                                    build.model_id.clone(),
                                    InspectionOutputBudgetCapability::Bounded,
                                ));
                            }
                            Ok((build.model, build.model_id))
                        }
                        Err(error) => Err(error),
                    }
                }
            });
        }
        AuthStyle::Bearer => {
            // The codex routing may re-target the endpoint (OAuth backend).
            if let Some(account_id) = codex.account_id.as_deref() {
                extra_headers.push((
                    OPENAI_CODEX_ACCOUNT_HEADER.to_string(),
                    account_id.to_string(),
                ));
            }
            if codex.using_oauth {
                // Codex OAuth → Responses API primary + the codex request shape.
                extra_headers.push((
                    OPENAI_CODEX_ORIGINATOR_HEADER.to_string(),
                    OPENAI_CODEX_ORIGINATOR.to_string(),
                ));
                user_agent = Some(openai_codex_user_agent(
                    "OpenHuman",
                    env!("CARGO_PKG_VERSION"),
                ));
                extra_query_params
                    .push(("client_version".to_string(), openai_codex_client_version()));
            }
            codex_route = Some(&codex);
        }
        AuthStyle::OpenhumanJwt => {
            return Some(Err(anyhow::anyhow!(
                "hosted cloud authentication is unavailable without a registered host provider"
            )));
        }
    }

    // Egress spine (privacy epic S2, #4436): committed to a BYOK cloud slug here
    // — past the managed/bespoke returns and the access
    // gates, so this constructs. Disclose as external. Single cloud chokepoint
    // for every cloud ChatModel/turn entry.
    crate::security::egress::emit_external_transfer_in(
        egress,
        crate::security::egress::EgressDescriptor::inference(&slug, &effective_model, true),
    );

    let http = match config
        .cloud_provider_ca_certs
        .get(&slug)
        .filter(|pem| !pem.is_empty())
    {
        Some(pem) => {
            let service_key = if anthropic_native {
                "provider.anthropic"
            } else {
                "provider.compatible"
            };
            match crate::util::tls::client_with_ca_bundle(pem, service_key) {
                Ok(http) => Some(http),
                Err(error) => return Some(Err(anyhow::anyhow!(error))),
            }
        }
        None => None,
    };
    let auth_style = cloud_spec::registry_auth_style(entry.auth_style)
        .expect("managed JWT entries returned above");
    let chat = cloud_spec::build_cloud_model(cloud_spec::CloudBuild {
        spec: cloud_spec::cloud_provider_spec(entry, &effective_model),
        auth_style,
        key: key.as_str(),
        temperature_override,
        temperature_unsupported_models: config.temperature_unsupported_models.as_slice(),
        native_tool_calling,
        default_provider_options,
        extra_headers: extra_headers.as_slice(),
        extra_query_params: extra_query_params.as_slice(),
        user_agent: user_agent.as_deref(),
        codex: codex_route,
        http,
    });
    if let Some(identity) = inspection_identity {
        let credential = if entry.auth_style == AuthStyle::None {
            String::new()
        } else {
            key.clone()
        };
        let output_budget = inspection_output_budget_capability(&codex);
        // Doctor's live probe deduplicates on this identity, so it carries
        // everything else that shapes the request: two aliases with the same
        // endpoint, key and model but another auth style, header or CA bundle
        // are two different requests.
        let transport = format!(
            "{:?}\u{1f}{extra_headers:?}\u{1f}{extra_query_params:?}\u{1f}{user_agent:?}\u{1f}{:?}",
            entry.auth_style,
            config.cloud_provider_ca_certs.get(&slug),
        );
        *identity = Some((
            format!("{}\u{1f}{transport}", codex.endpoint),
            credential,
            effective_model.clone(),
            output_budget,
        ));
    }
    Some(chat.map(|chat| (chat, effective_model)))
}
