#![allow(clippy::await_holding_lock)]

use std::io::Write as _;
use std::sync::Mutex;

use axum::http::StatusCode;

use crate::core_host::core::runtime::DispatchCtx;
use crate::http_host::auth::{
    resolve_default_auth_username_from_user_value, sanitize_basic_auth_username,
};
use crate::http_host::ops::{
    list_hosted_dir_servers, start_hosted_dir_server, stop_all_hosted_dir_servers,
    stop_hosted_dir_server,
};
use crate::http_host::path_utils::resolve_request_path;
use crate::http_host::types::StartHostedDirParams;

// Serializes tests that share the process-global server registry; the guard is
// deliberately held across the awaits of each test.
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

#[cfg(unix)]
#[test]
fn resolve_request_path_rejects_a_symlink_out_of_the_hosted_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(tmp.path()).unwrap().join("shared");
    let outside = std::fs::canonicalize(tmp.path()).unwrap().join("outside");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("secret.txt"), "not shared").unwrap();
    std::os::unix::fs::symlink(&outside, root.join("link")).unwrap();

    let err = resolve_request_path(&root, "link/secret.txt").unwrap_err();
    assert!(err.contains("escapes"), "{err}");
}

#[test]
fn basic_auth_accepts_only_the_exact_credentials() {
    use axum::http::{header, HeaderMap, HeaderValue};
    use base64::Engine as _;

    let auth = crate::http_host::types::HostedDirAuth {
        enabled: true,
        username: Some("ada".into()),
        password: Some("s3cret".into()),
    };
    let headers_for = |user: &str, pass: &str| {
        let token = base64::engine::general_purpose::STANDARD.encode(format!("{user}:{pass}"));
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Basic {token}")).unwrap(),
        );
        headers
    };
    let check = |user: &str, pass: &str| {
        crate::http_host::auth::ensure_authorized(&headers_for(user, pass), &auth).is_ok()
    };
    assert!(check("ada", "s3cret"));
    for (user, pass) in [
        ("ada", "s3cre"),
        ("ada", "s3cretX"),
        ("adb", "s3cret"),
        ("", ""),
    ] {
        assert!(!check(user, pass), "{user}:{pass} must be refused");
    }
}

#[tokio::test]
async fn start_serves_files_with_basic_auth() {
    let _guard = TEST_MUTEX.lock().unwrap();
    stop_all_hosted_dir_servers().await;

    let tmp = tempfile::tempdir().unwrap();
    let file_path = tmp.path().join("hello.txt");
    let mut file = std::fs::File::create(&file_path).unwrap();
    writeln!(file, "hello from hosted dir").unwrap();

    let server = start_hosted_dir_server(
        &DispatchCtx::host_default(),
        StartHostedDirParams {
            directory: tmp.path().display().to_string(),
            port: 0,
            bind_host: "127.0.0.1".to_string(),
            server_name: Some("test".to_string()),
            disable_auth: false,
            username: Some("tester".to_string()),
        },
    )
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

#[test]
fn register_controllers_adds_http_host_namespace_idempotently() {
    crate::http_host::register_controllers().expect("first registration");
    crate::http_host::register_controllers().expect("re-registration is a no-op");
    let schemas = crate::core_host::core::all::all_controller_schemas();
    let functions: Vec<&str> = schemas
        .iter()
        .filter(|s| s.namespace == "http_host")
        .map(|s| s.function)
        .collect();
    for expected in ["start", "stop", "get", "list"] {
        assert!(
            functions.contains(&expected),
            "http_host.{expected} must be registered: {functions:?}"
        );
    }
}
