use super::*;

#[tokio::test]
async fn composio_set_api_key_rejects_invalid_direct_key_before_persisting() {
    use crate::openhuman::config::TEST_ENV_LOCK;
    use crate::openhuman::security::credentials::get_composio_api_key;

    let _env_guard = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let app = Router::new().route(
        "/connected_accounts",
        get(|| async {
            (
                axum::http::StatusCode::UNAUTHORIZED,
                Json(json!({ "error": { "message": "Invalid API key" } })),
            )
        }),
    );
    let base = start_mock_backend(app).await;
    let _base_v2 = EnvVarGuard::set("OPENHUMAN_COMPOSIO_DIRECT_BASE_V2", &base);
    let _base_v3 = EnvVarGuard::set("OPENHUMAN_COMPOSIO_DIRECT_BASE_V3", &base);

    let tmp = tempfile::tempdir().unwrap();
    let config = direct_mode_no_key_config(&tmp);
    let _auth_guard = DirectAuthFailureGuard::new("ck_invalid_direct");

    let err = composio_set_api_key(&config, "ck_invalid_direct", false)
        .await
        .expect_err("invalid direct-mode key must be rejected before persistence");
    assert!(
        err.contains("Invalid Composio API key"),
        "unexpected error: {err}"
    );
    assert_eq!(
        get_composio_api_key(&config).expect("read composio key after failed save"),
        None,
        "invalid key must not be stored"
    );
}

#[tokio::test]
async fn composio_set_api_key_validates_candidate_key_even_when_stored_key_exists() {
    use crate::openhuman::config::TEST_ENV_LOCK;
    use crate::openhuman::security::credentials::{get_composio_api_key, store_composio_api_key};
    use std::sync::{Arc, Mutex};

    let _env_guard = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let seen_keys = Arc::new(Mutex::new(Vec::<String>::new()));
    let app = Router::new()
        .route(
            "/connected_accounts",
            get(
                |State(seen_keys): State<Arc<Mutex<Vec<String>>>>, headers: HeaderMap| async move {
                    let key = headers
                        .get("x-api-key")
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or_default()
                        .to_string();
                    seen_keys.lock().unwrap().push(key.clone());
                    if key == "ck_old_valid" {
                        (axum::http::StatusCode::OK, Json(json!({ "items": [] })))
                    } else {
                        (
                            axum::http::StatusCode::UNAUTHORIZED,
                            Json(json!({ "error": { "message": "Invalid API key" } })),
                        )
                    }
                },
            ),
        )
        .with_state(seen_keys.clone());
    let base = start_mock_backend(app).await;
    let _base_v2 = EnvVarGuard::set("OPENHUMAN_COMPOSIO_DIRECT_BASE_V2", &base);
    let _base_v3 = EnvVarGuard::set("OPENHUMAN_COMPOSIO_DIRECT_BASE_V3", &base);

    let tmp = tempfile::tempdir().unwrap();
    let config = direct_mode_no_key_config(&tmp);
    store_composio_api_key(&config, "ck_old_valid")
        .await
        .expect("seed old stored composio key");
    let _new_key_guard = DirectAuthFailureGuard::new("ck_new_invalid");

    let err = composio_set_api_key(&config, "ck_new_invalid", false)
        .await
        .expect_err("candidate invalid key must be rejected even if old stored key is valid");
    assert!(
        err.contains("Invalid Composio API key"),
        "unexpected error: {err}"
    );
    assert_eq!(
        get_composio_api_key(&config).expect("read composio key after failed replacement"),
        Some("ck_old_valid".to_string()),
        "failed replacement must leave the old stored key intact"
    );
    assert!(
        seen_keys
            .lock()
            .unwrap()
            .iter()
            .any(|key| key == "ck_new_invalid"),
        "validation must probe the candidate key even when other parallel direct-mode tests \
         share the process-wide mock base URL"
    );
}

// ── Direct-mode authorize / list_tools / execute (commit 1, #1710) ─

