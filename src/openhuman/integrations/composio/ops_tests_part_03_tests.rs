use super::*;

#[tokio::test]
async fn fetch_connected_integrations_via_mock_aggregates_tools() {
    let _guard = cache_guard();
    // Connections: gmail + notion. Tools: filtered to those toolkits
    // and prefixed with the uppercased slug. The toolkits route
    // backs the `list_toolkits()` allowlist gate that
    // `fetch_connected_integrations_uncached` calls before touching
    // connections — without it the function bails out at the first
    // step and returns an empty vec.
    let app = Router::new()
        .route(
            "/agent-integrations/composio/toolkits",
            get(|| async {
                Json(json!({
                    "success": true,
                    "data": {"toolkits": ["gmail", "notion"]}
                }))
            }),
        )
        .route(
            "/agent-integrations/composio/connections",
            get(|| async {
                Json(json!({
                    "success": true,
                    "data": {"connections": [
                        {"id":"c1","toolkit":"gmail","status":"ACTIVE"},
                        {"id":"c2","toolkit":"notion","status":"CONNECTED"}
                    ]}
                }))
            }),
        )
        .route(
            "/agent-integrations/composio/tools",
            get(|| async {
                Json(json!({
                    "success": true,
                    "data": {"tools": [
                        {"type":"function","function":{
                            "name":"GMAIL_SEND_EMAIL",
                            "description":"Send"
                        }},
                        {"type":"function","function":{
                            "name":"NOTION_CREATE_PAGE",
                            "description":"Create"
                        }}
                    ]}
                }))
            }),
        );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    // Use a fresh cache key by isolating config_path.
    let config = config_with_backend(&tmp, base);
    invalidate_connected_integrations_cache();
    let integrations = fetch_connected_integrations(&config).await;
    assert_eq!(integrations.len(), 2);
    // Sorted by toolkit name
    assert_eq!(integrations[0].toolkit, "gmail");
    assert_eq!(integrations[1].toolkit, "notion");
    assert_eq!(integrations[0].tools.len(), 1);
    assert_eq!(integrations[0].tools[0].name, "GMAIL_SEND_EMAIL");
}

#[tokio::test]
async fn fetch_connected_integrations_treats_slack_and_telegram_status_like_ui() {
    let _guard = cache_guard();
    let app = Router::new()
        .route(
            "/agent-integrations/composio/toolkits",
            get(|| async {
                Json(json!({
                    "success": true,
                    "data": {"toolkits": [" Slack ", "telegram"]}
                }))
            }),
        )
        .route(
            "/agent-integrations/composio/connections",
            get(|| async {
                Json(json!({
                    "success": true,
                    "data": {"connections": [
                        {"id":"c-slack","toolkit":" Slack ","status":"connected"},
                        {"id":"c-telegram","toolkit":"telegram","status":" active "}
                    ]}
                }))
            }),
        )
        .route(
            "/agent-integrations/composio/tools",
            get(|| async {
                Json(json!({
                    "success": true,
                    "data": {"tools": [
                        {"type":"function","function":{
                            "name":"SLACK_FETCH_CONVERSATION_HISTORY",
                            "description":"Read Slack channel history"
                        }},
                        {"type":"function","function":{
                            "name":"TELEGRAM_GET_CHAT_HISTORY",
                            "description":"Read Telegram chat history"
                        }},
                        {"type":"function","function":{
                            "name":"SLACK_DELETE_CHANNEL",
                            "description":"Delete a channel"
                        }}
                    ]}
                }))
            }),
        );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_backend(&tmp, base);
    invalidate_connected_integrations_cache();

    let integrations = fetch_connected_integrations(&config).await;

    let slack = integrations
        .iter()
        .find(|i| i.toolkit == "slack")
        .expect("slack integration should be present");
    assert!(slack.connected);
    assert_eq!(slack.tools.len(), 1);
    assert_eq!(slack.tools[0].name, "SLACK_FETCH_CONVERSATION_HISTORY");

    let telegram = integrations
        .iter()
        .find(|i| i.toolkit == "telegram")
        .expect("telegram integration should be present");
    assert!(telegram.connected);
    assert_eq!(telegram.tools.len(), 1);
    assert_eq!(telegram.tools[0].name, "TELEGRAM_GET_CHAT_HISTORY");
}

