use crate::config::Config;

/// Which agent-turn roles the incoming patch pinned explicitly.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct ExplicitRolePins {
    pub(super) chat: bool,
    pub(super) reasoning: bool,
    pub(super) agentic: bool,
    pub(super) coding: bool,
}

/// The `cloud_providers` slug [`complete_byok_route`] registers under.
///
/// Distinct from `ephemeral_route::EPHEMERAL_ROUTE_SLUG`: that one is
/// in-memory only and must never be persisted, while this entry exists
/// precisely to be saved.
pub(super) const BYOK_INFERENCE_SLUG: &str = "byok-inference";

/// Two endpoints are the same route if they differ only by trailing slashes.
fn same_endpoint(a: &str, b: &str) -> bool {
    a.trim().trim_end_matches('/') == b.trim().trim_end_matches('/')
}

/// Make an `inference_url` + `api_key` pair actually route.
///
/// Setting those two is the documented way to point inference at a custom
/// OpenAI-compatible endpoint ("When set together with `api_key`, inference
/// goes direct to this URL instead of the OpenHuman backend"). It did not work:
/// `provider_for_role` resolves through `cloud_providers`, never through
/// `inference_url`, so the save succeeded and the *next turn* died with
///
/// ```text
/// [chat-factory] BYOK_INCOMPLETE: inference_url is set to a custom/direct
/// endpoint (…) but no matching cloud_providers entry was found for role 'chat'
/// ```
///
/// — a failure in a different subsystem, one call later, for a write the API
/// accepted. The caller had to also hand-build the provider entry and pin four
/// roles to `<slug>:<model>`, which is not what the field promises and is not
/// discoverable from the error.
///
/// So complete the statement here, the same way
/// [`ephemeral_route::apply`](crate::config::schema::ephemeral_route::apply)
/// completes it for a single call: register the endpoint as a provider and pin
/// the four roles an agent turn runs on.
///
/// Deliberately conservative:
/// - does nothing unless BOTH `inference_url` and `api_key` are non-blank — an
///   endpoint with no credential is a partial statement, and guessing the other
///   half is how a turn ends up somewhere the caller did not ask for;
/// - does nothing when an entry already matches the endpoint, so a
///   hand-configured provider is never overwritten;
/// - needs a resolved `default_model`, because the provider grammar is
///   `<slug>:<model>` and pinning a role to `<slug>:` trades a working default
///   for a resolution failure;
/// - leaves any role the same patch pinned explicitly, and any role already
///   pinned to something other than the managed default, alone.
pub(super) fn complete_byok_route(config: &mut Config, explicit: &ExplicitRolePins) {
    use crate::config::schema::cloud_providers::{AuthStyle, CloudProviderCreds};

    let Some(endpoint) = config
        .inference_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
    else {
        return;
    };
    let has_key = config
        .api_key
        .as_deref()
        .map(str::trim)
        .is_some_and(|value| !value.is_empty());
    if !has_key {
        return;
    }

    let existing = config
        .cloud_providers
        .iter()
        .find(|entry| same_endpoint(&entry.endpoint, &endpoint))
        .map(|entry| entry.slug.trim().to_string());

    let Some(model) = config
        .default_model
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
    else {
        if existing.is_none() {
            log::warn!(
                "[config][byok] inference_url is set with a key but no default_model is \
                 resolved — cannot complete the BYOK route; turns will report BYOK_INCOMPLETE"
            );
        }
        return;
    };

    let slug = match existing {
        Some(slug) => slug,
        None => {
            log::info!(
                "[config][byok] registering cloud provider '{BYOK_INFERENCE_SLUG}' for the \
                 configured inference_url so agent turns can resolve it"
            );
            config.cloud_providers.push(CloudProviderCreds {
                id: BYOK_INFERENCE_SLUG.to_string(),
                slug: BYOK_INFERENCE_SLUG.to_string(),
                label: "Custom inference endpoint".to_string(),
                endpoint: endpoint.clone(),
                auth_style: AuthStyle::Bearer,
                default_model: Some(model.clone()),
            });
            BYOK_INFERENCE_SLUG.to_string()
        }
    };

    let provider_string = format!("{slug}:{model}");
    // "Unspoken for" means empty or the managed `cloud` sentinel. Anything else
    // is a deliberate choice — a local Ollama role, a second BYOK provider —
    // and repointing it at this endpoint would be exactly the silent
    // repointing this function exists to avoid.
    let unspoken = |value: &Option<String>| {
        value
            .as_deref()
            .map(str::trim)
            .is_none_or(|v| v.is_empty() || v == "cloud")
    };
    let mut pinned: Vec<&str> = Vec::new();
    for (role, pinned_explicitly, slot) in [
        ("chat", explicit.chat, &mut config.chat_provider),
        (
            "reasoning",
            explicit.reasoning,
            &mut config.reasoning_provider,
        ),
        ("agentic", explicit.agentic, &mut config.agentic_provider),
        ("coding", explicit.coding, &mut config.coding_provider),
    ] {
        if pinned_explicitly || !unspoken(slot) {
            continue;
        }
        *slot = Some(provider_string.clone());
        pinned.push(role);
    }
    if !pinned.is_empty() {
        log::info!(
            "[config][byok] pinned role(s) [{}] to '{}' from the configured inference_url",
            pinned.join(", "),
            provider_string
        );
    }
}