/// Direct-mode `composio_list_tools` now hits Composio v3 with the
/// user's own key (replacing the prior empty-short-circuit). The unit
/// test reaches an outbound HTTPS call against the real
/// `backend.composio.dev`, which immediately fails with HTTP 401 on the
/// fake key — exactly the shape we want the contract to preserve:
///
///   * NEVER fall back to the tinyhumans backend tenant (no
///     `"no backend session"` text in the error)
///   * Surface the failure with the `composio` grep prefix so it routes
///     through normal observability
///
/// A full schemas-mapped test that asserts response shape lives in the
/// `client_tests.rs` mock-axum suite (`direct_list_tools_*`); this
/// integration-style test only pins the failure-mode contract.
#[tokio::test]
async fn composio_list_tools_in_direct_mode_does_not_fall_back_to_backend() {
    let _serialised = module_guard().await;
    let tmp = tempfile::tempdir().unwrap();
    let config = direct_mode_config(&tmp);
    let result = composio_list_tools(&config, None, None).await;
    match result {
        Ok(outcome) => {
            // If the prefetch returns empty connections (test env may
            // intermittently mock that), the function short-circuits to
            // an empty tool list — still no backend leak.
            assert!(
                outcome.value.tools.is_empty(),
                "direct mode must NOT surface backend-tenant tool catalogue"
            );
            assert!(
                outcome.logs.iter().any(|l| l.contains("direct mode")),
                "log line must call out direct mode explicitly, got {:?}",
                outcome.logs
            );
        }
        Err(err) => {
            assert!(
                !err.contains("no backend session"),
                "direct mode must not surface backend-auth errors, got: {err}"
            );
            assert!(
                err.to_lowercase().contains("composio"),
                "error must carry the composio prefix, got: {err}"
            );
        }
    }
}

#[tokio::test]
async fn composio_authorize_routes_through_direct_mode() {
    let _serialised = module_guard().await;
    // The direct-mode `authorize` path actually calls
    // `backend.composio.dev/api/v3/connected_accounts/link` over HTTPS.
    // We can't mock that endpoint at the URL-rewriter level in this
    // unit test, so the assertion below verifies (a) the mode-aware
    // factory was hit (i.e. no "no backend session" error) and (b) the
    // error path is the direct-mode one (HTTP failure or DNS failure),
    // not the backend one. Both error shapes are acceptable — the
    // important thing is that backend mode would have produced
    // "composio unavailable / no backend session" instead.
    let tmp = tempfile::tempdir().unwrap();
    let config = direct_mode_config(&tmp);
    let err = composio_authorize(&config, "gmail", None)
        .await
        .unwrap_err();
    assert!(
        !err.contains("no backend session"),
        "direct mode must not surface backend-auth errors, got: {err}"
    );
    assert!(
        err.to_lowercase().contains("composio"),
        "error must carry the composio prefix, got: {err}"
    );
}

#[tokio::test]
async fn composio_execute_routes_through_direct_mode() {
    let _serialised = module_guard().await;
    // Same shape of assertion as
    // `composio_authorize_routes_through_direct_mode` — we can't mock
    // `backend.composio.dev` from a unit test, so we verify the factory
    // routed to direct mode (no backend-auth error) and that an error
    // surfaces because the live HTTP call cannot succeed against a
    // test key.
    let tmp = tempfile::tempdir().unwrap();
    let config = direct_mode_config(&tmp);
    let err = composio_execute(&config, "GMAIL_SEND_EMAIL", None, None)
        .await
        .unwrap_err();
    assert!(
        !err.contains("no backend session"),
        "direct mode must not surface backend-auth errors, got: {err}"
    );
    assert!(
        err.to_lowercase().contains("composio"),
        "error must carry the composio prefix, got: {err}"
    );
}

// ── classify_composio_failure_tag ──────────────────────────────
//
// Pin the failure-tag routing for `report_composio_op_error` so the
// `before_send` filter (`is_transient_integrations_failure` extended to
// `domain="composio"` in the same #1608 patch series) matches. The tag
// drives which branch of the filter fires:
//   - `failure="non_2xx"` + transient `status` (set by the integrations
//     wrapper) → dropped
//   - `failure="transport"` + transient transport phrase in the message
//     → dropped
// Any drift between the helper's classification and the filter's
// expectations would silently re-open the leak path.

#[test]
fn composio_failure_tag_is_non_2xx_for_backend_returned_502() {
    // OPENHUMAN-TAURI-35 / -2H wire shape — the dominant leak. The
    // integrations layer renders this on a 5xx response; composio's op
    // layer wraps the chain and re-reports under `domain=composio`. The
    // tag MUST be `non_2xx` so the existing transient-status filter
    // branch matches.
    let rendered = "Backend returned 502 Bad Gateway for POST \
                    https://api.tinyhumans.ai/agent-integrations/composio/connections: \
                    upstream temporarily unavailable";
    assert_eq!(classify_composio_failure_tag(rendered), "non_2xx");
}

