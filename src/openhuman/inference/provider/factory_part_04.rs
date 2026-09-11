
/// Resolve the slug of the cloud-provider entry that represents the legacy
/// direct-inference route — the entry whose endpoint matches the configured
/// custom `inference_url`.
///
/// Top-level `config.api_key` was historically paired with `inference_url`
/// for direct endpoint routing, so it is scoped to this single provider. The
/// `lookup_key_for_slug` fallback uses this to avoid leaking the global key to
/// any other provider slug whose auth-profile lookup returned empty.
fn legacy_inference_slug(config: &Config) -> Option<&str> {
    let inference_url = config
        .inference_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())?;

    if looks_like_openhuman_backend(inference_url) {
        return None;
    }

    let normalized_inference = normalize_endpoint_for_compare(inference_url);
    config
        .cloud_providers
        .iter()
        .find(|entry| {
            !is_openhuman_cloud_entry(entry)
                && normalize_endpoint_for_compare(&entry.endpoint) == normalized_inference
        })
        .map(|entry| entry.slug.as_str())
}

fn cloud_entry_provider_string(
    entry: &crate::openhuman::config::schema::cloud_providers::CloudProviderCreds,
    config: &Config,
) -> String {
    if is_openhuman_cloud_entry(entry) {
        return PROVIDER_OPENHUMAN.to_string();
    }

    let model = entry
        .default_model
        .as_deref()
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .or_else(|| {
            config
                .default_model
                .as_deref()
                .map(str::trim)
                .filter(|model| !model.is_empty())
        })
        .unwrap_or(crate::openhuman::config::DEFAULT_MODEL);

    format!("{}:{model}", entry.slug)
}

fn is_openhuman_cloud_entry(
    entry: &crate::openhuman::config::schema::cloud_providers::CloudProviderCreds,
) -> bool {
    entry.slug == PROVIDER_OPENHUMAN
        || matches!(entry.auth_style, AuthStyle::OpenhumanJwt)
        || looks_like_openhuman_backend(&entry.endpoint)
}

fn normalize_endpoint_for_compare(url: &str) -> String {
    url.trim().trim_end_matches('/').to_ascii_lowercase()
}

fn looks_like_openhuman_backend(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    let without_scheme = lower.split("://").nth(1).unwrap_or(&lower);
    let authority = without_scheme.split('/').next().unwrap_or("");
    let host = authority.split('@').next_back().unwrap_or(authority);
    let host_no_port = host.split(':').next().unwrap_or(host);
    matches!(
        host_no_port,
        "api.openhuman.ai" | "api.tinyhumans.ai" | "staging-api.tinyhumans.ai" | "openhuman"
    ) || host_no_port.ends_with(".openhuman.ai")
        || host_no_port.ends_with(".tinyhumans.ai")
}

/// Parse a `<model>[@<temp>]` tail into `(model, override)`.
///
/// Tolerates whitespace around the components. Returns `temperature = None`
/// when the suffix is absent or unparseable — the model text is taken as-is.
fn split_model_and_temperature(raw: &str) -> (String, Option<f64>) {
    let trimmed = raw.trim();
    if let Some(at_pos) = trimmed.rfind('@') {
        let head = trimmed[..at_pos].trim();
        let tail = trimmed[at_pos + 1..].trim();
        if !head.is_empty() {
            if let Ok(parsed) = tail.parse::<f64>() {
                if parsed.is_finite() {
                    return (head.to_string(), Some(parsed));
                }
            }
        }
    }
    (trimmed.to_string(), None)
}

/// Look up a `cloud_providers` entry by slug and build the provider.
/// The shared resolution for a `<slug>:<model>` cloud provider — the cloud
/// `cloud_providers` entry, the effective model id (with `default_model`
/// fallback + abstract-tier remapping), the resolved API key, and the OpenAI
/// codex-oauth routing shared by every cloud `ChatModel` constructor.
struct CloudSlugResolution<'a> {
    entry: &'a crate::openhuman::config::schema::cloud_providers::CloudProviderCreds,
    effective_model: String,
    key: String,
    codex: crate::openhuman::inference::provider::openai_codex::OpenAiCodexRouting,
}

