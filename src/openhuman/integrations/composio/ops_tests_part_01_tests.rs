use super::*;

#[test]
fn parse_sync_reason_accepts_known_values() {
    assert_eq!(parse_sync_reason(None).unwrap(), SyncReason::Manual);
    assert_eq!(
        parse_sync_reason(Some("manual")).unwrap(),
        SyncReason::Manual
    );
    assert_eq!(
        parse_sync_reason(Some("periodic")).unwrap(),
        SyncReason::Periodic
    );
    assert_eq!(
        parse_sync_reason(Some("connection_created")).unwrap(),
        SyncReason::ConnectionCreated
    );
}

#[test]
fn parse_sync_reason_rejects_unknown_values() {
    let err = parse_sync_reason(Some("scheduled")).unwrap_err();
    assert!(err.contains("unrecognized sync reason"));
    assert!(err.contains("scheduled"));
    // Typo of a real value should also fail rather than coerce.
    assert!(parse_sync_reason(Some("Periodic")).is_err());
    assert!(parse_sync_reason(Some("")).is_err());
}

#[test]
fn resolve_client_errors_without_session() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(&tmp);
    // `ComposioClient` intentionally doesn't implement `Debug` — use a
    // pattern match instead of `.unwrap_err()`.
    let Err(err) = resolve_client(&config) else {
        panic!("expected auth error when no session is stored");
    };
    assert!(err.contains("composio unavailable"));
    assert!(err.contains("auth_store_session"));
}

#[tokio::test]
async fn composio_list_toolkits_errors_without_session() {
    let _serialised = module_guard().await;
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(&tmp);
    let err = composio_list_toolkits(&config).await.unwrap_err();
    // Backend mode (the default) with no session. What matters is that the
    // call *fails* rather than quietly answering with an empty list, and that
    // the message tells the user what to do about it. The wording moved into
    // the connector module when the client did — it now reports the missing
    // route — so this asserts the contract, not the phrasing.
    assert!(
        err.to_lowercase().contains("composio"),
        "the error should name the domain: {err}"
    );
    assert!(
        err.contains("no backend session") || err.contains("unavailable") || err.contains("route"),
        "the error should say what is missing: {err}"
    );
}

#[tokio::test]
async fn composio_list_capabilities_does_not_require_session() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(&tmp);
    let outcome = composio_list_capabilities(&config).await.unwrap();
    assert!(outcome
        .value
        .capabilities
        .iter()
        .any(|entry| { entry.toolkit == "gmail" && entry.native_provider && entry.memory_ingest }));
    // Capabilities now come from the connector module, rather than the old
    // host-side TinyMemory provider matrix. The module's current contract has
    // no Google Calendar row; keep this regression focused on the sessionless
    // compiled capability that consumers rely on.
    assert!(outcome
        .value
        .capabilities
        .iter()
        .any(|entry| { entry.toolkit == "gmail" && entry.tool_execution }));
}

#[tokio::test]
async fn composio_list_connections_is_quietly_unavailable_without_session() {
    let _serialised = module_guard().await;
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(&tmp);
    // Backend mode (the default) with no app-session JWT is the fresh-install /
    // signed-out state, not a failure. The connector module has no proxy route
    // to be given, so before #6176 this call reached the module and reported its
    // "loaded without a connector route" answer at error level on every boot
    // and periodic tick. Now the guard answers first with the same "no backend
    // session token" error every other member gives here: still an `Err` — the
    // connections may exist server-side, so "unavailable" is the truthful
    // answer and the reconcile / flows callers keep their fail-open handling —
    // but nothing is reported to Sentry, and the JSON-RPC boundary demotes the
    // wording as expected user state (`is_session_expired_message`).
    let err = composio_list_connections(&config).await.unwrap_err();
    assert!(
        err.contains("no backend session token"),
        "the error should say what is missing: {err}"
    );
    // The module's own wording is what used to be reported at error level; its
    // absence proves the call never reached the module (and so never reached
    // `report_composio_op_error`).
    assert!(
        !err.contains("loaded without a connector route"),
        "the guard must answer before the connector module does: {err}"
    );
}