#[test]
fn composio_failure_tag_is_non_2xx_for_envelope_error() {
    // Envelope errors don't carry a transport phrase or "error sending
    // request" anchor; default to non_2xx.
    let rendered = "Backend error for POST https://api.tinyhumans.ai/x: \
                    unknown backend error";
    assert_eq!(classify_composio_failure_tag(rendered), "non_2xx");
}

#[test]
fn composio_failure_tag_is_transport_for_operation_timed_out() {
    // OPENHUMAN-TAURI-18 / -G shape — `composio/execute` reqwest chain
    // surfaces `operation timed out` (one of `TRANSIENT_TRANSPORT_PHRASES`).
    // Tag MUST be `transport` so the filter's transport-phrase branch fires
    // even though the report carries no `status`.
    let rendered = "POST https://api.tinyhumans.ai/agent-integrations/composio/execute \
                    failed: error sending request for url \
                    (https://api.tinyhumans.ai/agent-integrations/composio/execute) → \
                    client error (SendRequest) → connection error → \
                    Operation timed out (os error 60)";
    assert_eq!(classify_composio_failure_tag(rendered), "transport");
}

#[test]
fn composio_failure_tag_is_transport_for_dns_and_tls_phrases() {
    for raw in [
        "POST /v1/foo failed: error sending request for url (https://api.example.com/x)",
        "GET /agent-integrations/composio/connections failed: tls handshake eof",
        "POST /agent-integrations/composio/triggers failed: connection reset by peer",
        "GET /agent-integrations/composio/toolkits failed: connection forcibly closed (os 10054)",
    ] {
        assert_eq!(
            classify_composio_failure_tag(raw),
            "transport",
            "should classify as transport: {raw}"
        );
    }
}

#[test]
fn composio_failure_tag_does_not_misclassify_unrelated_messages() {
    // A bare error string with no transport / "error sending request"
    // anchor must default to non_2xx — the safe choice for the dominant
    // leak shape.
    for raw in [
        "[composio] no connection with id 'abc'",
        "[composio] no native provider registered for toolkit 'foo'",
        "fetch_user_profile failed: invalid JSON in profile facet",
    ] {
        assert_eq!(
            classify_composio_failure_tag(raw),
            "non_2xx",
            "should default to non_2xx: {raw}"
        );
    }
}

// ── extract_backend_returned_status ───────────────────────────
//
// Pin status extraction so the `report_composio_op_error` Sentry tag
// stays in lockstep with the `Backend returned <status> ...` rendering
// the integrations layer produces. Without the digit anchor the
// `before_send` filter's transient-status branch can't distinguish a 502
// from a 401, and the dominant leak shape (OPENHUMAN-TAURI-35 / -2H)
// re-opens.

#[test]
fn extract_backend_returned_status_parses_three_digit_status() {
    let rendered = "Backend returned 502 Bad Gateway for POST \
                    https://api.tinyhumans.ai/agent-integrations/composio/connections: \
                    upstream temporarily unavailable";
    assert_eq!(
        extract_backend_returned_status(rendered),
        Some("502".to_string())
    );
}

#[test]
fn extract_backend_returned_status_returns_none_when_no_status() {
    // Envelope-style error with no HTTP status digits after the anchor.
    let rendered = "Backend returned bad gateway (envelope-only error)";
    assert_eq!(extract_backend_returned_status(rendered), None);
}

#[test]
fn extract_backend_returned_status_handles_mixed_case() {
    // Some renders upper-case the prefix; the helper lowercases before
    // matching so both shapes resolve to the same status string.
    let rendered = "BACKEND RETURNED 429 Too Many Requests for GET \
                    https://api.tinyhumans.ai/agent-integrations/composio/triggers";
    assert_eq!(
        extract_backend_returned_status(rendered),
        Some("429".to_string())
    );
}

// ── before_send filter integration ─────────────────────────────
//
// Belt-and-suspenders: re-assert the cross-module contract from the
// composio side. If `is_transient_integrations_failure` ever stops
// matching `domain="composio"` (e.g. accidental revert), the
// `report_composio_op_error` events flood Sentry again with no test in
// the composio crate to catch it. These guards make the link explicit.

