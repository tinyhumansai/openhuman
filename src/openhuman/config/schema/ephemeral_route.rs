//! A per-call inference route that lives only in memory.
//!
//! # Why a config field rather than a parameter
//!
//! An embedding host — the desktop app, or another process driving the core
//! through the embed facade — sometimes knows where a *single* turn should run
//! before the core does: it holds an OpenAI-compatible endpoint and a bearer for
//! it, and wants this turn on that endpoint without changing where the account's
//! other work runs.
//!
//! The obvious spelling, writing `inference_url`/`api_key`/`cloud_providers`
//! through `config_update_model_settings`, is wrong for that: those are
//! persisted, so a caller borrowing the account's inference route for one turn
//! would repoint the operator's whole install and have to remember to put it
//! back. A crash between the two writes leaves the install repointed for good.
//!
//! So the route rides on [`Config`](crate::openhuman::config::Config) — which
//! every resolution step already reads — in a field that is `#[serde(skip)]`.
//! It cannot reach `config.toml` even if a caller does save the config, because
//! there is no field on the wire to save it into. Controllers that accept a
//! route apply it to their own per-call `Config` copy, which is dropped when the
//! call returns.
//!
//! # What it does and does not capture
//!
//! [`apply`] pins the four roles an agent turn actually runs on — chat,
//! reasoning, agentic, coding. It deliberately leaves the background roles
//! (memory, embeddings, heartbeat, learning, subconscious) and `vision` alone:
//! those run tier-specific models a coding endpoint generally cannot serve, and
//! silently sending an embeddings request to a chat model is worse than ignoring
//! the route for that workload.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::cloud_providers::{AuthStyle, CloudProviderCreds};

/// The `cloud_providers` slug [`apply`] registers the route under.
///
/// Hyphenated and prefixed so it cannot collide with a provider slug a user
/// would choose, and deliberately *not* in
/// [`is_slug_reserved`](super::cloud_providers::is_slug_reserved): reserved
/// slugs are re-injected from the stored config on save, which is the opposite
/// of what an in-memory-only entry wants.
pub const EPHEMERAL_ROUTE_SLUG: &str = "ephemeral-route";

/// Where one call's inference should go, and what authenticates it.
///
/// Never serialized — see the module docs. `JsonSchema` is derived only because
/// [`Config`](crate::openhuman::config::Config) derives it for the whole struct.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EphemeralRoute {
    /// OpenAI-compatible base URL, as a `cloud_providers` endpoint would be
    /// spelled (`/chat/completions` is appended to it).
    pub endpoint: String,
    /// The bearer presented to `endpoint`.
    ///
    /// Read back only by
    /// [`lookup_key_for_slug`](crate::openhuman::inference::provider::factory::lookup_key_for_slug)
    /// and only for [`EPHEMERAL_ROUTE_SLUG`], so it cannot be handed to a
    /// provider the caller did not name.
    pub api_key: String,
}

impl EphemeralRoute {
    /// Build a route from a controller's two optional parameters.
    ///
    /// `None` unless both arrive non-blank: an endpoint with no credential and a
    /// credential with no endpoint are both partial statements, and guessing the
    /// missing half is how a turn ends up somewhere the caller did not ask for.
    pub fn from_params(endpoint: Option<String>, api_key: Option<String>) -> Option<Self> {
        let endpoint = endpoint?.trim().to_string();
        let api_key = api_key?.trim().to_string();
        (!endpoint.is_empty() && !api_key.is_empty()).then_some(Self { endpoint, api_key })
    }
}

/// Point `config`'s agent-turn roles at `route`, in memory.
///
/// Registers the route as a `cloud_providers` entry and pins the four roles a
/// turn runs on to `"<slug>:<model>"`, taking the model from `config`'s
/// resolved `default_model` — which a caller passing `model_override` has
/// already set. Any explicit pin on those roles is replaced, because the route
/// *is* the caller's statement about where this turn runs and
/// [`provider_for_role`](crate::openhuman::inference::provider::factory::provider_for_role)
/// consults the pins first.
///
/// A blank `default_model` leaves the roles alone and registers nothing: the
/// slug grammar has no model to carry, and pinning a role to `"<slug>:"` would
/// trade a working default for a "resolved to an empty model id" bail.
pub fn apply(config: &mut crate::openhuman::config::Config, route: EphemeralRoute) {
    let model = config
        .default_model
        .as_deref()
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(str::to_string);
    let Some(model) = model else {
        log::warn!(
            "[config][ephemeral-route] ignoring route to {} — no model resolved for this call",
            crate::openhuman::inference::provider::factory::redact_endpoint(&route.endpoint)
        );
        return;
    };

    // Replace rather than append: a second call on the same `Config` copy is not
    // expected, but two entries sharing a slug would make which one resolves an
    // ordering accident.
    config
        .cloud_providers
        .retain(|entry| entry.slug != EPHEMERAL_ROUTE_SLUG);
    config.cloud_providers.push(CloudProviderCreds {
        // A fixed id, not a generated one: nothing persists this entry and
        // nothing selects it by id (`primary_cloud` is left untouched), so a
        // fresh random id per call would only make the value unpredictable.
        id: EPHEMERAL_ROUTE_SLUG.to_string(),
        slug: EPHEMERAL_ROUTE_SLUG.to_string(),
        label: "Per-call route".to_string(),
        endpoint: route.endpoint.clone(),
        auth_style: AuthStyle::Bearer,
        legacy_type: None,
        default_model: Some(model.clone()),
    });

    let provider_string = format!("{EPHEMERAL_ROUTE_SLUG}:{model}");
    config.chat_provider = Some(provider_string.clone());
    config.reasoning_provider = Some(provider_string.clone());
    config.agentic_provider = Some(provider_string.clone());
    config.coding_provider = Some(provider_string);

    log::info!(
        "[config][ephemeral-route] this call routed to {} model={}",
        crate::openhuman::inference::provider::factory::redact_endpoint(&route.endpoint),
        model
    );

    config.ephemeral_route = Some(route);
}

#[cfg(test)]
#[path = "ephemeral_route_tests.rs"]
mod tests;
