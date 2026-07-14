use std::path::PathBuf;
use std::sync::Arc;

use serde_json::json;

use super::{
    resolve_upload_path, sanitize_filename, StorageDeleteFileTool, StorageDownloadFileTool,
    StorageGetLinkTool, StorageListFilesTool, StorageSetVisibilityTool, StorageUploadFileTool,
};
use crate::openhuman::integrations::IntegrationClient;
use crate::openhuman::security::{AutonomyLevel, SecurityPolicy};
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolCategory};

fn dummy_client() -> Arc<IntegrationClient> {
    // No requests are made in these tests; the URL/token are placeholders.
    Arc::new(IntegrationClient::new(
        "http://127.0.0.1:0".to_string(),
        "test-token".to_string(),
    ))
}

// ── Metadata / schema ───────────────────────────────────────────────

#[test]
fn upload_tool_schema_and_metadata() {
    let tool = StorageUploadFileTool::new(dummy_client(), PathBuf::from("/tmp"));
    assert_eq!(tool.name(), "storage_upload_file");
    assert_eq!(tool.permission_level(), PermissionLevel::Execute);
    assert_eq!(tool.category(), ToolCategory::Workflow);
    assert!(tool.external_effect());

    let schema = tool.parameters_schema();
    assert_eq!(schema["required"], json!(["path"]));
    let props = schema["properties"].as_object().unwrap();
    for key in ["path", "visibility", "ttl_days"] {
        assert!(props.contains_key(key), "missing upload property {key}");
    }
}

#[test]
fn download_tool_schema_and_metadata() {
    let tool = StorageDownloadFileTool::new(dummy_client(), PathBuf::from("/tmp"));
    assert_eq!(tool.name(), "storage_download_file");
    assert_eq!(tool.permission_level(), PermissionLevel::Write);
    assert_eq!(tool.category(), ToolCategory::Workflow);
    assert!(tool.external_effect());

    let schema = tool.parameters_schema();
    assert_eq!(schema["required"], json!(["file_id"]));
    let props = schema["properties"].as_object().unwrap();
    for key in ["file_id", "filename"] {
        assert!(props.contains_key(key), "missing download property {key}");
    }
}

#[test]
fn list_tool_metadata() {
    let tool = StorageListFilesTool::new(dummy_client());
    assert_eq!(tool.name(), "storage_list_files");
    assert_eq!(tool.permission_level(), PermissionLevel::ReadOnly);
    assert_eq!(tool.category(), ToolCategory::Workflow);
    assert!(!tool.external_effect());
}

#[test]
fn get_link_tool_schema_and_metadata() {
    let tool = StorageGetLinkTool::new(dummy_client());
    assert_eq!(tool.name(), "storage_get_link");
    assert_eq!(tool.permission_level(), PermissionLevel::Write);
    assert_eq!(tool.category(), ToolCategory::Workflow);
    assert!(tool.external_effect());

    let schema = tool.parameters_schema();
    assert_eq!(schema["required"], json!(["file_id"]));
    let props = schema["properties"].as_object().unwrap();
    for key in ["file_id", "expires_in_seconds"] {
        assert!(props.contains_key(key), "missing link property {key}");
    }
}

#[test]
fn set_visibility_tool_schema_and_metadata() {
    let tool = StorageSetVisibilityTool::new(dummy_client());
    assert_eq!(tool.name(), "storage_set_visibility");
    assert_eq!(tool.permission_level(), PermissionLevel::Write);
    assert_eq!(tool.category(), ToolCategory::Workflow);
    assert!(tool.external_effect());

    let schema = tool.parameters_schema();
    assert_eq!(schema["required"], json!(["file_id", "visibility"]));
}

#[test]
fn delete_tool_schema_and_metadata() {
    let tool = StorageDeleteFileTool::new(dummy_client());
    assert_eq!(tool.name(), "storage_delete_file");
    assert_eq!(tool.permission_level(), PermissionLevel::Write);
    assert_eq!(tool.category(), ToolCategory::Workflow);
    assert!(tool.external_effect());
    assert_eq!(tool.parameters_schema()["required"], json!(["file_id"]));
}

// ── Arg validation (no network) ─────────────────────────────────────

#[tokio::test]
async fn upload_rejects_missing_path_without_network() {
    let tool = StorageUploadFileTool::new(dummy_client(), PathBuf::from("/tmp"));
    let res = tool.execute(json!({})).await.unwrap();
    assert!(res.is_error);
    assert!(res.output().contains("path is required"));
}