#[cfg(feature = "crash-reporting")]
#[test]
fn composio_domain_502_is_dropped_by_before_send() {
    let mut event = sentry::protocol::Event::default();
    let mut tags: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    tags.insert("domain".into(), "composio".into());
    tags.insert("failure".into(), "non_2xx".into());
    tags.insert("status".into(), "502".into());
    event.tags = tags;
    assert!(
        crate::core::observability::is_transient_integrations_failure(&event),
        "composio non_2xx 502 must be dropped by integrations filter (#1608)"
    );
}

#[cfg(feature = "crash-reporting")]
#[test]
fn composio_transport_timeout_is_dropped_by_before_send() {
    let mut event = sentry::protocol::Event::default();
    let mut tags: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    tags.insert("domain".into(), "composio".into());
    tags.insert("failure".into(), "transport".into());
    event.tags = tags;
    event.message = Some(
        "POST /agent-integrations/composio/execute failed: error sending request → \
         operation timed out"
            .to_string(),
    );
    assert!(
        crate::core::observability::is_transient_integrations_failure(&event),
        "composio transport timeout must be dropped by integrations filter (#1608)"
    );
}

// ── TAURI-RUST-X9 (#1166): direct-mode auth-rejection routing ───────────
//
// Pins the contract that direct-mode 401 / Invalid API key shapes are
// classified by the observability matcher AND their failure-tag stays
// `non_2xx` so the `before_send` integrations filter has consistent
// inputs. Together with the classifier-arm tests in
// `core::observability` these tests prove the leak path (~15.7 k events
// in ~22h before #1166) is closed end-to-end.

#[test]
fn composio_direct_invalid_api_key_classifies_as_provider_user_state() {
    // The verbatim Sentry TAURI-RUST-X9 wire shape — emitted by
    // `ops.rs::composio_list_connections` direct branch via the
    // `report_composio_op_error` hook restored in #1166. Routing this
    // through `expected_error_kind` is what demotes it to
    // `ProviderUserState` (info breadcrumb) instead of firing a Sentry
    // event.
    let msg = "[composio-direct] list_connections failed: \
               Composio v3 connected_accounts failed: \
               HTTP 401: Invalid API key: ak_VsUvq*****";
    assert_eq!(
        crate::core::observability::expected_error_kind(msg),
        Some(crate::core::observability::ExpectedErrorKind::ProviderUserState),
        "the canonical TAURI-RUST-X9 wire shape must demote via the composio-direct arm"
    );
}

#[test]
fn composio_direct_invalid_api_key_failure_tag_is_non_2xx() {
    // Belt-and-suspenders: even if `expected_error_kind` ever stops
    // matching the body (regression in the classifier arm), the
    // failure tag must STILL be `non_2xx`. Combined with the
    // `before_send` filter's transient-status handling and a
    // future-added `status="401"` tag (Patch 1 doesn't extract status
    // from the `HTTP 401` shape — only the `Backend returned <status>`
    // shape — so this just pins the safe default), this is the
    // backstop drop path.
    let rendered = "[composio-direct] list_connections failed: \
                    Composio v3 connected_accounts failed: \
                    HTTP 401: Invalid API key: ak_VsUvq*****";
    assert_eq!(
        classify_composio_failure_tag(rendered),
        "non_2xx",
        "direct-mode auth-rejection must tag as non_2xx (not transport)"
    );
}

#[test]
fn composio_direct_invalid_api_key_extract_status_returns_none() {
    // Pins the contract: `extract_backend_returned_status` only parses
    // the integrations-layer `Backend returned <status>` rendering, NOT
    // the direct-mode `HTTP 401` shape. The direct-mode arm relies on
    // the classifier demotion + the failure-tag drop path instead of
    // status extraction; if this ever changes (e.g. we extend the
    // status extractor to cover both shapes), the new behaviour should
    // come with an explicit test, not be inferred.
    let rendered = "[composio-direct] list_connections failed: \
                    Composio v3 connected_accounts failed: \
                    HTTP 401: Invalid API key: ak_…";
    assert_eq!(
        extract_backend_returned_status(rendered),
        None,
        "direct-mode HTTP 401 must not parse via extract_backend_returned_status"
    );
}

