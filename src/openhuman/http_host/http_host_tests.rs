use std::io::Write as _;
use std::sync::Mutex;

use axum::http::StatusCode;

use crate::openhuman::http_host::auth::{
    resolve_default_auth_username_from_user_value, sanitize_basic_auth_username,
};
use crate::openhuman::http_host::ops::{
    list_hosted_dir_servers, start_hosted_dir_server, stop_all_hosted_dir_servers,
    stop_hosted_dir_server,
};
use crate::openhuman::http_host::path_utils::resolve_request_path;
use crate::openhuman::http_host::types::StartHostedDirParams;

static TEST_MUTEX: Mutex<()> = Mutex::new(());

#[test]
fn sanitize_basic_auth_username_normalizes_and_limits() {
    let username = sanitize_basic_auth_username(Some(" Jane Doe:admin ".to_string())).unwrap();
    assert_eq!(username, "Jane-Doeadmin");
}

#[test]
fn resolve_user_name_prefers_username_like_fields() {
    let user = serde_json::json!({
        "displayName": "Display Name",
        "username": "primary-user"
    });
    assert_eq!(
        resolve_default_auth_username_from_user_value(&user).as_deref(),
        Some("primary-user")
    );
}

#[test]
fn resolve_request_path_rejects_traversal() {
    let tmp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    let err = resolve_request_path(&root, "../secret").unwrap_err();
    assert!(err.contains("path traversal"));
}

#[tokio::test]
async fn start_serves_files_with_basic_auth() {
    let _guard = TEST_MUTEX.lock().unwrap();
    stop_all_hosted_dir_servers().await;

    let tmp = tempfile::tempdir().unwrap();
    let file_path = tmp.path().join("hello.txt");
    let mut file = std::fs::File::create(&file_path).unwrap();
    writeln!(file, "hello from hosted dir").unwrap();

    let server = start_hosted_dir_server(StartHostedDirParams {
        directory: tmp.path().display().to_string(),
        port: 0,
        bind_host: "127.0.0.1".to_string(),
        server_name: Some("test".to_string()),
        disable_auth: false,
        username: Some("tester".to_string()),
    })
    .await
    .unwrap();

    let client = reqwest::Client::builder().build().unwrap();
    let unauthorized = client
        .get(format!("{}hello.txt", server.local_url))
        .send()
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    let auth_user = server.auth.username.clone().unwrap();
    let auth_pass = server.auth.password.clone().unwrap();
    let authorized = client
        .get(format!("{}hello.txt", server.local_url))
        .basic_auth(auth_user, Some(auth_pass))
        .send()
        .await
        .unwrap();
    assert_eq!(authorized.status(), StatusCode::OK);
    assert!(authorized
        .text()
        .await
        .unwrap()
        .contains("hello from hosted dir"));

    let listed = list_hosted_dir_servers().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].server_id, server.server_id);

    let stopped = stop_hosted_dir_server(&server.server_id).await.unwrap();
    assert_eq!(stopped.server_id, server.server_id);
    assert!(list_hosted_dir_servers().unwrap().is_empty());
}
