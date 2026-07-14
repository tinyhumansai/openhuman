use axum::extract::Json;
use axum::http::StatusCode;
use axum::routing::post;
use axum::Router;
use openhuman_core::openhuman::memory_tree::score::embed::{
    Embedder, OllamaEmbedder, EMBEDDING_DIM,
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
        "/api/embeddings",
        post({
            let success_vec = success_vec.clone();
            move |Json(body): Json<Value>| {
                let success_vec = success_vec.clone();
                async move {
                    assert_eq!(body["model"], "round25-embed");
                    assert_eq!(body["prompt"], "memory tree round25");
                    assert_eq!(body["options"]["num_ctx"], 8192);
                    Json(json!({ "embedding": success_vec }))
                }
            }
        }),
    );
    let url = start_embed_server(app).await;
    let embedder = OllamaEmbedder::new(format!("{url}/"), "round25-embed".to_string(), 0);
    assert_eq!(embedder.name(), "ollama");
    let embedding = embedder
        .embed("memory tree round25")
        .await
        .expect("loopback embedding");
    assert_eq!(embedding.len(), EMBEDDING_DIM);
    assert!((embedding[0] - 0.125).abs() < f32::EPSILON);

    let missing_model_url = start_embed_server(Router::new().route(
        "/api/embeddings",
        post(|| async { (StatusCode::NOT_FOUND, "{\"error\":\"model not found\"}") }),
    ))
    .await;
    let missing = OllamaEmbedder::new(missing_model_url, "missing-round25".to_string(), 500);
    let missing_err = missing
        .embed("text")
        .await
        .expect_err("missing model should fail")
        .to_string();
    assert!(missing_err.contains("embedding model `missing-round25` is not installed"));
    assert!(missing_err.contains("ollama pull missing-round25"));

    let dim_url = start_embed_server(Router::new().route(
        "/api/embeddings",
        post(|| async { Json(json!({ "embedding": [0.1, 0.2, 0.3] })) }),
    ))
    .await;
    let dim_mismatch = OllamaEmbedder::new(dim_url, String::new(), 0);
    let dim_err = dim_mismatch
        .embed("text")
        .await
        .expect_err("wrong dimensions should fail")
        .to_string();
    assert!(dim_err.contains("3 dims"));
    assert!(dim_err.contains("expected 1024"));

    let bad_json_url = start_embed_server(Router::new().route(
        "/api/embeddings",
        post(|| async { (StatusCode::OK, "not-json") }),
    ))
    .await;
    let bad_json = OllamaEmbedder::new(bad_json_url, String::new(), 0);
    let parse_err = bad_json
        .embed("text")
        .await
        .expect_err("invalid json should fail")
        .to_string();
    assert!(parse_err.contains("response parse failed"));
}