#[tokio::test]
async fn composio_authorize_errors_without_session() {
    let _serialised = module_guard().await;
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(&tmp);
    let err = composio_authorize(&config, "gmail", None)
        .await
        .unwrap_err();
    // Backend mode (default) without a session — the mode-aware factory
    // surfaces "no backend session token" once `composio_authorize`
    // routes through `create_composio_client`. Accept either the
    // legacy `composio unavailable` prefix or the new factory phrasing.
    assert!(
        err.to_lowercase().contains("composio")
            && (err.contains("no backend session")
                || err.contains("unavailable")
                || err.contains("route")),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn composio_delete_connection_errors_without_session() {
    let _serialised = module_guard().await;
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(&tmp);
    let err = composio_delete_connection(&config, "c-1", false)
        .await
        .unwrap_err();
    assert!(
        err.to_lowercase().contains("composio")
            && (err.contains("unavailable") || err.contains("route")),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn composio_list_tools_errors_without_session() {
    let _serialised = module_guard().await;
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(&tmp);
    let err = composio_list_tools(&config, None, None).await.unwrap_err();
    // Same contract as `composio_list_toolkits_errors_without_session`.
    assert!(
        err.to_lowercase().contains("composio"),
        "the error should name the domain: {err}"
    );
    assert!(
        err.contains("no backend session") || err.contains("unavailable") || err.contains("route"),
        "the error should say what is missing: {err}"
    );
}

#[tokio::test]
async fn composio_execute_errors_without_session() {
    let _serialised = module_guard().await;
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(&tmp);
    let err = composio_execute(&config, "GMAIL_SEND_EMAIL", None, None)
        .await
        .unwrap_err();
    // What matters is that an action with no credential *fails* rather than
    // reporting a send that never happened. The wording moved into the
    // connector module with the client — it now names the missing route — so
    // this asserts the contract, not the phrasing.
    assert!(
        err.to_lowercase().contains("composio")
            && (err.contains("no backend session")
                || err.contains("unavailable")
                || err.contains("route")),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn composio_get_user_profile_errors_without_session() {
    let _serialised = module_guard().await;
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(&tmp);
    let err = composio_get_user_profile(&config, "c-1").await.unwrap_err();
    // Fails while resolving which toolkit `c-1` belongs to, because that needs
    // the connection list and the connection list needs a credential. Still a
    // failure that names the domain, which is the contract here.
    assert!(err.to_lowercase().contains("composio"), "{err}");
}

#[tokio::test]
async fn composio_sync_errors_without_session() {
    let _serialised = module_guard().await;
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(&tmp);
    let err = composio_sync(&config, "c-1", None).await.unwrap_err();
    assert!(err.to_lowercase().contains("composio"), "{err}");
}

#[tokio::test]
async fn composio_sync_rejects_invalid_reason_before_client_check() {
    let _serialised = module_guard().await;
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(&tmp);
    // Invalid reason → should fail at parse step *before* touching the
    // client, so the error message references the reason, not auth.
    let err = composio_sync(&config, "c-1", Some("weird".into()))
        .await
        .unwrap_err();
    assert!(err.contains("unrecognized sync reason"));
}

#[tokio::test]
async fn composio_list_trigger_history_errors_when_store_not_init() {
    let _serialised = module_guard().await;
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(&tmp);
    // The archive moved into the module, which is what writes to it as
    // deliveries are dispatched. A module with no state directory has none, so
    // reading history reports that rather than answering with an empty list —
    // "no triggers have fired" and "nothing is recording them" are different
    // things to tell someone debugging a trigger.
    let err = composio_list_trigger_history(&config, Some(10))
        .await
        .unwrap_err();
    assert!(err.contains("list_trigger_history failed"), "{err}");
}

#[test]
fn cache_key_is_based_on_config_path_string() {
    let tmp = tempfile::tempdir().unwrap();
    let mut a = Config::default();
    a.config_path = tmp.path().join("a.toml");
    let mut b = Config::default();
    b.config_path = tmp.path().join("b.toml");
    assert_ne!(cache_key(&a), cache_key(&b));
    assert_eq!(cache_key(&a), cache_key(&a));
}

#[tokio::test]
async fn fetch_connected_integrations_returns_empty_without_auth() {
    let _guard = cache_guard();
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(&tmp);
    let integrations = fetch_connected_integrations(&config).await;
    assert!(integrations.is_empty());
}

#[test]
fn invalidate_connected_integrations_cache_is_safe_without_prior_insert() {
    let _guard = cache_guard();
    // Must not panic on an empty cache.
    invalidate_connected_integrations_cache();
    invalidate_connected_integrations_cache();
}

#[tokio::test]
async fn composio_list_toolkits_via_mock() {
    let _serialised = module_guard().await;
    let app = Router::new().route(
        "/agent-integrations/composio/toolkits",
        get(|| async { Json(json!({"success": true, "data": {"toolkits": ["gmail"]}})) }),
    );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_backend(&tmp, base);
    let outcome = composio_list_toolkits(&config).await.unwrap();
    assert_eq!(outcome.value.toolkits, vec!["gmail".to_string()]);
    assert!(outcome.logs.iter().any(|l| l.contains("toolkit")));
}

#[tokio::test]
async fn composio_list_connections_via_mock_counts_active() {
    let _serialised = module_guard().await;
    let app = Router::new().route(
        "/agent-integrations/composio/connections",
        get(|| async {
            Json(json!({
                "success": true,
                "data": {"connections": [
                    {"id":"c1","toolkit":"gmail","status":"ACTIVE"},
                    {"id":"c2","toolkit":"notion","status":"PENDING"},
                    {"id":"c3","toolkit":"gmail","status":"CONNECTED"}
                ]}
            }))
        }),
    );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_backend(&tmp, base);
    let outcome = composio_list_connections(&config).await.unwrap();
    assert_eq!(outcome.value.connections.len(), 3);
    // 2 active, 3 total
    assert!(outcome.logs.iter().any(|l| l.contains("3 connection")));
    assert!(outcome.logs.iter().any(|l| l.contains("2 active")));
}

#[cfg(feature = "modules")]
#[tokio::test]
async fn composio_list_connections_drops_the_module_route_once_signed_out() {
    use crate::openhuman::integrations::composio::module_client::methods;
    use crate::openhuman::modules::connectors::{last_route_is_none, proxy_without_reconcile};

    let _serialised = module_guard().await;
    let app = Router::new().route(
        "/agent-integrations/composio/connections",
        get(|| async {
            Json(json!({
                "success": true,
                "data": {"connections": [{"id":"c1","toolkit":"gmail","status":"ACTIVE"}]}
            }))
        }),
    );
    let base = start_mock_backend(app).await;
    let signed_in_tmp = tempfile::tempdir().unwrap();
    let signed_in = config_with_backend(&signed_in_tmp, base);
    // Signed in: the module is loaded and routed to the mock backend.
    let outcome = composio_list_connections(&signed_in).await.unwrap();
    assert_eq!(outcome.value.connections.len(), 1);
    assert!(
        !last_route_is_none(),
        "the signed-in call must have routed the module"
    );

    // Signed out: a config whose auth store holds no session. The guard
    // answers without a module call — but the module still holds the
    // signed-in bearer, and telling it to drop that is the host's job (the
    // module chooses no route on its own), so the guard must do so before
    // answering.
    let signed_out_tmp = tempfile::tempdir().unwrap();
    let signed_out = test_config(&signed_out_tmp);
    let err = composio_list_connections(&signed_out).await.unwrap_err();
    assert!(err.contains("no backend session token"), "{err}");
    assert!(
        last_route_is_none(),
        "the module must have been told to drop its route"
    );
    // And the module really did drop it: a proxy that does NOT reconcile the
    // route first gets the module's own no-route answer, not the mock's list.
    let proxy = proxy_without_reconcile()
        .await
        .expect("the module is serving");
    let err = proxy
        .call::<crate::openhuman::integrations::composio::types::ComposioConnectionsResponse>(
            methods::LIST_CONNECTIONS,
            (),
        )
        .await
        .expect_err("a module without a route cannot list connections");
    assert!(
        err.to_string().contains("without a connector route"),
        "{err}"
    );
}

#[tokio::test]
async fn composio_authorize_clears_pending_meta_connection_before_handoff() {
    let _serialised = module_guard().await;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    let deletes = Arc::new(AtomicUsize::new(0));
    let deletes_for_delete = Arc::clone(&deletes);
    let app = Router::new()
        .route(
            "/agent-integrations/composio/connections",
            get(|| async {
                Json(json!({
                    "success": true,
                    "data": {"connections": [
                        {"id":"ig-pending","toolkit":"instagram","status":"PENDING"}
                    ]}
                }))
            }),
        )
        .route(
            "/agent-integrations/composio/connections/{id}",
            axum::routing::delete(move |Path(id): Path<String>| {
                let deletes = Arc::clone(&deletes_for_delete);
                async move {
                    if id == "ig-pending" {
                        deletes.fetch_add(1, Ordering::SeqCst);
                    }
                    Json(json!({"success": true, "data": {"deleted": true}}))
                }
            }),
        )
        .route(
            "/agent-integrations/composio/authorize",
            post(|Json(body): Json<Value>| async move {
                assert_eq!(body["toolkit"], "instagram");
                Json(json!({
                    "success": true,
                    "data": {
                        "connectUrl": "https://meta.example/oauth",
                        "connectionId": "c-new"
                    }
                }))
            }),
        );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_backend(&tmp, base);
    let outcome = composio_authorize(&config, "instagram", None)
        .await
        .unwrap();
    assert_eq!(outcome.value.connection_id, "c-new");
    assert_eq!(deletes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn composio_authorize_via_mock_publishes_event_and_returns_url() {
    let _serialised = module_guard().await;
    let app = Router::new().route(
        "/agent-integrations/composio/authorize",
        post(|Json(_b): Json<Value>| async move {
            Json(json!({
                "success": true,
                "data": {"connectUrl": "https://x", "connectionId": "c1"}
            }))
        }),
    );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_backend(&tmp, base);
    let outcome = composio_authorize(&config, "gmail", None).await.unwrap();
    assert_eq!(outcome.value.connect_url, "https://x");
    assert_eq!(outcome.value.connection_id, "c1");
}

#[tokio::test]
async fn composio_delete_connection_via_mock() {
    let _serialised = module_guard().await;
    let app = Router::new().route(
        "/agent-integrations/composio/connections/{id}",
        axum::routing::delete(|Path(_id): Path<String>| async move {
            Json(json!({"success": true, "data": {"deleted": true}}))
        }),
    );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_backend(&tmp, base);
    let outcome = composio_delete_connection(&config, "c1", false)
        .await
        .unwrap();
    assert!(outcome.value.deleted);
}
