//! The doctor's read-only managed embedding probe versus real embedding traffic.
//!
//! The doctor never loads TinySecurity, so its inspection embedder must not
//! ask it; ordinary embedders must keep asking it on every batch. These tests
//! run with the `security-module` feature on (the default) and no TinySecurity
//! module loaded, which is exactly the doctor's situation.

use super::{from_request, from_request_for_inspection};
use axum::{routing::post, Json, Router};
use openhuman_embed::__host::config::Config;
use openhuman_embed::__host::inference_registry::{
    EmbeddingCallContext, EmbeddingRequest, ProviderContext,
};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tinyinference_embeddings::EmbeddingModel;

const PROBE: &str = "OpenHuman doctor embedding probe";

/// A loopback `/openai/v1/embeddings` mock that answers two-wide vectors and
/// counts the requests it receives.
async fn mock_embeddings() -> (String, Arc<AtomicUsize>) {
    let requests = Arc::new(AtomicUsize::new(0));
    let app = Router::new().route(
        "/openai/v1/embeddings",
        post({
            let requests = Arc::clone(&requests);
            move || async move {
                requests.fetch_add(1, Ordering::SeqCst);
                Json(serde_json::json!({
                    "object": "list",
                    "data": [{ "object": "embedding", "index": 0, "embedding": [0.5, 0.25] }],
                    "model": "m"
                }))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (base, requests)
}

/// A config pointed at `api_url` whose credential scope holds a TinyHumans
/// API key, so both the frozen and the live bearer resolve without a session.
async fn config_with_api_key(tmp: &tempfile::TempDir, api_url: &str) -> Config {
    let mut config = Config {
        config_path: tmp.path().join("config.toml"),
        api_url: Some(api_url.to_owned()),
        ..Config::default()
    };
    config.secrets.encrypt = false;
    openhuman_embed::__host::credentials::api_key::store_api_key_in(
        tmp.path(),
        false,
        "tiny_test_doctor_probe",
    )
    .await
    .unwrap();
    config
}

fn request_for(config: &Config) -> EmbeddingRequest<ProviderContext> {
    EmbeddingRequest {
        model: "m".to_owned(),
        dimensions: 2,
        context: ProviderContext {
            config: config.clone(),
            egress: openhuman_embed::__host::egress::EgressLedger::none(),
            embedding: Some(EmbeddingCallContext {
                config: Some(config.clone()),
                chat: None,
                state_dir: config.config_path.parent().unwrap().to_path_buf(),
                secrets_encrypt: config.secrets.encrypt,
            }),
            catalog: None,
        },
    }
}

/// The doctor's probe reaches the managed endpoint although TinySecurity is
/// not loaded: inspection is read-only and sends one fixed, non-user string.
#[tokio::test]
async fn inspection_embedder_probes_without_the_security_module() {
    let (base, requests) = mock_embeddings().await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_api_key(&tmp, &base).await;

    let model = from_request_for_inspection(&request_for(&config))
        .await
        .expect("the frozen API key builds an inspection embedder");
    let vectors = model
        .embed(&[PROBE.to_owned()])
        .await
        .expect("the inspection probe does not depend on TinySecurity");

    assert_eq!(vectors.first().map(Vec::len), Some(2));
    assert_eq!(requests.load(Ordering::SeqCst), 1);
}

/// An inspection embedder pointed at a closed port fails the probe instead of
/// reporting a vector width.
#[tokio::test]
async fn inspection_embedder_reports_an_unreachable_endpoint() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let closed = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_api_key(&tmp, &closed).await;

    let model = from_request_for_inspection(&request_for(&config))
        .await
        .expect("the frozen API key builds an inspection embedder");
    let error = model
        .embed(&[PROBE.to_owned()])
        .await
        .expect_err("a closed endpoint cannot answer the probe");

    assert!(
        !error.to_string().contains("security module"),
        "the failure is the transport, not the egress guard: {error}"
    );
}

/// Real embedding traffic still asks TinySecurity before a batch leaves: with
/// the module absent, the runtime embedder refuses and sends nothing, even
/// with a valid credential and a reachable endpoint.
#[tokio::test]
async fn runtime_embedder_still_enforces_the_egress_guard() {
    let (base, requests) = mock_embeddings().await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_api_key(&tmp, &base).await;

    let error = from_request(request_for(&config))
        .embed(&["user text".to_owned()])
        .await
        .expect_err("an unconfirmed egress must be refused");

    assert!(
        error
            .to_string()
            .contains("security module could not confirm"),
        "{error}"
    );
    assert_eq!(requests.load(Ordering::SeqCst), 0);
}
