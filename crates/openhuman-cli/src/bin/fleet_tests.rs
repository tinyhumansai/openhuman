use super::*;

#[test]
fn port_assignment_is_sequential_from_base() {
    assert_eq!(port_for_index(7900, 0), Some(7900));
    assert_eq!(port_for_index(7900, 5), Some(7905));
}

#[test]
fn port_assignment_detects_overflow() {
    assert_eq!(port_for_index(u16::MAX, 1), None);
}

#[test]
fn workspace_is_user_scoped_under_root() {
    let ws = workspace_for(Path::new("/srv/fleet"), "alice");
    assert_eq!(ws, PathBuf::from("/srv/fleet/alice"));
}

#[test]
fn action_dir_is_user_scoped_outside_workspace_tree() {
    let root = Path::new("/srv/fleet");
    let workspace = workspace_for(root, "alice");
    let action_dir = action_dir_for(root, "alice");

    assert_eq!(
        action_dir,
        PathBuf::from("/srv/fleet/.tenant-action-dirs/alice")
    );
    assert!(!action_dir.starts_with(&workspace));
}

#[test]
fn user_id_validation_rejects_path_escapes() {
    assert!(is_valid_user_id("alice"));
    assert!(is_valid_user_id("user_42-x"));
    assert!(!is_valid_user_id(""));
    assert!(!is_valid_user_id("../etc"));
    assert!(!is_valid_user_id("a/b"));
    assert!(!is_valid_user_id("a.b"));
}

#[test]
fn provision_assigns_distinct_ports_and_edge_tokens() {
    let root = PathBuf::from("/tmp/ws");
    let users = vec!["alice".to_string(), "bob".to_string()];
    let (instances, edge_auth, minted) = provision(&users, &root, 7900).unwrap();

    assert_eq!(instances.len(), 2);
    assert_eq!(instances["alice"].port, 7900);
    assert_eq!(instances["bob"].port, 7901);
    assert_eq!(instances["alice"].workspace_dir, root.join("alice"));
    assert_eq!(
        instances["alice"].action_dir,
        root.join(".tenant-action-dirs").join("alice")
    );
    assert_ne!(instances["alice"].action_dir, instances["bob"].action_dir);
    assert_ne!(instances["alice"].core_bearer, instances["bob"].core_bearer);
    assert_eq!(instances["alice"].rpc_url(), "http://127.0.0.1:7900/rpc");

    // Every minted edge token resolves back to exactly its user.
    assert_eq!(edge_auth.len(), 2);
    for (user_id, token) in &minted {
        assert_eq!(edge_auth.user_for(token), Some(user_id.as_str()));
    }
}

#[test]
fn provision_rejects_duplicate_and_invalid_users() {
    let root = PathBuf::from("/tmp/ws");
    assert!(provision(&["a".into(), "a".into()], &root, 7900).is_err());
    assert!(provision(&["../x".into()], &root, 7900).is_err());
}

#[test]
fn bearer_parsing_requires_bearer_prefix() {
    let mut h = HeaderMap::new();
    h.insert(
        axum::http::header::AUTHORIZATION,
        "Bearer edge-123".parse().unwrap(),
    );
    assert_eq!(bearer_from_headers(&h), Some(EdgeToken::new("edge-123")));

    let mut lower = HeaderMap::new();
    lower.insert(
        axum::http::header::AUTHORIZATION,
        "bearer edge-456".parse().unwrap(),
    );
    assert_eq!(
        bearer_from_headers(&lower),
        Some(EdgeToken::new("edge-456"))
    );

    let mut h2 = HeaderMap::new();
    h2.insert(
        axum::http::header::AUTHORIZATION,
        "edge-123".parse().unwrap(),
    );
    assert_eq!(bearer_from_headers(&h2), None);
}

#[test]
fn readiness_body_requires_jsonrpc_result() {
    assert!(readiness_body_succeeded(&serde_json::json!({
        "jsonrpc": "2.0",
        "id": "fleet-ready",
        "result": {"tier": "supervised"}
    })));

    assert!(!readiness_body_succeeded(&serde_json::json!({
        "jsonrpc": "2.0",
        "id": "fleet-ready",
        "error": {"code": -32000, "message": "config unavailable"}
    })));

    assert!(!readiness_body_succeeded(&serde_json::json!({
        "jsonrpc": "2.0",
        "id": "fleet-ready"
    })));
}

#[cfg(unix)]
#[test]
fn edge_token_output_is_written_0600() {
    use std::os::unix::fs::PermissionsExt as _;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("edge-tokens.txt");
    write_edge_tokens(
        &path,
        &[("alice".to_string(), EdgeToken::new("edge-secret"))],
    )
    .unwrap();

    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    assert_eq!(
        std::fs::read_to_string(path).unwrap(),
        "alice edge-secret\n"
    );
}