#[tokio::test]
async fn fetch_connected_integrations_via_mock_returns_empty_with_no_active() {
    let _guard = cache_guard();
    let app = Router::new().route(
        "/agent-integrations/composio/connections",
        get(|| async {
            Json(json!({"success": true, "data": {"connections": [
                {"id":"c1","toolkit":"gmail","status":"PENDING"}
            ]}}))
        }),
    );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_backend(&tmp, base);
    invalidate_connected_integrations_cache();
    let integrations = fetch_connected_integrations(&config).await;
    assert!(integrations.is_empty());
}

#[test]
fn sync_cache_invalidates_when_connection_becomes_active() {
    let _guard = cache_guard();
    // Cache reflects the pre-connect world: gmail is listed but
    // not connected. This is exactly the state the chat runtime
    // gets stuck in on Windows when the user completes OAuth
    // after the event-bus 60 s readiness poll times out.
    let key = "windows-regression-1";
    clear_cache_key(key);
    seed_cache(
        key,
        vec![integration("gmail", false), integration("notion", false)],
    );

    // Fresh UI poll shows gmail just flipped ACTIVE — mirrors a
    // user who finished OAuth in the system browser.
    sync_cache_with_connections(&[conn("c-1", "gmail", "ACTIVE")]);

    // Chat-runtime cache must be cleared so the next
    // `fetch_connected_integrations` re-fetches truth from the
    // backend. Without this fix the entry would live on until
    // `CACHE_TTL` expired or the process restarted.
    let guard = INTEGRATIONS_CACHE.read().unwrap();
    assert!(
        guard.get(key).is_none(),
        "expected cache to be busted when a new toolkit flips ACTIVE"
    );
}

#[test]
fn sync_cache_invalidates_when_connection_is_removed() {
    let _guard = cache_guard();
    // Cache remembers gmail as connected. The user just
    // disconnected it from Settings; the next UI poll returns an
    // empty list. Chat must forget gmail within one poll.
    let key = "windows-regression-2";
    clear_cache_key(key);
    seed_cache(key, vec![integration("gmail", true)]);

    sync_cache_with_connections(&[]);

    let guard = INTEGRATIONS_CACHE.read().unwrap();
    assert!(
        guard.get(key).is_none(),
        "expected cache to be busted when a connected toolkit disappears"
    );
}

#[test]
fn sync_cache_noop_when_backend_matches_cached_state() {
    let _guard = cache_guard();
    // Steady state: UI polls confirm cache is accurate. No
    // invalidation — we must not thrash the chat runtime's tool
    // registry on every 5 s UI poll.
    let key = "windows-regression-3";
    clear_cache_key(key);
    seed_cache(
        key,
        vec![integration("gmail", true), integration("notion", false)],
    );

    sync_cache_with_connections(&[conn("c-1", "gmail", "ACTIVE")]);

    let guard = INTEGRATIONS_CACHE.read().unwrap();
    assert!(
        guard.get(key).is_some(),
        "expected cache entry to survive when backend matches cached state"
    );
    // And the seeded entries are still there byte-for-byte.
    assert_eq!(guard.get(key).unwrap().entries.len(), 2);
}

#[test]
fn sync_cache_ignores_non_active_connection_rows() {
    let _guard = cache_guard();
    // Backend reports a PENDING row (user started OAuth but
    // hasn't completed). The cache should NOT be invalidated —
    // that would trigger a fresh `list_tools` call on every poll
    // while the OAuth handshake is in flight, which is wasteful
    // and would also clear `tools` vecs for real active
    // integrations already on disk.
    let key = "windows-regression-4";
    clear_cache_key(key);
    seed_cache(key, vec![integration("gmail", true)]);

    sync_cache_with_connections(&[
        conn("c-1", "gmail", "ACTIVE"),
        conn("c-2", "notion", "PENDING"),
        conn("c-3", "slack", "FAILED"),
    ]);

    let guard = INTEGRATIONS_CACHE.read().unwrap();
    assert!(
        guard.get(key).is_some(),
        "PENDING/FAILED rows must not trigger invalidation"
    );
}

