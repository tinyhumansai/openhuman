use std::path::PathBuf;
use std::sync::Arc;

use serde_json::json;

use super::{MediaGenerateImageTool, MediaGenerateVideoTool, MediaListModelsTool};
use crate::openhuman::integrations::IntegrationClient;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolCategory};

fn dummy_client() -> Arc<IntegrationClient> {
    // No requests are made in these tests; the URL/token are placeholders.
    Arc::new(IntegrationClient::new(
        "http://127.0.0.1:0".to_string(),
        "test-token".to_string(),
    ))
}

#[test]
fn image_tool_schema_and_metadata() {
    let tool = MediaGenerateImageTool::new(dummy_client(), PathBuf::from("/tmp"));
    assert_eq!(tool.name(), "media_generate_image");
    assert_eq!(tool.permission_level(), PermissionLevel::Execute);
    assert_eq!(tool.category(), ToolCategory::Workflow);

    let schema = tool.parameters_schema();
    assert_eq!(schema["required"], json!(["prompt"]));
    let props = schema["properties"].as_object().unwrap();
    for key in ["prompt", "model", "size", "n", "input_images", "seed"] {
        assert!(props.contains_key(key), "missing image property {key}");
    }
}

#[test]
fn video_tool_schema_and_metadata() {
    let tool = MediaGenerateVideoTool::new(dummy_client(), PathBuf::from("/tmp"));
    assert_eq!(tool.name(), "media_generate_video");
    assert_eq!(tool.permission_level(), PermissionLevel::Execute);
    assert_eq!(tool.category(), ToolCategory::Workflow);

    let schema = tool.parameters_schema();
    assert_eq!(schema["required"], json!(["prompt"]));
    let props = schema["properties"].as_object().unwrap();
    for key in [
        "prompt",
        "model",
        "input_image",
        "duration_seconds",
        "aspect_ratio",
        "negative_prompt",
        "seed",
    ] {
        assert!(props.contains_key(key), "missing video property {key}");
    }
}

#[test]
fn list_models_tool_metadata() {
    let tool = MediaListModelsTool::new(dummy_client());
    assert_eq!(tool.name(), "media_list_models");
    assert_eq!(tool.category(), ToolCategory::Workflow);
    assert!(tool.parameters_schema()["properties"]
        .as_object()
        .unwrap()
        .contains_key("include_upstream"));
}

#[tokio::test]
async fn image_tool_rejects_empty_prompt_without_network() {
    let tool = MediaGenerateImageTool::new(dummy_client(), PathBuf::from("/tmp"));
    let result = tool.execute(json!({ "prompt": "   " })).await.unwrap();
    assert!(result.is_error);
}

#[tokio::test]
async fn video_tool_rejects_missing_prompt_without_network() {
    let tool = MediaGenerateVideoTool::new(dummy_client(), PathBuf::from("/tmp"));
    let result = tool.execute(json!({ "model": "x" })).await.unwrap();
    assert!(result.is_error);
}

// ── End-to-end flow against a mock backend (wiremock) ───────────────