#[tokio::test]
async fn upload_rejects_path_escaping_workspace() {
    let tmp = tempfile::tempdir().unwrap();
    // A real file OUTSIDE the workspace root.
    let outside = tempfile::tempdir().unwrap();
    let secret = outside.path().join("secret.txt");
    std::fs::write(&secret, b"nope").unwrap();

    let tool = StorageUploadFileTool::new(dummy_client(), tmp.path().to_path_buf());

    // Absolute path outside the workspace.
    let res = tool
        .execute(json!({ "path": secret.display().to_string() }))
        .await
        .unwrap();
    assert!(res.is_error, "absolute escape must be rejected: {res:?}");
    assert!(res.output().contains("escapes"), "got: {}", res.output());

    // Relative traversal out of the workspace.
    let rel = format!(
        "../{}/secret.txt",
        outside.path().file_name().unwrap().to_str().unwrap()
    );
    let res = tool.execute(json!({ "path": rel })).await.unwrap();
    assert!(res.is_error, "relative escape must be rejected: {res:?}");
}

#[tokio::test]
async fn upload_rejects_nonexistent_file() {
    let tmp = tempfile::tempdir().unwrap();
    let tool = StorageUploadFileTool::new(dummy_client(), tmp.path().to_path_buf());
    let res = tool
        .execute(json!({ "path": "missing.txt" }))
        .await
        .unwrap();
    assert!(res.is_error);
}

#[tokio::test]
async fn upload_rejects_bad_visibility_and_ttl() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("a.txt"), b"hi").unwrap();
    let tool = StorageUploadFileTool::new(dummy_client(), tmp.path().to_path_buf());

    let res = tool
        .execute(json!({ "path": "a.txt", "visibility": "everyone" }))
        .await
        .unwrap();
    assert!(res.is_error);
    assert!(res.output().contains("visibility"));

    let res = tool
        .execute(json!({ "path": "a.txt", "ttl_days": 0 }))
        .await
        .unwrap();
    assert!(res.is_error);
    assert!(res.output().contains("ttl_days"));
}

#[tokio::test]
async fn download_rejects_missing_or_invalid_file_id() {
    let tool = StorageDownloadFileTool::new(dummy_client(), PathBuf::from("/tmp"));
    let res = tool.execute(json!({})).await.unwrap();
    assert!(res.is_error);
    let res = tool.execute(json!({ "file_id": "../etc" })).await.unwrap();
    assert!(res.is_error);
}

#[tokio::test]
async fn get_link_rejects_missing_file_id() {
    let tool = StorageGetLinkTool::new(dummy_client());
    let res = tool.execute(json!({})).await.unwrap();
    assert!(res.is_error);
}

#[tokio::test]
async fn set_visibility_rejects_missing_or_invalid_args() {
    let tool = StorageSetVisibilityTool::new(dummy_client());
    let res = tool.execute(json!({ "file_id": "f1" })).await.unwrap();
    assert!(res.is_error);
    let res = tool
        .execute(json!({ "file_id": "f1", "visibility": "hidden" }))
        .await
        .unwrap();
    assert!(res.is_error);
}

#[tokio::test]
async fn delete_rejects_missing_file_id() {
    let tool = StorageDeleteFileTool::new(dummy_client());
    let res = tool.execute(json!({})).await.unwrap();
    assert!(res.is_error);
}

// ── Path / filename helpers ─────────────────────────────────────────

#[test]
fn resolve_upload_path_accepts_inside_and_rejects_outside() {
    let tmp = tempfile::tempdir().unwrap();
    let inner = tmp.path().join("sub");
    std::fs::create_dir_all(&inner).unwrap();
    let file = inner.join("data.bin");
    std::fs::write(&file, b"x").unwrap();

    // Relative path inside the root resolves.
    let ok = resolve_upload_path(tmp.path(), "sub/data.bin").unwrap();
    assert!(ok.ends_with("data.bin"));

    // Traversal escaping the root is rejected.
    let err = resolve_upload_path(&inner, "../../etc/hosts").unwrap_err();
    assert!(
        err.contains("escapes") || err.contains("not exist"),
        "got: {err}"
    );

    // A directory is not uploadable.
    let err = resolve_upload_path(tmp.path(), "sub").unwrap_err();
    assert!(err.contains("not a regular file"), "got: {err}");
}

#[test]
fn sanitize_filename_strips_separators_and_traversal() {
    assert_eq!(
        sanitize_filename("report.pdf").as_deref(),
        Some("report.pdf")
    );
    assert_eq!(
        sanitize_filename("../../evil.sh").as_deref(),
        Some("evil.sh")
    );
    assert_eq!(sanitize_filename("a/b\\c.txt").as_deref(), Some("c.txt"));
    assert_eq!(sanitize_filename("..").is_none(), true);
    assert_eq!(sanitize_filename("  ").is_none(), true);
}

// ── End-to-end flows against a mock backend (wiremock) ──────────────