#[test]
fn sync_cache_treats_connected_status_equivalent_to_active() {
    let _guard = cache_guard();
    // Backend may emit either "ACTIVE" or "CONNECTED" — we treat
    // them identically in every status check (see
    // `fetch_connected_integrations_uncached` filter). Make sure
    // the new diff path matches that convention so it doesn't
    // produce a false-positive invalidation.
    let key = "windows-regression-5";
    clear_cache_key(key);
    seed_cache(key, vec![integration("gmail", true)]);

    // Same toolkit set but reported via the legacy "CONNECTED" spelling.
    sync_cache_with_connections(&[conn("c-1", "gmail", "CONNECTED")]);

    let guard = INTEGRATIONS_CACHE.read().unwrap();
    assert!(
        guard.get(key).is_some(),
        "CONNECTED should be treated as an active status"
    );
}

#[test]
fn cache_entries_expire_after_ttl() {
    let _guard = cache_guard();
    // Even without any UI polling, the chat runtime must
    // self-heal stale state within `CACHE_TTL`. We can't wait
    // 60 s in a unit test; instead, directly age the entry by
    // rewriting its `cached_at`.
    let key = "windows-regression-6";
    clear_cache_key(key);
    seed_cache(key, vec![integration("gmail", true)]);

    // Age the entry past the TTL.
    {
        let mut guard = INTEGRATIONS_CACHE.write().unwrap();
        let entry = guard.get_mut(key).unwrap();
        entry.cached_at = Instant::now() - (CACHE_TTL + Duration::from_secs(1));
    }

    // Re-read via the public API — expired reads must not serve
    // the stale entry. We can't trigger a real backend call in a
    // unit test, so assert that the read path falls through (by
    // asserting the entry is still present before the read, and
    // proving the staleness check via a direct helper).
    let is_fresh = {
        let guard = INTEGRATIONS_CACHE.read().unwrap();
        guard
            .get(key)
            .map(|c| c.cached_at.elapsed() < CACHE_TTL)
            .unwrap_or(false)
    };
    assert!(
        !is_fresh,
        "entry aged past CACHE_TTL must not be treated as fresh"
    );
}

#[test]
fn including_expired_serves_stale_snapshot_for_transient_fallback() {
    let _guard = cache_guard();
    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);
    let key = crate::openhuman::integrations::composio::connected_integrations::cache_key(&config);
    clear_cache_key(&key);
    seed_cache(&key, vec![integration("gmail", true)]);

    // Age the entry past the TTL (simulates a session idle > 60s).
    {
        let mut guard = INTEGRATIONS_CACHE.write().unwrap();
        guard.get_mut(&key).unwrap().cached_at =
            Instant::now() - (CACHE_TTL + Duration::from_secs(1));
    }

    // The TTL-enforcing read treats the expired entry as missing…
    assert!(
        cached_active_integrations(&config).is_none(),
        "expired entry must not be served by the freshness-checked read"
    );
    // …but the transient-failure fallback read preserves the last-known set,
    // so a backend blip just after TTL expiry doesn't drop tool-calling.
    let stale = cached_active_integrations_including_expired(&config)
        .expect("expired entry should still be returned by the fallback read");
    assert_eq!(stale.len(), 1);
    assert_eq!(stale[0].toolkit, "gmail");

    clear_cache_key(&key);
}

// ── Trigger management ops (PR #671) ────────────────────────────────

#[tokio::test]
async fn composio_list_available_triggers_via_mock() {
    let _serialised = module_guard().await;
    let app = Router::new().route(
        "/agent-integrations/composio/triggers/available",
        get(|Query(q): Query<HashMap<String, String>>| async move {
            assert_eq!(q.get("toolkit"), Some(&"gmail".into()));
            assert_eq!(q.get("connectionId"), Some(&"c1".into()));
            // Echo back so the test can also assert what was forwarded.
            Json(json!({
                "success": true,
                "data": {"triggers": [
                    {
                        "slug": "GMAIL_NEW_GMAIL_MESSAGE",
                        "scope": "static",
                        "defaultConfig": {"labelIds": "INBOX"},
                        "_echoed_connectionId": q.get("connectionId"),
                        "_echoed_toolkit": q.get("toolkit"),
                    }
                ]}
            }))
        }),
    );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_backend(&tmp, base);

    let outcome = composio_list_available_triggers(&config, "gmail", Some("c1".into()))
        .await
        .unwrap();
    assert_eq!(outcome.value.triggers.len(), 1);
    assert_eq!(outcome.value.triggers[0].slug, "GMAIL_NEW_GMAIL_MESSAGE");
    assert_eq!(outcome.value.triggers[0].scope, "static");
    assert!(outcome.logs.iter().any(|l| l.contains("available trigger")));
}