use wiremock::matchers::{method, path, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn client_for(server: &MockServer) -> std::sync::Arc<IntegrationClient> {
    std::sync::Arc::new(IntegrationClient::new(server.uri(), "tok".to_string()))
}

/// Mount a media download endpoint that returns `bytes` for the given path.
async fn mount_media(server: &MockServer, p: &str, content_type: &str, bytes: &[u8]) {
    Mock::given(method("GET"))
        .and(path(p.to_string()))
        .respond_with(ResponseTemplate::new(200).set_body_raw(bytes.to_vec(), content_type))
        .mount(server)
        .await;
}

#[tokio::test]
async fn image_tool_submits_downloads_and_persists_local_artifact() {
    let server = MockServer::start().await;
    let media_url = format!("{}/media/out.png", server.uri());
    Mock::given(method("POST"))
        .and(path("/agent-integrations/media-generation/images"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            serde_json::json!({ "success": true, "data": {
            "requestId": "req-1",
            "status": "success",
            "model": "seedream-4-0-250828",
            "media": [{ "type": "image", "url": media_url }],
            "costUsd": 0.039
        } }),
        ))
        .mount(&server)
        .await;
    mount_media(&server, "/media/out.png", "image/png", b"PNGBYTES").await;

    let tmp = tempfile::tempdir().unwrap();
    let tool = MediaGenerateImageTool::new(client_for(&server), tmp.path().to_path_buf());
    let res = tool
        .execute(json!({ "prompt": "a fox", "size": "1024x1024" }))
        .await
        .unwrap();

    assert!(!res.is_error, "expected success, got {res:?}");
    let dir = tmp.path().join("generated-media");
    let files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    assert_eq!(files.len(), 1, "exactly one artifact should be persisted");
    assert_eq!(std::fs::read(files[0].path()).unwrap(), b"PNGBYTES");
}

#[tokio::test]
async fn video_tool_persists_clip_with_image_to_video_payload() {
    let server = MockServer::start().await;
    let media_url = format!("{}/media/clip.mp4", server.uri());
    Mock::given(method("POST"))
        .and(path("/agent-integrations/media-generation/videos"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            serde_json::json!({ "success": true, "data": {
            "requestId": "vid-1",
            "status": "success",
            "model": "seedance-1-0-pro-fast-251015",
            "media": [{ "type": "video", "url": media_url, "thumbnailUrl": "https://x/t.png" }],
            "costUsd": 0.13
        } }),
        ))
        .mount(&server)
        .await;
    mount_media(&server, "/media/clip.mp4", "video/mp4", b"MP4BYTES").await;

    let tmp = tempfile::tempdir().unwrap();
    let tool = MediaGenerateVideoTool::new(client_for(&server), tmp.path().to_path_buf());
    let res = tool
        .execute(
            json!({ "prompt": "a wave", "input_image": "https://in/f.png", "duration_seconds": 6 }),
        )
        .await
        .unwrap();

    assert!(!res.is_error, "expected success, got {res:?}");
    let dir = tmp.path().join("generated-media");
    let files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    assert_eq!(files.len(), 1);
    assert!(files[0].path().extension().is_some_and(|e| e == "mp4"));
}

#[tokio::test]
async fn image_tool_polls_until_terminal_then_persists() {
    let server = MockServer::start().await;
    let media_url = format!("{}/media/p.png", server.uri());
    // Submit returns a non-terminal status; the tool must poll the status endpoint.
    Mock::given(method("POST"))
        .and(path("/agent-integrations/media-generation/images"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            serde_json::json!({ "success": true, "data": {
            "requestId": "req-2", "status": "queued", "model": "seedream-4-0-250828", "media": []
        } }),
        ))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path_regex(
            r"^/agent-integrations/media-generation/requests/.+",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            serde_json::json!({ "success": true, "data": {
            "requestId": "req-2",
            "status": "success",
            "model": "seedream-4-0-250828",
            "media": [{ "type": "image", "url": media_url }],
            "costUsd": 0.039
        } }),
        ))
        .mount(&server)
        .await;
    mount_media(&server, "/media/p.png", "image/png", b"POLLED").await;

    let tmp = tempfile::tempdir().unwrap();
    let tool = MediaGenerateImageTool::new(client_for(&server), tmp.path().to_path_buf());
    let res = tool.execute(json!({ "prompt": "a fox" })).await.unwrap();
    assert!(!res.is_error, "expected success after poll, got {res:?}");
    assert_eq!(
        std::fs::read_dir(tmp.path().join("generated-media"))
            .unwrap()
            .count(),
        1
    );
}

#[tokio::test]
async fn image_tool_reports_failed_terminal_status() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/agent-integrations/media-generation/images"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            serde_json::json!({ "success": true, "data": {
            "requestId": "req-3", "status": "failed", "model": "seedream-4-0-250828", "media": []
        } }),
        ))
        .mount(&server)
        .await;
    let tmp = tempfile::tempdir().unwrap();
    let tool = MediaGenerateImageTool::new(client_for(&server), tmp.path().to_path_buf());
    let res = tool.execute(json!({ "prompt": "a fox" })).await.unwrap();
    assert!(res.is_error);
}

#[tokio::test]
async fn list_models_tool_returns_backend_catalog() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/agent-integrations/media-generation/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            serde_json::json!({ "success": true, "data": {
            "curated": [{ "id": "seedream-4-0-250828", "modality": "image" }]
        } }),
        ))
        .mount(&server)
        .await;
    let tool = MediaListModelsTool::new(client_for(&server));
    let res = tool.execute(json!({})).await.unwrap();
    assert!(!res.is_error, "expected success, got {res:?}");
}
