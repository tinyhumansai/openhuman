//! The managed model catalog: the `list_configured_models` listing for the
//! managed provider and the model-limits discovery request, both against the
//! hosted backend's `/openai/v1/models?catalog=openrouter`.
//!
//! `?catalog=openrouter` is required: without it the backend returns only the
//! curated legacy list. The catalog listing is gated server-side and returns an
//! empty set when the passthrough is off, so this degrades to "no models"
//! rather than an error on a backend that has not enabled it.

use openhuman_embed::__host::config::Config;
use openhuman_embed::__host::core::Outcome;
use openhuman_embed::__host::credentials::backend_credential::{
    publish_local_session_expiry, resolve_backend_credential, BackendCredential,
};
use openhuman_embed::__host::credentials::session_state::{
    classify_session_token, load_app_session_profile, session_token_from_profile, SessionTokenCheck,
};
use openhuman_embed::__host::inference_registry::{
    BoxFuture, CatalogEntry, CatalogHostRequest, CatalogLimits, CatalogListing, CatalogRequest,
    InspectionCatalogError, InspectionCatalogListing, InspectionCatalogRequest,
    PreparedInspectionCatalog, ProviderContext,
};
use openhuman_embed::__host::util::url::join_url;
use std::sync::Arc;
use tinyinference_llm::catalog::{append_query_param, parse_models_response};
use tinyinference_llm::model::discover::DiscoveryRequest;
use tinyinference_llm::providers::tinyhumans::{
    discovery_request, list_models as fetch_model_catalog,
};

use super::PROVIDER_OPENHUMAN;
use crate::backend::headers::attribution_headers;
use crate::backend::url::{effective_api_url, effective_backend_api_url};
use crate::client::endpoint::{
    is_managed_endpoint_for_api_key, is_safe_endpoint_for_managed_bearer,
};

const MODELS_PATH: &str = "/openai/v1/models";

/// The registry entry for the managed catalog.
pub(super) fn entry() -> CatalogEntry<ProviderContext> {
    let list: CatalogListing<ProviderContext> = Arc::new(
        |request: CatalogRequest<ProviderContext>| -> BoxFuture<Result<serde_json::Value, String>> {
            Box::pin(async move {
                let context = request.context;
                let host = context.catalog.as_ref().expect("catalog host context");
                let outcome = list_models(
                    &context.config,
                    CatalogHostRequest {
                        provider_key: host.request.provider_key.clone(),
                        client: host.request.client.clone(),
                    },
                )
                .await?;
                host.logs
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .extend(outcome.logs);
                Ok(outcome.value)
            })
        },
    );
    let limits: CatalogLimits<ProviderContext> = Arc::new(|request| {
        Box::pin(async move { limits_request(&request.model, &request.context.config).await })
    });
    CatalogEntry {
        list,
        limits: Some(limits),
    }
}