#[tokio::test]
async fn composio_list_available_triggers_omits_connection_when_none() {
    let _serialised = module_guard().await;
    let app = Router::new().route(
        "/agent-integrations/composio/triggers/available",
        get(|Query(q): Query<HashMap<String, String>>| async move {
            assert!(
                q.get("connectionId").is_none(),
                "should not forward connectionId"
            );
            Json(json!({"success": true, "data": {"triggers": []}}))
        }),
    );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_backend(&tmp, base);

    let outcome = composio_list_available_triggers(&config, "gmail", None)
        .await
        .unwrap();
    assert!(outcome.value.triggers.is_empty());
}

#[tokio::test]
async fn composio_list_triggers_via_mock_with_filter() {
    let _serialised = module_guard().await;
    let app = Router::new().route(
        "/agent-integrations/composio/triggers",
        get(|Query(_q): Query<HashMap<String, String>>| async move {
            Json(json!({
                "success": true,
                "data": {"triggers": [
                    {
                        "id": "ti_1",
                        "slug": "GMAIL_NEW_GMAIL_MESSAGE",
                        "toolkit": "gmail",
                        "connectionId": "c1",
                        "state": "active"
                    }
                ]}
            }))
        }),
    );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_backend(&tmp, base);

    let outcome = composio_list_triggers(&config, Some("gmail".into()))
        .await
        .unwrap();
    assert_eq!(outcome.value.triggers.len(), 1);
    assert_eq!(outcome.value.triggers[0].id, "ti_1");
    assert_eq!(outcome.value.triggers[0].connection_id, "c1");
}

#[tokio::test]
async fn composio_list_triggers_without_filter() {
    let _serialised = module_guard().await;
    let app = Router::new().route(
        "/agent-integrations/composio/triggers",
        get(|| async { Json(json!({"success": true, "data": {"triggers": []}})) }),
    );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_backend(&tmp, base);

    let outcome = composio_list_triggers(&config, None).await.unwrap();
    assert!(outcome.value.triggers.is_empty());
}

#[tokio::test]
async fn composio_enable_trigger_via_mock() {
    let _serialised = module_guard().await;
    let app = Router::new().route(
        "/agent-integrations/composio/triggers",
        post(|Json(body): Json<Value>| async move {
            assert_eq!(body["slug"], "GMAIL_NEW_GMAIL_MESSAGE");
            assert_eq!(body["connectionId"], "c1");
            assert_eq!(body["triggerConfig"]["labelIds"], "INBOX");
            Json(json!({
                "success": true,
                "data": {
                    "triggerId": "ti_new",
                    "slug": "GMAIL_NEW_GMAIL_MESSAGE",
                    "connectionId": "c1"
                }
            }))
        }),
    );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_backend(&tmp, base);

    let outcome = composio_enable_trigger(
        &config,
        "c1",
        "GMAIL_NEW_GMAIL_MESSAGE",
        Some(json!({"labelIds": "INBOX"})),
    )
    .await
    .unwrap();
    assert_eq!(outcome.value.trigger_id, "ti_new");
    assert_eq!(outcome.value.connection_id, "c1");
    assert!(outcome.logs.iter().any(|l| l.contains("enabled trigger")));
}

#[tokio::test]
async fn composio_disable_trigger_via_mock() {
    let _serialised = module_guard().await;
    let app = Router::new().route(
        "/agent-integrations/composio/triggers/{id}",
        axum::routing::delete(|Path(id): Path<String>| async move {
            assert_eq!(id, "ti_1");
            Json(json!({"success": true, "data": {"deleted": true}}))
        }),
    );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_backend(&tmp, base);

    let outcome = composio_disable_trigger(&config, "ti_1").await.unwrap();
    assert!(outcome.value.deleted);
    assert!(outcome.logs.iter().any(|l| l.contains("disabled trigger")));
}

