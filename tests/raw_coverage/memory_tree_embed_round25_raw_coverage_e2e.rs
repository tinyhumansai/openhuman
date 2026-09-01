use axum::extract::Json;
use axum::http::StatusCode;
use axum::routing::post;
use axum::Router;
use tinymemory_core::tree::score::embed::EMBEDDING_DIM;
use tinyinference::embeddings::{
    EmbeddingModel, OllamaEmbeddingModel, RECOMMENDED_OLLAMA_CONTEXT_TOKENS,
};
use serde_json::{json, Value};

async fn start_embed_server(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind embed fixture");
    let addr = listener.local_addr().expect("listener addr");
    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve embed fixture");
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn round25_ollama_embedder_covers_success_and_error_edges_without_real_ollama() {
    let success_vec = vec![0.125_f32; EMBEDDING_DIM];
    let app = Router::new().route(
        "/api/embed",
        post({
            let success_vec = success_vec.clone();
            move |Json(body): Json<Value>| {
                let success_vec = success_vec.clone();
                async move {
                    assert_eq!(body["model"], "round25-embed");
                    assert_eq!(body["input"][0], "memory tree round25");
                    assert_eq!(body["options"]["num_ctx"], 8192);
                    assert_eq!(body["options"]["num_batch"], 8192);
                    Json(json!({ "embeddings": [success_vec] }))
                }
            }
        }),
    );
    let url = start_embed_server(app).await;
    let embedder = OllamaEmbeddingModel::new(&format!("{url}/"), "round25-embed", EMBEDDING_DIM)
        .with_context_options(
            RECOMMENDED_OLLAMA_CONTEXT_TOKENS,
            RECOMMENDED_OLLAMA_CONTEXT_TOKENS,
        );
    assert_eq!(embedder.name(), "ollama");
    let embedding = embedder
        .embed(&["memory tree round25".to_string()])
        .await
        .expect("loopback embedding");
    assert_eq!(embedding[0].len(), EMBEDDING_DIM);
    assert!((embedding[0][0] - 0.125).abs() < f32::EPSILON);

    let missing_model_url = start_embed_server(Router::new().route(
        "/api/embed",
        post(|| async { (StatusCode::NOT_FOUND, "{\"error\":\"model not found\"}") }),
    ))
    .await;
    let missing = OllamaEmbeddingModel::new(&missing_model_url, "missing-round25", EMBEDDING_DIM);
    let missing_err = missing
        .embed(&["text".to_string()])
        .await
        .expect_err("missing model should fail")
        .to_string();
    assert!(
        missing_err.contains("ollama embed failed with status 404 Not Found"),
        "{missing_err}"
    );
    assert!(missing_err.contains("model not found"));

    let dim_url = start_embed_server(Router::new().route(
        "/api/embed",
        post(|| async { Json(json!({ "embeddings": [[0.1, 0.2, 0.3]] })) }),
    ))
    .await;
    let dim_mismatch = OllamaEmbeddingModel::new(&dim_url, "", EMBEDDING_DIM);
    let dim_err = dim_mismatch
        .embed(&["text".to_string()])
        .await
        .expect_err("wrong dimensions should fail")
        .to_string();
    assert!(dim_err.contains("got 3"));
    assert!(dim_err.contains("expected 1024"));

    let bad_json_url = start_embed_server(Router::new().route(
        "/api/embed",
        post(|| async { (StatusCode::OK, "not-json") }),
    ))
    .await;
    let bad_json = OllamaEmbeddingModel::new(&bad_json_url, "", EMBEDDING_DIM);
    let parse_err = bad_json
        .embed(&["text".to_string()])
        .await
        .expect_err("invalid json should fail")
        .to_string();
    assert!(parse_err.contains("response parse failed"));
}
