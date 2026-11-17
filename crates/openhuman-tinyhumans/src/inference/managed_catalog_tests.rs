use super::*;
use axum::{
    http::{HeaderMap, StatusCode},
    routing::get,
    Router,
};
use openhuman_embed::__host::credentials::{AuthService, APP_SESSION_PROVIDER};
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

async fn catalog_server(
    status: StatusCode,
    body: &'static str,
) -> (String, Arc<Mutex<Vec<Option<String>>>>) {
    let authorization = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&authorization);
    let app = Router::new().route(
        "/openai/v1/models",
        get(move |headers: HeaderMap| {
            let captured = Arc::clone(&captured);
            async move {
                captured.lock().unwrap().push(
                    headers
                        .get(axum::http::header::AUTHORIZATION)
                        .and_then(|value| value.to_str().ok())
                        .map(str::to_owned),
                );
                (status, body)
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{address}"), authorization)
}

fn catalog_config(tmp: &TempDir, base_url: &str) -> Config {
    crate::hosted::test_support::config(tmp, base_url)
}

async fn without_session(config: &Config) {
    crate::hosted::test_support::store_session(config, "fixture-session-to-remove").await;
    AuthService::from_config(config)
        .remove_profile(APP_SESSION_PROVIDER, "default")
        .await
        .expect("remove fixture session");
}

#[test]
fn a_managed_401_on_an_attached_session_reads_as_signed_out() {
    assert!(managed_401_means_signed_out(401, true));
    assert!(!managed_401_means_signed_out(200, true));
}

/// A 401 on a request that never carried the session token describes the
/// request (a misconfigured URL), not the session, so the error surfaces.
#[test]
fn a_401_without_an_attached_session_still_surfaces() {
    assert!(!managed_401_means_signed_out(401, false));
}

#[test]
fn other_statuses_still_surface_the_error() {
    for status in [400, 403, 404, 429, 500, 503] {
        assert!(
            !managed_401_means_signed_out(status, true),
            "managed {status}"
        );
    }
}

#[test]
fn the_catalog_url_targets_the_openrouter_catalog_on_the_control_plane() {
    let config = Config {
        api_url: Some("https://api.tinyhumans.ai".into()),
        ..Config::default()
    };
    let url = catalog_url(&config);
    assert!(
        url.ends_with("/openai/v1/models?catalog=openrouter"),
        "{url}"
    );
}

#[tokio::test]
async fn limits_request_is_none_for_a_blank_model() {
    assert!(limits_request("  ", &Config::default()).await.is_none());
}

#[tokio::test]
async fn limits_request_lists_the_catalog_keyed_by_the_inference_base() {
    let config = Config {
        api_url: Some("https://api.tinyhumans.ai".into()),
        ..Config::default()
    };
    let request = limits_request("some/model", &config)
        .await
        .expect("managed limits request");
    assert_eq!(request.endpoint, "https://api.tinyhumans.ai/openai/v1");
    assert!(request
        .listing_url
        .as_deref()
        .is_some_and(|url| url.ends_with("/openai/v1/models?catalog=openrouter")));
}

#[tokio::test]
async fn a_live_session_wins_over_the_provider_key() {
    assert_eq!(
        listing_auth(
            SessionTokenCheck::Live("jwt".into()),
            Some("jwt"),
            "provider-key",
        )
        .await,
        ListingAuth::Session("jwt".into())
    );
}

/// A self-hosted entry that stores its own key still lists its catalog when the
/// user is signed out; before the move this fell back to that key.
#[tokio::test]
async fn without_a_live_session_the_entrys_provider_key_authenticates() {
    assert_eq!(
        listing_auth(SessionTokenCheck::Absent, None, " provider-key ").await,
        ListingAuth::ProviderKey("provider-key".into())
    );
    assert_eq!(
        listing_auth(SessionTokenCheck::Expired, Some("old-jwt"), "provider-key").await,
        ListingAuth::ProviderKey("provider-key".into())
    );
}

#[tokio::test]
async fn with_neither_credential_the_catalog_is_empty() {
    assert_eq!(
        listing_auth(SessionTokenCheck::Absent, None, "").await,
        ListingAuth::Empty("no session")
    );
    assert_eq!(
        listing_auth(SessionTokenCheck::Expired, Some("old-jwt"), "  ").await,
        ListingAuth::Empty("session expired")
    );
}

#[tokio::test]
async fn managed_catalog_treats_http_401_as_signed_out_only_for_an_attached_session() {
    let (base_url, authorization) =
        catalog_server(StatusCode::UNAUTHORIZED, "session token rejected").await;
    let tmp = TempDir::new().unwrap();
    let config = catalog_config(&tmp, &base_url);
    crate::hosted::test_support::store_session(&config, "fixture-live-session").await;

    let result = list_models(
        &config,
        CatalogHostRequest {
            provider_key: String::new(),
            client: reqwest::Client::new(),
        },
    )
    .await
    .expect("a rejected attached session degrades to an empty catalog");

    assert_eq!(result.value["models"], serde_json::json!([]));
    assert_eq!(
        authorization.lock().unwrap().as_slice(),
        [Some("Bearer fixture-live-session".to_string())]
    );
}

#[tokio::test]
async fn managed_catalog_keeps_provider_key_401_as_a_provider_error() {
    let (base_url, authorization) =
        catalog_server(StatusCode::UNAUTHORIZED, "provider key rejected").await;
    let tmp = TempDir::new().unwrap();
    let config = catalog_config(&tmp, &base_url);
    without_session(&config).await;

    let error = list_models(
        &config,
        CatalogHostRequest {
            provider_key: "fixture-provider-key".into(),
            client: reqwest::Client::new(),
        },
    )
    .await
    .expect_err("a 401 for a provider key remains actionable");

    assert!(error.contains("provider returned 401"), "{error}");
    assert_eq!(
        authorization.lock().unwrap().as_slice(),
        [Some("Bearer fixture-provider-key".to_string())]
    );
}

#[tokio::test]
async fn managed_catalog_does_not_infer_http_status_from_error_payload_text() {
    let (base_url, _authorization) =
        catalog_server(StatusCode::OK, r#"{"error":"401 session expired"}"#).await;
    let tmp = TempDir::new().unwrap();
    let config = catalog_config(&tmp, &base_url);
    crate::hosted::test_support::store_session(&config, "fixture-live-session").await;

    let error = list_models(
        &config,
        CatalogHostRequest {
            provider_key: String::new(),
            client: reqwest::Client::new(),
        },
    )
    .await
    .expect_err("a successful error envelope is not an HTTP 401");

    assert!(error.contains("provider returned error payload"), "{error}");
    assert!(error.contains("401 session expired"), "{error}");
}

#[cfg(test)]
#[path = "managed_catalog_inspection_tests.rs"]
mod inspection_tests;