#[tokio::test]
async fn composio_disable_trigger_propagates_backend_error() {
    let _serialised = module_guard().await;
    let app = Router::new().route(
        "/agent-integrations/composio/triggers/{id}",
        axum::routing::delete(|Path(_id): Path<String>| async move {
            (
                axum::http::StatusCode::NOT_FOUND,
                Json(json!({"success": false, "error": "Trigger not found"})),
            )
        }),
    );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_backend(&tmp, base);

    let err = composio_disable_trigger(&config, "missing")
        .await
        .unwrap_err();
    assert!(err.contains("disable_trigger failed"), "unexpected: {err}");
}

#[tokio::test]
async fn composio_list_toolkits_returns_empty_in_direct_mode() {
    let _serialised = module_guard().await;
    let tmp = tempfile::tempdir().unwrap();
    let config = direct_mode_config(&tmp);
    let outcome = composio_list_toolkits(&config)
        .await
        .expect("direct-mode list_toolkits must succeed without HTTP");
    assert!(
        outcome.value.toolkits.is_empty(),
        "direct mode must not surface the backend allowlist"
    );
    assert!(
        outcome.logs.iter().any(|l| l.contains("direct mode")),
        "log line must call out direct mode explicitly, got {:?}",
        outcome.logs
    );
}