use wiremock::matchers::{body_string_contains, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn client_for(server: &MockServer) -> Arc<IntegrationClient> {
    Arc::new(IntegrationClient::new(server.uri(), "tok".to_string()))
}

fn readonly_security() -> Arc<SecurityPolicy> {
    Arc::new(SecurityPolicy {
        autonomy: AutonomyLevel::ReadOnly,
        ..SecurityPolicy::default()
    })
}

#[tokio::test]
async fn storage_transfer_tools_block_readonly_autonomy_before_network() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("a.txt"), b"hi").unwrap();

    let upload = StorageUploadFileTool::new_with_security(
        dummy_client(),
        tmp.path().to_path_buf(),
        readonly_security(),
    );
    let upload_res = upload.execute(json!({ "path": "a.txt" })).await.unwrap();
    assert!(upload_res.is_error);
    assert!(upload_res.output().contains("[policy-blocked]"));

    let download = StorageDownloadFileTool::new_with_security(
        dummy_client(),
        tmp.path().to_path_buf(),
        readonly_security(),
    );
    let download_res = download
        .execute(json!({ "file_id": "file_123" }))
        .await
        .unwrap();
    assert!(download_res.is_error);
    assert!(download_res.output().contains("[policy-blocked]"));

    let get_link = StorageGetLinkTool::new_with_security(dummy_client(), readonly_security());
    let link_res = get_link
        .execute(json!({ "file_id": "file_123" }))
        .await
        .unwrap();
    assert!(link_res.is_error);
    assert!(link_res.output().contains("[policy-blocked]"));

    let set_visibility =
        StorageSetVisibilityTool::new_with_security(dummy_client(), readonly_security());
    let visibility_res = set_visibility
        .execute(json!({ "file_id": "file_123", "visibility": "public" }))
        .await
        .unwrap();
    assert!(visibility_res.is_error);
    assert!(visibility_res.output().contains("[policy-blocked]"));

    let delete = StorageDeleteFileTool::new_with_security(dummy_client(), readonly_security());
    let delete_res = delete
        .execute(json!({ "file_id": "file_123" }))
        .await
        .unwrap();
    assert!(delete_res.is_error);
    assert!(delete_res.output().contains("[policy-blocked]"));
}

#[tokio::test]
async fn upload_tool_posts_multipart_and_reports_file_id() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/agent-integrations/file-storage/files"))
        .and(header("authorization", "Bearer tok"))
        // Multipart body carries the file part + our extra form fields.
        .and(body_string_contains("name=\"file\""))
        .and(body_string_contains("HELLO-BYTES"))
        .and(body_string_contains("name=\"visibility\""))
        .and(body_string_contains("name=\"ttlDays\""))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "success": true,
            "data": {
                "fileId": "file-1",
                "filename": "hello.txt",
                "size": 11,
                "contentType": "text/plain",
                "visibility": "public",
                "expiresAt": "2026-07-12T00:00:00.000Z",
                "publicUrl": "https://api.example/agent-integrations/file-storage/public/file-1",
                "costUsd": 0.0001
            }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("hello.txt"), b"HELLO-BYTES").unwrap();

    let tool = StorageUploadFileTool::new(client_for(&server), tmp.path().to_path_buf());
    let res = tool
        .execute(json!({ "path": "hello.txt", "visibility": "public", "ttl_days": 7 }))
        .await
        .unwrap();

    assert!(!res.is_error, "expected success, got {res:?}");
    let out = res.output();
    assert!(out.contains("file-1"), "output should carry file_id: {out}");
    assert!(
        out.contains("public"),
        "output should carry public url/visibility: {out}"
    );
}

#[tokio::test]
async fn download_tool_follows_redirect_and_persists_file() {
    let server = MockServer::start().await;
    // The backend 302s to a presigned URL; reqwest follows it (same host in
    // this test, but the redirect-following behavior is what's exercised).
    let presigned = format!("{}/s3/blob", server.uri());
    Mock::given(method("GET"))
        .and(path(
            "/agent-integrations/file-storage/files/file-2/download",
        ))
        .and(header("authorization", "Bearer tok"))
        .respond_with(ResponseTemplate::new(302).insert_header("Location", presigned.as_str()))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/s3/blob"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(b"RAW-CONTENT".to_vec(), "text/plain")
                .insert_header("Content-Disposition", "attachment; filename=\"notes.txt\""),
        )
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    let tool = StorageDownloadFileTool::new(client_for(&server), tmp.path().to_path_buf());
    let res = tool.execute(json!({ "file_id": "file-2" })).await.unwrap();

    assert!(!res.is_error, "expected success, got {res:?}");
    let saved = tmp.path().join("storage-downloads").join("notes.txt");
    assert_eq!(std::fs::read(&saved).unwrap(), b"RAW-CONTENT");
    assert!(res.output().contains("notes.txt"));
}