fn resolve_cloud_slug<'a>(
    role: &str,
    slug: &str,
    model: &str,
    config: &'a Config,
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

    // Resolve effective model: use provided model if non-empty, else fall back
    // to the entry's legacy default_model (if any), else empty → error.
    let mut effective_model = if model.trim().is_empty() {
        entry.default_model.clone().unwrap_or_default()
    } else {
        model.to_string()
    };

    // Guard: if effective_model is still empty after fallback, bail with an
    // actionable error. Sending an empty model string to providers like
    // nvidia-nim causes a 400 "model field is required" — a confusing error
    // that obscures the real cause (missing model in the provider string or
    // unset default_model on the config entry).
    // See https://github.com/tinyhumansai/openhuman/issues/2784.
    //
    // OpenhumanJwt entries are exempt: they always delegate to
    // make_openhuman_backend which derives the model from config.default_model,
    // ignoring whatever effective_model we computed here.
    if entry.auth_style != AuthStyle::OpenhumanJwt && effective_model.trim().is_empty() {
        log::warn!(
            "[nvidia-nim][chat-factory] role={} slug={} resolved to empty model — \
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

    if entry.auth_style != AuthStyle::OpenhumanJwt && is_abstract_tier_model(&effective_model) {
        if let Some(default_model) = entry
            .default_model
            .as_deref()
            .map(str::trim)
            .filter(|m| !m.is_empty() && !is_abstract_tier_model(m))
        {
            log::info!(
                "[providers][chat-factory] role={} slug={} remapping abstract model {} -> {}",
                role,
                slug,
                effective_model,
                default_model
            );
            effective_model = default_model.to_string();
        } else {
            anyhow::bail!(
                "[chat-factory] model '{}' is an abstract tier for role '{}', \
                 but cloud provider slug '{}' has no concrete default_model configured. \
                 Set cloud_providers[].default_model to a provider-native model id (e.g. deepseek-v4-pro).",
                effective_model,
                role,
                slug
            );
        }
    }

    log::info!(
        "[providers][chat-factory] role={} slug={} model={} endpoint_host={}",
        role,
        slug,
        effective_model,
        redact_endpoint(&entry.endpoint)
    );

    // #5146 §2.1: a raw "failed to read API key for slug 'anthropic'" is
    // baffling when the user never configured Anthropic — they set a local
    // Ollama model and this is a background role that fell back to the cloud.
    // Attach the role, and the local chat model that caused the fallback, so
    // the message explains itself and names a concrete remedy.
    // Only an *implicit* fallback is explained as one. A role with its own
    // explicit cloud route can fail key lookup here too, and telling that user
    // their local chat model caused it would be a lie.
    let implicit_fallback = role_uses_implicit_cloud_fallback(role, config);
    let local_chat = if implicit_fallback {
        config.chat_provider.as_deref().filter(|chat| {
            crate::openhuman::inference::local::profile::is_local_provider_string(chat)
        })
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
        super::fallback_diagnostics::missing_provider_credentials_message(role, slug, local_chat)
    };

    let key = lookup_key_for_slug(slug, config)
        .map_err(|e| anyhow::anyhow!("{} (underlying error: {e})", missing_credentials()))?;

    // A readable auth profile with no key for this slug returns `Ok("")`, which
    // would otherwise build a client with an empty bearer and surface as a raw
    // 401 from the provider several layers later — exactly the baffling error
    // this diagnostic exists to replace.
    //
    // Scoped to the *implicit fallback* path deliberately. That is the case the
    // diagnostic is for: a local-chat user whose background role landed on a
    // BYOK slug they never configured. An explicitly routed provider keeps its
    // existing behaviour and is allowed to build without a stored key — callers
    // construct such models to probe or describe a provider before a key is
    // saved, and failing that at construction time would be a behaviour change
    // well beyond this diagnostic.
    //
    // Styles that carry no stored key (`OpenhumanJwt` injects a session JWT
    // downstream, `None` sends no auth header at all) are legitimately blank and
    // never trip this.
    if implicit_fallback
        && key.trim().is_empty()
        && matches!(entry.auth_style, AuthStyle::Bearer | AuthStyle::Anthropic)
    {
        anyhow::bail!("{}", missing_credentials());
    }
    let bearer_is_oauth = slug == "openai" && openai_bearer_is_oauth(config);
    let codex = resolve_openai_codex_routing(config, slug, &entry.endpoint, &key, bearer_is_oauth)
        .map_err(anyhow::Error::msg)?;

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
/// The resolution is shared via [`resolve_cloud_slug`], so slugs resolve
/// identically to the legacy path; only the wire client differs. The **same**
/// access gate the `Provider` path applies (`enforce_local_only_inference` +
/// `verify_session_active`) runs before building. Temperature rides the per-call
/// `ModelRequest` (managed/local parity; the `@<temp>` suffix still bakes a fixed
/// override).
fn try_create_cloud_slug_chat_model(role: &str, config: &Config) -> OptionalChatModelResult {
    try_create_cloud_slug_chat_model_with_native_tools(role, config, true)
}

fn try_create_cloud_slug_chat_model_with_native_tools(
    role: &str,
    config: &Config,
    native_tool_calling: bool,
) -> OptionalChatModelResult {
    // Resolve the role's provider string, expanding the empty / "cloud" sentinel
    // to the primary cloud target.
    let mut resolved = provider_for_role(role, config);
    if resolved.trim().is_empty() || resolved.trim() == "cloud" {
        resolved = resolve_primary_cloud_provider_string(config);
    }
    try_create_cloud_slug_chat_model_from_string_with_native_tools(
        role,
        &resolved,
        config,
        native_tool_calling,
    )
}

fn try_create_cloud_slug_chat_model_from_string(
    role: &str,
    provider: &str,
    config: &Config,
) -> OptionalChatModelResult {
    try_create_cloud_slug_chat_model_from_string_with_native_tools(role, provider, config, true)
}

fn try_create_cloud_slug_chat_model_from_string_with_native_tools(
    role: &str,
    provider: &str,
    config: &Config,
    native_tool_calling: bool,
) -> OptionalChatModelResult {
    let p = provider.trim().to_string();

    // Only the "<slug>:<model>[@temp]" cloud form routes here. The managed
    // backend, BYOK-incomplete sentinel, and bespoke subprocess providers
    // (claude-code / claude_agent_sdk) are handled on the `Provider` path.
    if p == PROVIDER_OPENHUMAN
        || p == BYOK_INCOMPLETE_SENTINEL
        || p == CLAUDE_AGENT_SDK_PROVIDER
        || p.starts_with(CLAUDE_AGENT_SDK_PREFIX)
        || p.starts_with(crate::openhuman::inference::provider::claude_code::PROVIDER_PREFIX)
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
    #[cfg(not(test))]
    if let Err(e) = verify_session_active(config) {
        return Some(Err(e));
    }

    let CloudSlugResolution {
        entry,
        effective_model,
        key,
        codex,
    } = match resolve_cloud_slug(role, &slug, &raw_model, config) {
        Ok(r) => r,
        Err(e) => return Some(Err(e)),
    };

    // Every configured cloud slug builds a crate-native model. OpenhumanJwt
    // delegates to the managed backend model; Codex OAuth routes to the
    // Responses API with its headers / UA / query; every other
    // Bearer/Anthropic/None slug uses Chat Completions (its primary path — the
    // legacy host's rare 404 → `/v1/responses` fallback for non-codex slugs is
    // not replicated).
    let mut endpoint = entry.endpoint.clone();
    let mut extra_headers: Vec<(String, String)> = Vec::new();
    let mut extra_query_params: Vec<(String, String)> = Vec::new();
    let mut user_agent: Option<String> = None;
    let mut responses_api_primary = false;
    let mut responses_omit_max_output_tokens = false;

    let auth = match entry.auth_style {
        AuthStyle::Anthropic => CompatAuthStyle::Anthropic,
        AuthStyle::None => CompatAuthStyle::None,
        AuthStyle::OpenhumanJwt => {
            #[cfg(not(test))]
            if let Err(error) = verify_backend_session_active(config) {
                return Some(Err(error));
            }
            let model_override =
                (!effective_model.trim().is_empty()).then_some(effective_model.as_str());
            let (backend, pinned_model) =
                match resolve_managed_backend_with_model_override(role, config, model_override) {
                    Ok(result) => result,
                    Err(error) => return Some(Err(error)),
                };
            return Some(Ok((
                Arc::new(backend.with_native_tool_calling(native_tool_calling)),
                pinned_model,
            )));
        }
        AuthStyle::Bearer => {
            // The codex routing may re-target the endpoint (OAuth backend).
            endpoint = codex.endpoint.clone();
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
                user_agent = Some(openai_codex_user_agent());
                extra_query_params
                    .push(("client_version".to_string(), openai_codex_client_version()));
                responses_api_primary = true;
                responses_omit_max_output_tokens = true;
            }
            CompatAuthStyle::Bearer
        }
    };

    // Egress spine (privacy epic S2, #4436): committed to a BYOK cloud slug here
    // — past the managed/bespoke returns and the access
    // gates, so this constructs. Disclose as external. Single cloud chokepoint
    // for every cloud ChatModel/turn entry.
    crate::openhuman::security::egress::emit_external_transfer(
        crate::openhuman::security::egress::EgressDescriptor::inference(
            &slug,
            &effective_model,
            true,
        ),
    );

    let unsupported = config.temperature_unsupported_models.clone();
    let chat =
        super::crate_openai::build_crate_openai_model(super::crate_openai::CrateOpenAiConfig {
            provider_name: slug.as_str(),
            endpoint: endpoint.as_str(),
            api_key: key.as_str(),
            auth_style: auth,
            model: effective_model.as_str(),
            temperature_unsupported_models: unsupported.as_slice(),
            temperature_override,
            // Cloud OpenAI-compatible providers accept a `system` role — no merge
            // (parity with the crate-native OpenAI model defaults).
            merge_system_into_user: false,
            extra_headers: extra_headers.as_slice(),
            native_tool_calling: Some(native_tool_calling),
            vision: None,
            default_provider_options: None,
            responses_api_primary,
            responses_omit_max_output_tokens,
            extra_query_params: extra_query_params.as_slice(),
            user_agent: user_agent.as_deref(),
        });
    Some(Ok((chat, effective_model)))
}

/// Whether the openai bearer that [`lookup_key_for_slug`] resolves is an OAuth
/// (Codex-subscription) credential rather than a standard API key.
///
/// OAuth and API-key credentials share the same `provider:openai` profile store
/// and differ only by [`AuthProfileKind`], so the bearer *string* cannot reveal
/// its source — which is exactly why the old `access_token == bearer_key` compare
/// broke under token rotation (#5353). This mirrors `lookup_key_for_slug`'s
/// precedence (`provider:openai`, then the legacy bare `openai`) and reports the
/// *kind* of the profile that would win. With no stored openai profile carrying a
/// credential, the only bearer source is the OAuth fallback, so a present OAuth
/// credential means the bearer is OAuth.
pub(crate) fn openai_bearer_is_oauth(config: &Config) -> bool {
    use crate::openhuman::security::credentials::profiles::AuthProfileKind;

    let auth = AuthService::from_config(config);
    for provider in [auth_key_for_slug("openai"), "openai".to_string()] {
        if let Ok(Some(profile)) = auth.get_profile(&provider, None) {
            // A profile with an empty credential is skipped by
            // `lookup_key_for_slug`, so fall through to the next precedence level.
            let has_credential = match profile.kind {
                AuthProfileKind::Token => profile
                    .token
                    .as_deref()
                    .is_some_and(|t| !t.trim().is_empty()),
                AuthProfileKind::OAuth => profile
                    .token_set
                    .as_ref()
                    .is_some_and(|t| !t.access_token.trim().is_empty()),
            };
            if has_credential {
                return matches!(profile.kind, AuthProfileKind::OAuth);
            }
        }
    }
    // No stored openai profile with a credential → the bearer, if any, comes from
    // the OAuth fallback (`lookup_openai_bearer_token`).
    crate::openhuman::inference::openai_oauth::lookup_openai_oauth_credentials(config)
        .ok()
        .flatten()
        .is_some()
}

/// Fetch the bearer token for a slug from the workspace `auth-profiles.json`.
///
/// Tries `provider:<slug>` first (new key format), then the bare `<slug>`
/// (legacy format where keys were stored as `"openai"`, `"anthropic"`, etc.).
/// Missing or empty keys return `Ok(String::new())` — callers treat that as
/// "no auth", which surfaces an authentication error at first call rather than
/// at factory build time.
pub fn lookup_key_for_slug(slug: &str, config: &Config) -> anyhow::Result<String> {
    // Ahead of the stored profiles, and scoped to the one slug the per-call
    // route registers. A caller that named an endpoint and a bearer for this
    // turn has said where the credential comes from, and there is nothing on
    // disk to find for a slug that exists only in this `Config` copy. Scoping it
    // by slug is what keeps the bearer from reaching a provider the caller never
    // named — the same containment the legacy `config.api_key` fallback below
    // gets from `legacy_inference_slug`.
    if slug == crate::openhuman::config::schema::EPHEMERAL_ROUTE_SLUG {
        if let Some(route) = config.ephemeral_route.as_ref() {
            log::debug!(
                "[providers][chat-factory] auth lookup slug={} key_present={} (per-call route)",
                slug,
                !route.api_key.trim().is_empty()
            );
            return Ok(route.api_key.trim().to_string());
        }
    }

    let auth = AuthService::from_config(config);
    // Try new-style key first.
    let new_key = auth_key_for_slug(slug);
    if let Ok(Some(k)) = auth.get_provider_bearer_token(&new_key, None) {
        if !k.is_empty() {
            log::debug!(
                "[providers][chat-factory] auth lookup slug={} key_present=true (new-style)",
                slug
            );
            return Ok(k);
        }
    }
    // Fall back to legacy bare slug.
    let key = auth
        .get_provider_bearer_token(slug, None)
        .map_err(|e| {
            anyhow::anyhow!(
                "[chat-factory] failed to read API key for slug '{}': {}",
                slug,
                e
            )
        })?
        .unwrap_or_default();
    if !key.is_empty() {
        log::debug!(
            "[providers][chat-factory] auth lookup slug={} key_present=true",
            slug
        );
        return Ok(key);
    }

    // OAuth fallback for `openai` runs only after standard API-key resolution
    // returns empty, so env/audit/metrics in the standard path always execute
    // and the OAuth path never silently bypasses provider-agnostic logic.
    if slug == "openai" {
        match crate::openhuman::inference::openai_oauth::lookup_openai_bearer_token(config) {
            Ok(Some(token)) if !token.is_empty() => {
                log::debug!(
                    "[providers][chat-factory] auth lookup slug={} key_present=true (oauth)",
                    slug
                );
                return Ok(token);
            }
            Ok(_) => {}
            Err(e) => {
                return Err(anyhow::anyhow!(
                    "[chat-factory] openai oauth lookup failed: {e}"
                ));
            }
        }
    }

    // Fallback: read from top-level config.api_key (direct config.toml api_key).
    // This handles the case where a key was set in config.toml but not saved
    // through the UI into auth-profiles.json.
    //
    // Scoped to the legacy direct-inference provider only — the cloud-provider
    // slug whose endpoint matches `config.inference_url`. `config.api_key` was
    // historically paired with `inference_url` for direct endpoint routing, so
    // an unscoped fallback would leak this global key to any other provider
    // whose auth-profile lookup returned empty (cross-provider credential leak
    // flagged by CodeRabbit + maintainers on #2724).
    if legacy_inference_slug(config) == Some(slug) {
        if let Some(config_key) = config.api_key.as_ref() {
            if !config_key.trim().is_empty() {
                log::debug!(
                    "[providers][chat-factory] auth lookup slug={} key_present=true (config.toml fallback for legacy inference_url)",
                    slug
                );
                return Ok(config_key.trim().to_string());
            }
        }
    }

    log::debug!(
        "[providers][chat-factory] auth lookup slug={} key_present=false",
        slug
    );
    Ok(String::new())
}

/// Return a safe-to-log representation of a URL endpoint: `scheme://host` only.
pub fn redact_endpoint(url: &str) -> String {
    let trimmed = url.trim();
    if let Some(rest) = trimmed.split_once("://") {
        let scheme = rest.0;
        let authority = rest.1.split('/').next().unwrap_or("");
        let host = authority.split('@').next_back().unwrap_or(authority);
        let host_no_query = host.split('?').next().unwrap_or(host);
        return format!("{}://{}", scheme, host_no_query);
    }
    "<endpoint>".to_string()
}