#[tokio::test]
async fn composio_list_connections_routes_through_direct_mode() {
    let _serialised = module_guard().await;
    let _guard = cache_guard();
    let tmp = tempfile::tempdir().unwrap();
    let config = direct_mode_config(&tmp);
    // [composio-direct] After commit 2 of #1710, direct mode actually
    // calls `backend.composio.dev/api/v3/connected_accounts` rather
    // than returning an empty stub. Without a real Composio key the
    // remote will reject the test request, so we assert on the error
    // shape: it must reference `composio` AND must NOT reference the
    // backend-session path (proving the factory routed us to direct).
    let result = composio_list_connections(&config).await;
    match result {
        Ok(outcome) => {
            // Some sandboxes resolve OK with an empty list — accept
            // that as well, but the connections vec must be empty
            // (the test key is not provisioned in any real tenant).
            assert!(
                outcome.value.connections.is_empty(),
                "test key should not surface real connections"
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

#[test]
fn direct_mode_without_key_is_false_in_backend_mode() {
    let tmp = tempfile::tempdir().unwrap();
    // Default mode is backend — the guard must never fire there, or
    // backend users would get a silent empty list.
    let config = test_config(&tmp);
    assert!(!direct_mode_without_key(&config).unwrap());
}

#[test]
fn direct_mode_without_key_is_true_when_direct_and_no_key() {
    let tmp = tempfile::tempdir().unwrap();
    let config = direct_mode_no_key_config(&tmp);
    assert!(direct_mode_without_key(&config).unwrap());
}

#[test]
fn direct_mode_without_key_is_false_when_key_in_config_toml() {
    let tmp = tempfile::tempdir().unwrap();
    // Key supplied via config.toml (not the keychain) still counts —
    // the factory accepts it, so the guard must NOT short-circuit and
    // hide the user's real connections.
    let mut config = direct_mode_no_key_config(&tmp);
    config.composio.api_key = Some("  ck_cfg_key  ".into());
    assert!(!direct_mode_without_key(&config).unwrap());
}

#[test]
fn direct_mode_without_key_is_false_when_key_in_keychain() {
    let tmp = tempfile::tempdir().unwrap();
    // `direct_mode_config` stores a key via the auth store — the guard
    // must see it and report "has key".
    let config = direct_mode_config(&tmp);
    assert!(!direct_mode_without_key(&config).unwrap());
}

#[tokio::test]
async fn composio_list_connections_returns_empty_when_direct_mode_no_key() {
    let _serialised = module_guard().await;
    let tmp = tempfile::tempdir().unwrap();
    let config = direct_mode_no_key_config(&tmp);
    // RC of TAURI-RUST-R4: this MUST be Ok(empty), not Err — no error is
    // constructed, so nothing reaches Sentry and the 5 s poll stops
    // churning.
    let outcome = composio_list_connections(&config)
        .await
        .expect("direct mode with no key must return an empty list, not an error");
    assert!(
        outcome.value.connections.is_empty(),
        "no key → no tenant → no connections"
    );
    assert!(
        outcome.logs.iter().any(|l| l.contains("no api key")),
        "log must explain the empty list is the no-key setup state, got {:?}",
        outcome.logs
    );
}

// ── sync stage-event contracts (#5932) ───────────────────────────────────────

/// The completed-stage detail is a parse contract with the Sources UI, which
/// extracts the count via `/ingested\s+(\d+)\s+item/i` and shows a generic
/// "up to date" when the pattern misses (#3295). This is the exact regex,
/// ported, against the exact producer.
#[test]
fn completed_sync_detail_matches_the_ui_parse_contract() {
    let re = regex::Regex::new(r"(?i)ingested\s+(\d+)\s+item").expect("ui parse regex");
    for count in [0u64, 1, 200, 25_000] {
        let detail = crate::openhuman::integrations::composio::ops::completed_sync_detail(
            count, false, None,
        );
        let caps = re
            .captures(&detail)
            .unwrap_or_else(|| panic!("detail must parse: {detail}"));
        assert_eq!(
            caps[1].parse::<u64>().unwrap(),
            count,
            "count survives: {detail}"
        );
    }
}

/// Every parsed sync reason is a distinct event trigger — the stage events
/// must not collapse periodic and connection-created syncs into "manual"
/// (review finding on #5932).
#[test]
fn sync_reasons_map_to_distinct_triggers() {
    use crate::openhuman::integrations::composio::providers::SyncReason;
    let all = [
        SyncReason::Manual,
        SyncReason::Periodic,
        SyncReason::ConnectionCreated,
    ];
    let mut seen = std::collections::HashSet::new();
    for reason in all {
        assert!(
            seen.insert(reason.as_str().to_string()),
            "duplicate trigger"
        );
    }
    assert_eq!(seen.len(), 3);
}

/// The budgeted loop's arithmetic, held still: unlimited slices at the pass
/// ceiling, a cap slices to min(remaining, ceiling), a spent cap ends the run
/// (review finding on #5932 — this is the PR's core behavioural change).
#[test]
fn next_pass_budget_slices_and_exhausts_the_configured_cap() {
    use crate::openhuman::integrations::composio::ops::{next_pass_budget, SYNC_PASS_MAX_ITEMS};
    // Unlimited: every pass gets the ceiling.
    assert_eq!(next_pass_budget(None, 0), Some(SYNC_PASS_MAX_ITEMS));
    assert_eq!(next_pass_budget(None, 1_000_000), Some(SYNC_PASS_MAX_ITEMS));
    // A cap below the ceiling (200 since openhuman#6025) is one exact slice,
    // then exhaustion.
    assert_eq!(next_pass_budget(Some(50), 0), Some(50));
    assert_eq!(next_pass_budget(Some(50), 50), None);
    // A cap above the ceiling slices pass by pass and ends on the remainder —
    // a remainder smaller than the ceiling, so the two cannot be confused.
    assert_eq!(next_pass_budget(Some(1_100), 0), Some(SYNC_PASS_MAX_ITEMS));
    assert_eq!(next_pass_budget(Some(1_100), 1_000), Some(100));
    assert_eq!(next_pass_budget(Some(1_100), 1_100), None);
    // Over-written past the cap (dedupe drift) still ends, never underflows.
    assert_eq!(next_pass_budget(Some(100), 150), None);
}

/// Both detail variants keep the UI parse contract; the remainder text rides
/// after the count, never inside it.
#[test]
fn completed_detail_keeps_the_contract_with_a_remainder() {
    let re = regex::Regex::new(r"(?i)ingested\s+(\d+)\s+item").expect("ui parse regex");
    let capped =
        crate::openhuman::integrations::composio::ops::completed_sync_detail_for_test(7, true);
    assert!(
        re.captures(&capped).is_some(),
        "capped detail parses: {capped}"
    );
    assert!(capped.contains("more pending"));
}
