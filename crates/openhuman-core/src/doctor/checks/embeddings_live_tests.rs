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