#[test]
fn composio_direct_500_does_not_demote() {
    // Discrimination test from the composio side — a real bug shape
    // (500 with no auth body) MUST escape the classifier and reach
    // `report_error_message`. Without this guard the matcher in
    // `observability.rs` could be tightened too far and silence
    // genuine backend faults.
    let msg = "[composio-direct] list_connections failed: \
               Composio v3 connected_accounts failed: HTTP 500";
    assert_eq!(
        crate::core::observability::expected_error_kind(msg),
        None,
        "composio-direct 500 with no auth body must remain an unclassified bug shape"
    );
}

#[tokio::test]
async fn enrich_does_nothing_when_no_cached_identities() {
    // `enrich_connections_with_identity` reads through the bound memory
    // driver now (`identity_store::load_connected_identities`), not the
    // deleted engine's process-global client — see the module doc comment on
    // `identity_store`. The fresh temp workspace has no profiles, so it
    // returns `Vec::new()` and the connection is returned unchanged.
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(&tmp);
    crate::openhuman::memory::test_support::install_memory_driver_for_test(&config);
    let resp = make_connections_response(&[("c1", "gmail", "ACTIVE")]);
    let enriched = enrich_connections_with_identity(&config, resp).await;
    assert_eq!(enriched.connections.len(), 1);
    assert!(enriched.connections[0].account_email.is_none());
    assert!(enriched.connections[0].workspace.is_none());
    assert!(enriched.connections[0].username.is_none());
}

#[tokio::test]
async fn enrich_skips_connection_already_having_identity() {
    // If the backend-proxied path already populated account_email, the
    // enricher must NOT overwrite it with a potentially stale cached value.
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(&tmp);
    crate::openhuman::memory::test_support::install_memory_driver_for_test(&config);

    let mut resp = make_connections_response(&[("c-preloaded", "gmail", "ACTIVE")]);
    resp.connections[0].account_email = Some("preloaded@example.com".to_string());

    let enriched = enrich_connections_with_identity(&config, resp).await;
    assert_eq!(
        enriched.connections[0].account_email.as_deref(),
        Some("preloaded@example.com"),
        "pre-populated identity must not be overwritten"
    );
}

#[tokio::test]
async fn enrich_leaves_unmatched_connection_unchanged() {
    // Connection whose id has no cached profile row is returned with all
    // identity fields as None — the UI falls back to "toolkit · connection_id".
    use crate::openhuman::integrations::composio::identity_store::persist_provider_profile;
    use tinymemory_api::composio::ProviderUserProfile;

    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(&tmp);
    crate::openhuman::memory::test_support::install_memory_driver_for_test(&config);

    // Persist a profile for a DIFFERENT connection id.
    persist_provider_profile(
        &config,
        &ProviderUserProfile {
            toolkit: "gmail".to_string(),
            connection_id: Some("other-conn".to_string()),
            email: Some("other@example.com".to_string()),
            ..Default::default()
        },
    )
    .await
    .expect("persist provider profile");

    let resp = make_connections_response(&[("no-profile-conn", "gmail", "ACTIVE")]);
    let enriched = enrich_connections_with_identity(&config, resp).await;

    assert!(
        enriched.connections[0].account_email.is_none(),
        "connection with no cached profile must remain unenriched"
    );
}

/// A run that wrote nothing because the day's request budget was spent must
/// say so: the UI shows "Up to date" for a zero count, and a spent budget is
/// the opposite. The note rides after the count so the parse contract holds,
/// and a blank note is no separator with nothing behind it.
#[test]
fn completed_detail_carries_the_module_note_after_the_count() {
    let re = regex::Regex::new(r"(?i)ingested\s+(\d+)\s+item").expect("ui parse regex");
    let detail = crate::openhuman::integrations::composio::ops::completed_sync_detail(
        0,
        true,
        Some("today's provider request budget is spent"),
    );
    let caps = re.captures(&detail).expect("detail still parses");
    assert_eq!(&caps[1], "0");
    assert!(
        detail.ends_with("; today's provider request budget is spent"),
        "{detail}"
    );
    let bare =
        crate::openhuman::integrations::composio::ops::completed_sync_detail(3, false, Some("   "));
    assert_eq!(bare, "ingested 3 item(s)");
}