/// Inspection-only variant: freezes a read-only session credential before
/// the core consults its cache. This deliberately does not call `list_models`
/// or any mutable session/catalog path.
pub(super) fn inspection_entry() -> InspectionCatalogListing {
    Arc::new(|config: Config, request: InspectionCatalogRequest| {
        Box::pin(async move {
            use openhuman_embed::__host::credentials::session_state::{
                check_session_backend, classify_session_token, load_app_session_profile_read_only,
                SessionTokenCheck,
            };

            let profile = load_app_session_profile_read_only(&config)
                .await
                .map_err(|_| InspectionCatalogError::Unavailable)?;
            check_session_backend(&config, profile.as_ref())
                .map_err(|_| InspectionCatalogError::Unavailable)?;
            let session = match classify_session_token(profile.as_ref(), chrono::Utc::now()) {
                SessionTokenCheck::Live(token) => Some(token),
                SessionTokenCheck::Absent | SessionTokenCheck::Expired => None,
            };
            let provider_key = request.provider_key.trim();
            let api_key = if provider_key.is_empty() {
                openhuman_embed::__host::credentials::api_key::get_api_key_read_only(&config)
                    .await
                    .map_err(|_| InspectionCatalogError::Unavailable)?
            } else {
                Some(provider_key.to_owned())
            };
            let credential = session
                .filter(|_| is_safe_endpoint_for_managed_bearer(&catalog_url(&config)))
                .or_else(|| {
                    api_key.filter(|_| is_managed_endpoint_for_api_key(&catalog_url(&config)))
                });
            let url = catalog_url(&config);
            if credential.is_none() {
                return Err(InspectionCatalogError::Unavailable);
            }
            let client = request.client;
            let frozen = credential.clone();
            let fetch_url = url.clone();
            let fetch = Box::pin(async move {
                let mut get = client.get(&fetch_url);
                if let Some(token) = frozen.as_deref() {
                    get = get.header("Authorization", format!("Bearer {token}"));
                    if let Ok(headers) = attribution_headers() {
                        get = get.headers(headers);
                    }
                }
                let response = get.send().await.map_err(|error| {
                    if error.is_timeout() {
                        InspectionCatalogError::Failure
                    } else {
                        InspectionCatalogError::Unavailable
                    }
                })?;
                let status = response.status();
                if status == reqwest::StatusCode::NOT_FOUND {
                    return Err(InspectionCatalogError::NoCatalog);
                }
                if matches!(
                    status,
                    reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN
                ) {
                    return Err(InspectionCatalogError::AuthOrAccess);
                }
                if !status.is_success() {
                    return Err(InspectionCatalogError::Failure);
                }
                let body = response
                    .json::<serde_json::Value>()
                    .await
                    .map_err(|_| InspectionCatalogError::Failure)?;
                if body.get("error").is_some() {
                    return Err(InspectionCatalogError::Failure);
                }
                let models =
                    parse_models_response(&body).map_err(|_| InspectionCatalogError::Failure)?;
                Ok(Outcome::new(
                    serde_json::json!({"models": models}),
                    Vec::new(),
                ))
            });
            Ok(PreparedInspectionCatalog::new(
                PROVIDER_OPENHUMAN,
                &url,
                credential.as_deref(),
                fetch,
            ))
        })
    })
}

fn empty_catalog(reason: &str) -> Result<Outcome<serde_json::Value>, String> {
    Ok(Outcome::new(
        serde_json::json!({ "models": Vec::<serde_json::Value>::new() }),
        vec![format!("{reason}; managed catalog is empty")],
    ))
}

fn catalog_url(config: &Config) -> String {
    append_query_param(
        &join_url(&effective_backend_api_url(&config.api_url), MODELS_PATH),
        "catalog",
        "openrouter",
    )
}

/// Whether a non-2xx from the listing reads as "signed out": a 401 on a request
/// that actually carried the app session token. A 401 without one describes the
/// request, not the session.
pub(super) fn managed_401_means_signed_out(status: u16, session_attached: bool) -> bool {
    openhuman_embed::__host::core::session_expiry::backend_status_means_session_expired(
        status,
        !session_attached,
    )
}

/// What authenticates a listing request: the live session token, or the
/// entry's provider-scoped key when there is no live session.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum ListingAuth {
    /// The app session token.
    Session(String),
    /// A key stored for the entry's slug (a self-hosted entry that carries one).
    ProviderKey(String),
    /// Nothing to authenticate with: the catalog is empty, for this reason.
    Empty(&'static str),
}

/// Pick the listing credential. A live session wins; without one the entry's
/// provider-scoped key still authenticates, and only when neither exists does
/// the catalog degrade to empty.
pub(super) async fn listing_auth(
    check: SessionTokenCheck,
    selected_token: Option<&str>,
    provider_key: &str,
) -> ListingAuth {
    let provider_key = provider_key.trim();
    match check {
        SessionTokenCheck::Live(token) => ListingAuth::Session(token),
        SessionTokenCheck::Expired => {
            // `require_live_session_token` announced this; without it an expired
            // token stays in the store with nothing prompting a re-auth.
            publish_local_session_expiry("list_configured_models", selected_token).await;
            if provider_key.is_empty() {
                ListingAuth::Empty("session expired")
            } else {
                ListingAuth::ProviderKey(provider_key.to_string())
            }
        }
        SessionTokenCheck::Absent if provider_key.is_empty() => ListingAuth::Empty("no session"),
        SessionTokenCheck::Absent => ListingAuth::ProviderKey(provider_key.to_string()),
    }
}

