use super::EmbeddingsLiveCheck;
use crate::config::Config;
use crate::core::runtime::{CoreContext, DomainSet};
use crate::core::slots::Extensions;
use crate::doctor::contract::{DoctorCheck, DoctorCtx, FindingSeverity};
use crate::inference::embedding_host::create_embedding_provider_for_inspection;
use crate::inference::provider::factory::PROVIDER_OPENHUMAN;
use axum::{extract::State, routing::post, Json, Router};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tempfile::TempDir;

fn config_in(tmp: &TempDir, embedding_url: String) -> Config {
    let workspace = tmp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace directory");
    let mut config = Config {
        workspace_dir: workspace.clone(),
        action_dir: workspace,
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    config.memory.embedding_provider = "ollama".to_owned();
    config.memory.embedding_model = "nomic-embed-text".to_owned();
    config.memory.embedding_dimensions = 1536;
    config.local_ai.base_url = Some(embedding_url);
    config
}

async fn mock_ollama(dimensions: usize) -> (String, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route(
            "/api/embed",
            post(
                move |State(calls): State<Arc<AtomicUsize>>,
                      Json(_body): Json<serde_json::Value>| async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Json(serde_json::json!({
                        "embeddings": [vec![0.1_f32; dimensions]],
                    }))
                },
            ),
        )
        .with_state(Arc::clone(&calls));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind local embedding mock");
    let endpoint = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (endpoint, calls)
}

#[tokio::test]
async fn reports_actual_embedding_dimensions_when_the_provider_ignores_config() {
    let tmp = TempDir::new().expect("temporary config and workspace");
    let (endpoint, calls) = mock_ollama(768).await;
    let ctx = DoctorCtx::from_config(config_in(&tmp, endpoint));

    let findings = EmbeddingsLiveCheck.detect(&ctx).await;

    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "the local embed mock is called once"
    );
    let finding = findings
        .iter()
        .find(|finding| finding.severity == FindingSeverity::Error)
        .expect("a provider dimension mismatch is an error");
    assert!(
        finding.message.contains("768") && finding.message.contains("1536"),
        "the finding names actual and configured dimensions: {}",
        finding.message
    );
}

#[tokio::test]
async fn managed_inspection_does_not_fall_back_to_the_mutable_provider_factory() {
    let tmp = TempDir::new().expect("temporary config and workspace");
    let workspace = tmp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace directory");
    let config = Config {
        workspace_dir: workspace.clone(),
        action_dir: workspace,
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    let calls = Arc::new(AtomicUsize::new(0));
    let provider_calls = Arc::clone(&calls);
    let extensions = Extensions::new();
    crate::inference::registry::register_embedding_in(
        &extensions,
        PROVIDER_OPENHUMAN,
        Arc::new(move |_| {
            provider_calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(async {
                Ok(Box::new(tinyinference_embeddings::NoopEmbeddingModel)
                    as Box<dyn tinyinference_embeddings::EmbeddingModel>)
            })
        }),
    );
    let context = CoreContext::for_test_with_extensions(DomainSet::full(), None, extensions);

    CoreContext::scope(context, async {
        let model = create_embedding_provider_for_inspection(
            &config,
            "managed",
            "managed-test-model",
            768,
            "",
            None,
        )
        .await
        .expect("inspection factory returns its unavailable placeholder");
        assert!(model
            .embed(&["fixed doctor probe".to_owned()])
            .await
            .is_err());
    })
    .await;

    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

/// The selected config routes embeddings to the managed cloud provider with a
/// two-wide model, so a loopback mock can answer the expected width.
fn managed_config_in(tmp: &TempDir) -> Config {
    let mut config = config_in(tmp, String::new());
    config.local_ai.base_url = None;
    config.memory.embedding_provider = "cloud".to_owned();
    config.memory.embedding_model = "managed-test-model".to_owned();
    config.memory.embedding_dimensions = 2;
    config
}

/// Registers, for one context only, the host's inspection-safe managed
/// embedder: a frozen bearer and no TinySecurity egress guard, which is the
/// shape `openhuman-tinyhumans` registers for the doctor.
fn inspection_embedder_at(base_url: String) -> Extensions {
    let extensions = Extensions::new();
    crate::inference::registry::register_inspection_embedding_in(
        &extensions,
        PROVIDER_OPENHUMAN,
        move |request| {
            let base_url = format!("{base_url}/openai/v1");
            Box::pin(async move {
                let bearer: tinyinference_embeddings::BearerResolver =
                    Arc::new(|| Box::pin(async { Ok("frozen-doctor-bearer".to_owned()) }));
                Ok(Box::new(tinyinference_embeddings::CloudEmbeddingModel::new(
                    base_url,
                    request.model,
                    request.dimensions,
                    bearer,
                )) as Box<dyn tinyinference_embeddings::EmbeddingModel>)
            })
        },
    );
    extensions
}

async fn mock_managed_embeddings() -> (String, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route(
            "/openai/v1/embeddings",
            post(
                |State(calls): State<Arc<AtomicUsize>>, Json(_body): Json<serde_json::Value>| async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Json(serde_json::json!({
                        "object": "list",
                        "data": [{ "object": "embedding", "index": 0, "embedding": [0.5, 0.25] }],
                        "model": "managed-test-model",
                    }))
                },
            ),
        )
        .with_state(Arc::clone(&calls));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind managed embedding mock");
    let endpoint = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (endpoint, calls)
}

async fn detect_managed(base_url: String, tmp: &TempDir) -> Vec<crate::doctor::Finding> {
    let ctx = DoctorCtx::from_config(managed_config_in(tmp));
    let context = CoreContext::for_test_with_extensions(
        DomainSet::full(),
        None,
        inspection_embedder_at(base_url),
    );
    CoreContext::scope(context, async { EmbeddingsLiveCheck.detect(&ctx).await }).await
}

#[tokio::test]
async fn managed_probe_passes_without_loading_the_security_module() {
    let tmp = TempDir::new().expect("temporary config and workspace");
    let (endpoint, calls) = mock_managed_embeddings().await;

    let findings = detect_managed(endpoint, &tmp).await;

    assert_eq!(calls.load(Ordering::SeqCst), 1, "one fixed probe request");
    assert_eq!(findings.len(), 1);
    assert_eq!(
        findings[0].severity,
        FindingSeverity::Pass,
        "{}",
        findings[0].message
    );
}

#[tokio::test]
async fn managed_probe_reports_an_unreachable_endpoint_as_an_error() {
    let tmp = TempDir::new().expect("temporary config and workspace");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("reserve a local port");
    let closed = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    drop(listener);

    let findings = detect_managed(closed, &tmp).await;

    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].severity, FindingSeverity::Error);
    assert!(
        findings[0].message.contains("could not complete the live probe"),
        "{}",
        findings[0].message
    );
}

#[tokio::test]
async fn local_only_privacy_refuses_the_managed_probe_before_any_request() {
    let _privacy =
        crate::security::live_policy::test_privacy_scope(crate::config::PrivacyMode::LocalOnly);
    let tmp = TempDir::new().expect("temporary config and workspace");
    let (endpoint, calls) = mock_managed_embeddings().await;

    let findings = detect_managed(endpoint, &tmp).await;

    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "local-only mode keeps the probe on this machine"
    );
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].severity, FindingSeverity::Error);
}