/// The per-source depth cap is matched the way the engine keys the rows and a
/// zero reads as "no cap": the settings field stores unlimited as empty, and a
/// zero typed by hand must not ask for mail newer than today.
#[test]
fn source_depth_matches_the_row_and_treats_zero_as_unbounded() {
    use crate::openhuman::integrations::composio::ops::pick_source_sync_depth_days;
    let rows = [
        (Some("gmail"), Some("conn-1"), Some(30)),
        (Some("gmail"), Some("conn-2"), Some(0)),
        (Some("notion"), Some("conn-3"), Some(14)),
        (Some("gmail"), None, Some(7)),
    ];
    assert_eq!(
        pick_source_sync_depth_days(rows, "gmail", "conn-1"),
        Some(30)
    );
    assert_eq!(
        pick_source_sync_depth_days(rows, " GMAIL ", "conn-1 "),
        Some(30)
    );
    assert_eq!(pick_source_sync_depth_days(rows, "gmail", "conn-2"), None);
    assert_eq!(pick_source_sync_depth_days(rows, "gmail", "conn-9"), None);
    assert_eq!(
        pick_source_sync_depth_days(rows, "notion", "conn-3"),
        Some(14)
    );
    assert_eq!(pick_source_sync_depth_days([], "gmail", "conn-1"), None);
}

// ── Backend mode with no session yet (#6176) ──────────────────────────
//
// The twin of the direct-mode-without-key guard: backend mode (the default)
// with no app-session JWT is the fresh-install / signed-out state.
// `composio_list_connections` must answer with an empty list instead of
// letting the connector module report "loaded without a connector route",
// which the boot-time memory-source reconcile and the 60 s periodic tick
// would otherwise turn into an error-level report and a Sentry event.

/// Store an app-session JWT in the auth store `config` points at, the same
/// way `client_tests::config_with_session_token` does.
fn store_test_session_token(config: &crate::openhuman::config::Config) {
    crate::openhuman::security::credentials::AuthService::from_config(config)
        .store_provider_token(
            crate::openhuman::security::credentials::APP_SESSION_PROVIDER,
            crate::openhuman::security::credentials::DEFAULT_AUTH_PROFILE_NAME,
            "test-session-token",
            std::collections::HashMap::new(),
            true,
        )
        .expect("store test session token");
}

#[test]
fn backend_mode_without_session_is_true_for_default_mode_and_no_session() {
    let tmp = tempfile::tempdir().unwrap();
    // `Config::default()` leaves `composio.mode` empty, which is backend mode.
    let config = test_config(&tmp);
    assert!(backend_mode_without_session(&config));
}

#[test]
fn backend_mode_without_session_is_true_for_explicit_backend_mode() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = test_config(&tmp);
    config.composio.mode = crate::openhuman::config::schema::COMPOSIO_MODE_BACKEND.into();
    assert!(backend_mode_without_session(&config));
}

#[test]
fn backend_mode_without_session_is_false_in_direct_mode() {
    let tmp = tempfile::tempdir().unwrap();
    // Direct mode never needs a session; its no-key state belongs to
    // `direct_mode_without_key`, and the two guards must not overlap.
    let config = direct_mode_no_key_config(&tmp);
    assert!(!backend_mode_without_session(&config));
}

#[test]
fn backend_mode_without_session_is_false_once_signed_in() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(&tmp);
    store_test_session_token(&config);
    // A stored session means the module gets a proxy route — the guard must
    // step aside, or a signed-in user would see a silent empty list.
    assert!(!backend_mode_without_session(&config));
}

#[test]
fn backend_mode_without_session_is_false_when_the_session_store_is_unreadable() {
    let tmp = tempfile::tempdir().unwrap();
    // The auth-profile store lives in `config_path.parent()`
    // (`state_dir_from_config`); a regular file there makes it fail to load,
    // the same trick `auth_profile_lock_errors_do_not_include_local_paths`
    // uses. A lookup that *failed* is not "signed out": the guard must step
    // aside so the real fault keeps surfacing through the normal error path
    // instead of being hidden behind a silent empty list.
    let occupied = tmp.path().join("occupied");
    std::fs::write(&occupied, "not a directory").unwrap();
    let mut config = test_config(&tmp);
    config.config_path = occupied.join("config.toml");
    assert!(!backend_mode_without_session(&config));
}