/// The `list_configured_models` listing for the managed provider.
pub(super) async fn list_models(
    config: &Config,
    request: CatalogHostRequest,
) -> Result<Outcome<serde_json::Value>, String> {
    // Classify the session directly: a store error propagates (a recoverable
    // fault the picker should surface), only an absent or expired session
    // degrades to an empty list, or to the entry's own key.
    let profile = load_app_session_profile(config).await?;
    let check = classify_session_token(profile.as_ref(), chrono::Utc::now());
    let selected_token = session_token_from_profile(profile.as_ref());
    let (token, session_token) =
        match listing_auth(check, selected_token.as_deref(), &request.provider_key).await {
            ListingAuth::Session(token) => (token.clone(), true),
            ListingAuth::ProviderKey(key) => {
                log::debug!(
                    "[tinyhumans][inference] no live session; using the provider-scoped key"
                );
                (key, false)
            }
            ListingAuth::Empty(reason) => {
                log::info!("[tinyhumans][inference] managed catalog unavailable: {reason}");
                return empty_catalog(reason);
            }
        };

    let url = catalog_url(config);
    log::debug!("[tinyhumans][inference] managed catalog url={url}");
    let client = request.client;

    let attach = is_safe_endpoint_for_managed_bearer(&url);
    // Whether the session token itself made it onto the wire: a 401 on a
    // request that carried only a provider key describes that key, not the
    // session.
    let session_attached = attach && session_token;
    let extra_headers = if attach {
        attribution_headers()
            .ok()
            .map(|headers| {
                headers
                    .iter()
                    .filter_map(|(name, value)| {
                        value
                            .to_str()
                            .ok()
                            .map(|value| (name.as_str().to_owned(), value.to_owned()))
                    })
                    .collect()
            })
            .unwrap_or_default()
    } else {
        log::warn!(
            "[tinyhumans][inference] refusing to send a bearer token to a non-https, non-loopback URL"
        );
        Vec::new()
    };
    let request = discovery_request(
        &effective_backend_api_url(&config.api_url),
        "",
        attach.then_some(token.as_str()),
        &extra_headers,
    )
    .with_listing_url(url.clone())
    .with_timeout(std::time::Duration::from_secs(30));

    let models = match fetch_model_catalog(&client, request).await {
        Ok(models) => models,
        Err(error) => {
            if error
                .http_status()
                .is_some_and(|status| managed_401_means_signed_out(status, session_attached))
            {
                log::info!(
                    "[tinyhumans][inference] managed catalog unavailable: backend rejected the session token (401)"
                );
                return empty_catalog("session not accepted");
            }
            return Err(error.to_string());
        }
    };
    log::info!(
        "[tinyhumans][inference] managed catalog fetched {} models",
        models.len()
    );
    Ok(Outcome::new(
        serde_json::json!({ "models": models }),
        vec![format!("fetched {} models", models.len())],
    ))
}

/// Where to ask the managed catalog for `model`'s limits, keyed by the managed
/// inference base so overflow corrections from the chat model land on the same
/// entry. Authenticated with the backend credential when the URL is safe to
/// carry it.
pub(super) async fn limits_request(model: &str, config: &Config) -> Option<DiscoveryRequest> {
    let model = model.trim();
    if model.is_empty() {
        return None;
    }
    let inference_base = effective_api_url(&config.api_url);
    let listing_url = catalog_url(config);
    let auth = match resolve_backend_credential(config).await {
        Ok(BackendCredential::ApiKey(key)) if is_managed_endpoint_for_api_key(&listing_url) => {
            Some(key)
        }
        Ok(BackendCredential::Session(token))
            if is_safe_endpoint_for_managed_bearer(&listing_url) =>
        {
            Some(token)
        }
        Ok(_) => {
            log::warn!(
                "[tinyhumans][inference] refusing to send the backend credential to an unsafe listing url"
            );
            None
        }
        Err(_) => {
            log::debug!(
                "[tinyhumans][inference] no backend credential; managed listing probed anonymously"
            );
            None
        }
    };
    let extra_headers = attribution_headers()
        .ok()
        .map(|headers| {
            headers
                .iter()
                .filter_map(|(name, value)| {
                    value
                        .to_str()
                        .ok()
                        .map(|value| (name.as_str().to_owned(), value.to_owned()))
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let request = discovery_request(&inference_base, model, auth.as_deref(), &extra_headers)
        .with_listing_url(listing_url);
    Some(request)
}

#[cfg(test)]
#[path = "managed_catalog_tests.rs"]
mod tests;