#[cfg(unix)]
#[test]
fn edge_token_output_rejects_symlinks() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("attacker-readable.txt");
    std::fs::write(&target, "unchanged").unwrap();
    let output = dir.path().join("edge-tokens.txt");
    symlink(&target, &output).unwrap();

    assert!(write_edge_tokens(
        &output,
        &[("alice".to_string(), EdgeToken::new("edge-secret"))],
    )
    .is_err());
    assert_eq!(std::fs::read_to_string(target).unwrap(), "unchanged");
}

#[cfg(unix)]
#[test]
fn edge_token_output_rejects_preowned_files() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("edge-tokens.txt");
    std::fs::write(&output, "attacker keeps this inode").unwrap();

    assert!(write_edge_tokens(
        &output,
        &[("alice".to_string(), EdgeToken::new("edge-secret"))],
    )
    .is_err());
    assert_eq!(
        std::fs::read_to_string(output).unwrap(),
        "attacker keeps this inode"
    );
}

#[test]
fn fleet_cors_allows_tauri_loopback_and_extra_origins() {
    assert!(is_fleet_origin_allowed_with_extra(
        "tauri://localhost",
        None
    ));
    assert!(is_fleet_origin_allowed_with_extra(
        "http://127.0.0.1:1420",
        None
    ));
    assert!(is_fleet_origin_allowed_with_extra(
        "https://fleet.example",
        Some("https://fleet.example")
    ));
    assert!(!is_fleet_origin_allowed_with_extra(
        "https://evil.example",
        Some("https://fleet.example")
    ));
}

#[test]
fn fleet_cors_headers_echo_allowed_origin_only() {
    let allowed = with_fleet_cors_headers(
        StatusCode::NO_CONTENT.into_response(),
        Some("tauri://localhost"),
    );
    assert_eq!(
        allowed
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .and_then(|value| value.to_str().ok()),
        Some("tauri://localhost")
    );
    assert_eq!(
        allowed
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_HEADERS)
            .and_then(|value| value.to_str().ok()),
        Some("Content-Type, Authorization")
    );

    let rejected = with_fleet_cors_headers(
        StatusCode::NO_CONTENT.into_response(),
        Some("https://evil.example"),
    );
    assert!(rejected
        .headers()
        .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
        .is_none());
}

/// Tenant isolation at the edge: alice's token on `/bob/rpc` is refused and
/// never reaches bob's core, while bob's own token is forwarded to it with bob's
/// core bearer. Bob's core is a local mock bound to the port the fleet assigns
/// him, so the test needs no outside network and owns every port it dials.
#[tokio::test]
async fn rpc_proxy_refuses_one_tenants_token_on_another_tenants_core() {
    use std::sync::Mutex;
    let seen: Arc<Mutex<Vec<Option<String>>>> = Arc::default();
    let seen_by_upstream = seen.clone();
    let upstream = axum::Router::new().route(
        "/rpc",
        post(move |headers: HeaderMap| {
            let seen = seen_by_upstream.clone();
            async move {
                seen.lock().unwrap().push(
                    headers
                        .get(header::AUTHORIZATION)
                        .and_then(|value| value.to_str().ok())
                        .map(str::to_owned),
                );
                (StatusCode::OK, r#"{"jsonrpc":"2.0","id":1,"result":"bob"}"#)
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let bob_port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });

    // Bob is provisioned first, so his core gets the mock's port.
    let users = vec!["bob".to_string(), "alice".to_string()];
    let (instances, edge_auth, minted) =
        provision(&users, &PathBuf::from("/tmp/ws"), bob_port).unwrap();
    let bob_core_bearer = instances["bob"].core_bearer.as_str().to_owned();
    let token = |user: &str| {
        minted
            .iter()
            .find(|(id, _)| id == user)
            .map(|(_, token)| token.as_str().to_owned())
            .unwrap()
    };
    let fleet = Arc::new(RwLock::new(Fleet {
        instances,
        edge_auth,
        http: reqwest::Client::new(),
    }));
    let call = |bearer: Option<String>, tenant: &str| {
        let mut headers = HeaderMap::new();
        if let Some(bearer) = bearer {
            headers.insert(
                header::AUTHORIZATION,
                HeaderValue::from_str(&format!("Bearer {bearer}")).unwrap(),
            );
        }
        rpc_proxy(
            State(fleet.clone()),
            AxumPath(tenant.to_owned()),
            headers,
            Bytes::from_static(b"{}"),
        )
    };

    assert_eq!(
        call(Some(token("alice")), "bob").await.status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(call(None, "bob").await.status(), StatusCode::UNAUTHORIZED);
    assert!(
        seen.lock().unwrap().is_empty(),
        "a refused request must never reach bob's core"
    );

    assert_eq!(
        call(Some(token("bob")), "bob").await.status(),
        StatusCode::OK
    );
    assert_eq!(
        *seen.lock().unwrap(),
        vec![Some(format!("Bearer {bob_core_bearer}"))],
        "bob's request reaches his core with his core bearer, not the edge token"
    );
}