#[tokio::test]
async fn download_tool_honors_explicit_filename() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/agent-integrations/file-storage/files/file-3/download",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_raw(b"BYTES".to_vec(), "application/pdf"))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    let tool = StorageDownloadFileTool::new(client_for(&server), tmp.path().to_path_buf());
    let res = tool
        .execute(json!({ "file_id": "file-3", "filename": "../sneaky/mine.pdf" }))
        .await
        .unwrap();

    assert!(!res.is_error, "expected success, got {res:?}");
    // Traversal in the requested name is stripped to the basename.
    let saved = tmp.path().join("storage-downloads").join("mine.pdf");
    assert_eq!(std::fs::read(&saved).unwrap(), b"BYTES");
}

#[tokio::test]
async fn list_tool_renders_files_and_usage() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/agent-integrations/file-storage/files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "success": true,
            "data": {
                "files": [
                    {
                        "fileId": "file-1",
                        "filename": "hello.txt",
                        "size": 11,
                        "contentType": "text/plain",
                        "visibility": "public",
                        "expiresAt": "2026-07-12T00:00:00.000Z",
                        "createdAt": "2026-07-05T00:00:00.000Z",
                        "publicUrl": "https://api.example/agent-integrations/file-storage/public/file-1"
                    }
                ],
                "usage": { "usedBytes": 11, "limitBytes": 1073741824 }
            }
        })))
        .mount(&server)
        .await;

    let tool = StorageListFilesTool::new(client_for(&server));
    let res = tool.execute(json!({})).await.unwrap();
    assert!(!res.is_error, "expected success, got {res:?}");
    let out = res.output();
    assert!(out.contains("file-1"));
    assert!(out.contains("hello.txt"));
}

#[tokio::test]
async fn get_link_tool_posts_expiry_and_returns_url() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/agent-integrations/file-storage/files/file-1/link"))
        .and(body_string_contains("expiresInSeconds"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "success": true,
            "data": {
                "url": "https://s3.example/presigned?sig=abc",
                "expiresAt": "2026-07-05T01:00:00.000Z",
                "costUsd": 0.0001
            }
        })))
        .mount(&server)
        .await;

    let tool = StorageGetLinkTool::new(client_for(&server));
    let res = tool
        .execute(json!({ "file_id": "file-1", "expires_in_seconds": 120 }))
        .await
        .unwrap();
    assert!(!res.is_error, "expected success, got {res:?}");
    assert!(res.output().contains("https://s3.example/presigned"));
}

#[tokio::test]
async fn set_visibility_tool_patches_and_reports_public_url() {
    let server = MockServer::start().await;
    Mock::given(method("PATCH"))
        .and(path("/agent-integrations/file-storage/files/file-1"))
        .and(body_string_contains("\"visibility\":\"public\""))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "success": true,
            "data": {
                "fileId": "file-1",
                "filename": "hello.txt",
                "size": 11,
                "visibility": "public",
                "expiresAt": "2026-07-12T00:00:00.000Z",
                "publicUrl": "https://api.example/agent-integrations/file-storage/public/file-1"
            }
        })))
        .mount(&server)
        .await;

    let tool = StorageSetVisibilityTool::new(client_for(&server));
    let res = tool
        .execute(json!({ "file_id": "file-1", "visibility": "public" }))
        .await
        .unwrap();
    assert!(!res.is_error, "expected success, got {res:?}");
    assert!(res.output().contains("public"));
    assert!(res.output().contains("file-storage/public/file-1"));
}

#[tokio::test]
async fn delete_tool_deletes_and_confirms() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/agent-integrations/file-storage/files/file-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "success": true,
            "data": { "deleted": true }
        })))
        .mount(&server)
        .await;

    let tool = StorageDeleteFileTool::new(client_for(&server));
    let res = tool.execute(json!({ "file_id": "file-1" })).await.unwrap();
    assert!(!res.is_error, "expected success, got {res:?}");
    assert!(
        res.output().contains("\"deleted\": true"),
        "got: {}",
        res.output()
    );
    assert!(res.output_for_llm(true).contains("Deleted"));
}

#[tokio::test]
async fn tools_surface_backend_envelope_errors() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/agent-integrations/file-storage/files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "success": false,
            "error": "Insufficient balance"
        })))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("a.txt"), b"hi").unwrap();
    let tool = StorageUploadFileTool::new(client_for(&server), tmp.path().to_path_buf());
    let res = tool.execute(json!({ "path": "a.txt" })).await.unwrap();
    assert!(res.is_error);
    assert!(
        res.output().contains("Insufficient balance"),
        "got: {}",
        res.output()
    );
}
