use super::*;

// ── native `oh:` tool result handling ──────────────────────────────────

#[test]
fn native_tool_payload_unwraps_a_single_json_block() {
    // `storage_get_link` returns exactly one Json block. A downstream node
    // must be able to bind `=nodes.<id>.item.json.url` — the same shape
    // used everywhere else — not `...item.json.content[0].data.url`.
    let result = ToolResult::json(json!({
        "url": "https://example.test/presigned",
        "expires_at": "2026-01-01T00:00:00Z",
    }));
    let payload = native_tool_payload(&result);
    assert_eq!(payload["url"], "https://example.test/presigned");
    assert_eq!(payload["expires_at"], "2026-01-01T00:00:00Z");
    assert!(
        payload.get("content").is_none() && payload.get("is_error").is_none(),
        "the ToolResult envelope must not leak into item.json: {payload}"
    );
}

#[test]
fn native_tool_payload_collapses_text_to_a_bindable_field() {
    let payload = native_tool_payload(&ToolResult::success("done"));
    assert_eq!(payload["text"], "done");
}

#[test]
fn native_tool_payload_collapses_mixed_blocks_to_text() {
    let result = ToolResult {
        content: vec![
            ToolContent::Text {
                text: "line".into(),
            },
            ToolContent::Json {
                data: json!({"k": 1}),
            },
        ],
        is_error: false,
        markdown_formatted: None,
        ..ToolResult::default()
    };
    let payload = native_tool_payload(&result);
    let text = payload["text"].as_str().expect("text field");
    assert!(text.contains("line") && text.contains('k'), "got {text}");
}

#[test]
fn native_tool_failure_fails_the_step_instead_of_recording_success() {
    // The bug this guards: `execute_tool` returns Ok for a tool that ran
    // and FAILED (is_error), so the engine recorded the step — and the run
    // — as Success while a downstream node bound a null value.
    let result = ToolResult::error("storage quota exceeded");
    let err = reject_failed_native_tool_result("oh:storage_upload_file", &result)
        .expect_err("an is_error ToolResult must fail the step");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("storage_upload_file") && msg.contains("storage quota exceeded"),
        "error must name the tool and the provider detail: {msg}"
    );
}

#[test]
fn native_tool_success_passes_through() {
    let result = ToolResult::json(json!({"file_id": "f_1"}));
    assert!(reject_failed_native_tool_result("oh:storage_upload_file", &result).is_ok());
}

// ── reject_unsuccessful_composio_response (B6) ──────────────────────────

#[test]
fn reject_unsuccessful_composio_response_errors_on_provider_failure() {
    // Live-observed shape: SLACK_SEND_MESSAGE 400s upstream but the
    // Composio execute call itself still returns HTTP 200.
    let resp = ComposioExecuteResponse {
        data: json!({}),
        successful: false,
        error: Some("Invalid request data".to_string()),
        cost_usd: 0.0,
        markdown_formatted: None,
    };
    let err = reject_unsuccessful_composio_response("SLACK_SEND_MESSAGE", resp)
        .expect_err("unsuccessful response must become an Err");
    let msg = err.to_string();
    assert!(msg.contains("SLACK_SEND_MESSAGE"), "message was: {msg}");
    assert!(msg.contains("Invalid request data"), "message was: {msg}");
}

#[test]
fn reject_unsuccessful_composio_response_falls_back_when_error_field_is_empty() {
    let resp = ComposioExecuteResponse {
        data: json!({}),
        successful: false,
        error: None,
        cost_usd: 0.0,
        markdown_formatted: None,
    };
    let err = reject_unsuccessful_composio_response("GMAIL_SEND_EMAIL", resp)
        .expect_err("unsuccessful response must become an Err");
    let msg = err.to_string();
    assert!(msg.contains("GMAIL_SEND_EMAIL"), "message was: {msg}");
    assert!(
        msg.contains("no error detail returned by the provider"),
        "message was: {msg}"
    );
}

#[test]
fn reject_unsuccessful_composio_response_passes_through_on_success() {
    let resp = ComposioExecuteResponse {
        data: json!({ "ts": "123.456" }),
        successful: true,
        error: None,
        cost_usd: 0.002,
        markdown_formatted: None,
    };
    let ok = reject_unsuccessful_composio_response("SLACK_SEND_MESSAGE", resp.clone())
        .expect("successful response must remain Ok");
    assert!(ok.successful);
    assert_eq!(ok.data, resp.data);
}

// ── input_context (PR A) ────────────────────────────────────────────────

/// A `composio:<toolkit>:<connection_id>` ref parses to its id and that id
/// resolves to the SPECIFIC connected account (toolkit + display label) —
/// not the toolkit's default connection.
#[test]
fn connection_ref_resolves_to_the_chosen_account() {
    let integrations = vec![integration(
        "gmail",
        true,
        vec![
            connection("conn_work", Some("work@example.com"), true),
            connection("conn_home", Some("home@example.com"), false),
        ],
    )];

    let id = composio_connection_id("composio:gmail:conn_home")
        .expect("well-formed composio connection_ref should parse");
    assert_eq!(id, "conn_home");

    let (toolkit, label) =
        resolve_account(&integrations, id).expect("id should resolve to a connected account");
    assert_eq!(toolkit, "gmail");
    // The non-default account was chosen — resolution is by id, not default.
    assert_eq!(label, Some("home@example.com"));

    // An id the user does not hold resolves to nothing (best-effort log path).
    assert!(resolve_account(&integrations, "conn_unknown").is_none());
}

/// A made-up toolkit that OpenHuman ships no static catalog for and the user
/// has NOT connected still rejects — even when the connected set is present
/// but simply doesn't contain it.
#[tokio::test]
async fn unknown_toolkit_still_rejects() {
    use crate::integrations::composio::contract::catalog_for_toolkit;
    let config = Config::default();
    // Precondition: `flowstestkit` is genuinely uncatalogued, so the decision
    // flows through the connected-set path (not the static curated path).
    assert!(catalog_for_toolkit("flowstestkit").is_none());

    // No connected set at all → fail-closed reject.
    assert!(!flow_tool_allowed(&config, "FLOWSTESTKIT_DO_THING", None).await);
    // Connected set present but does not include this toolkit → reject.
    assert!(
        !flow_tool_allowed(
            &config,
            "FLOWSTESTKIT_DO_THING",
            Some(&["gmail".to_string()])
        )
        .await
    );
    // A blank slug is always rejected.
    assert!(!flow_tool_allowed(&config, "", Some(&["flowstestkit".to_string()])).await);
}

/// A real Composio toolkit OpenHuman ships no static catalog for now PASSES
/// once the user has an ACTIVE connection for it (the TODO(0.3) fix) AND
/// the slug is a genuine action in its LIVE catalog (systemic tool-contract
/// fix) — seeded here so the test never touches a live Composio backend.
/// The exact same slug rejects above without a connection.
#[tokio::test]
async fn connected_uncatalogued_toolkit_now_passes() {
    use crate::integrations::composio::contract::catalog_for_toolkit;
    assert!(catalog_for_toolkit("flowstestkit").is_none());

    let config = Config::default();
    seed_live_catalog_cache(
        "flowstestkit",
        vec![ToolContract {
            slug: "FLOWSTESTKIT_DO_THING".to_string(),
            toolkit: "flowstestkit".to_string(),
            description: None,
            required_args: Vec::new(),
            input_schema: None,
            output_fields: Vec::new(),
            output_schema: None,
            primary_array_path: None,
            is_curated: false,
        }],
    );

    assert!(
        flow_tool_allowed(
            &config,
            "FLOWSTESTKIT_DO_THING",
            Some(&["flowstestkit".to_string()])
        )
        .await
    );
    // Case-insensitive match on the toolkit slug.
    assert!(
        flow_tool_allowed(
            &config,
            "FLOWSTESTKIT_DO_THING",
            Some(&["FlowsTestKit".to_string()])
        )
        .await
    );
}

/// E-m8: an EXPIRED `LIVE_CATALOG_CACHE` entry must be treated as a cache
/// miss, not a permanent hit. Before the TTL fix, seeding the cache once
/// (as `connected_uncatalogued_toolkit_now_passes` does above) made a
/// slug pass forever, for the life of the process — a Composio action
/// added after the first fetch would stay invisible until restart. Here
/// the seeded entry is pre-expired, so `fetch_live_toolkit_catalog` must
/// re-fetch — which fails in this test (no live Composio backend) — and
/// `flow_tool_allowed` must fail CLOSED, unlike the fresh-seed case above
/// which passes.
#[tokio::test]
async fn expired_live_catalog_entry_is_treated_as_a_cache_miss() {
    use crate::integrations::composio::contract::catalog_for_toolkit;
    assert!(catalog_for_toolkit("flowsexpiredkit").is_none());

    let config = Config::default();
    seed_live_catalog_cache_expired(
        "flowsexpiredkit",
        vec![ToolContract {
            slug: "FLOWSEXPIREDKIT_DO_THING".to_string(),
            toolkit: "flowsexpiredkit".to_string(),
            description: None,
            required_args: Vec::new(),
            input_schema: None,
            output_fields: Vec::new(),
            output_schema: None,
            primary_array_path: None,
            is_curated: false,
        }],
    );

    assert!(
        !flow_tool_allowed(
            &config,
            "FLOWSEXPIREDKIT_DO_THING",
            Some(&["flowsexpiredkit".to_string()])
        )
        .await,
        "an expired cache entry must be re-fetched (and, with no live backend in this test, \
         fail closed) rather than served as a permanent hit"
    );
}

/// A CONNECTED but uncatalogued toolkit still rejects a slug that shares
/// its prefix but isn't a genuine action in the LIVE catalog — the
/// systemic tool-contract fix's tightening: connection alone is no longer
/// sufficient, the slug itself must be real.
#[tokio::test]
async fn connected_uncatalogued_toolkit_rejects_a_hallucinated_slug() {
    use crate::integrations::composio::contract::catalog_for_toolkit;
    assert!(catalog_for_toolkit("flowstestkit").is_none());

    let config = Config::default();
    seed_live_catalog_cache(
        "flowstestkit",
        vec![ToolContract {
            slug: "FLOWSTESTKIT_DO_THING".to_string(),
            toolkit: "flowstestkit".to_string(),
            description: None,
            required_args: Vec::new(),
            input_schema: None,
            output_fields: Vec::new(),
            output_schema: None,
            primary_array_path: None,
            is_curated: false,
        }],
    );

    assert!(
        !flow_tool_allowed(
            &config,
            "FLOWSTESTKIT_MADE_UP_ACTION",
            Some(&["flowstestkit".to_string()])
        )
        .await,
        "a hallucinated slug for a connected-but-uncurated toolkit must still reject"
    );
}

/// A `http_cred:<name>` ref resolves to the stored bearer credential and
/// injects `Authorization: Bearer <token>` onto the outbound request.
#[test]
fn http_cred_resolves_and_injects_bearer_header() {
    let (_dir, store) = http_cred_store();
    store
        .upsert(&HttpCredential::bearer("stripe", "sk_live_secret"))
        .unwrap();

    let cred = resolve_http_credential(&store, Some("http_cred:stripe"))
        .expect("resolve ok")
        .expect("credential present");

    let mut request = json!({ "method": "GET", "url": "https://api.example.com" });
    let header = inject_http_credential(&mut request, &cred).unwrap();
    assert_eq!(header, "Authorization");
    assert_eq!(
        request["headers"]["Authorization"],
        json!("Bearer sk_live_secret")
    );
}

/// A custom-header credential injects under its own header name while
/// preserving any headers the flow author already set.
#[test]
fn http_cred_injection_preserves_existing_headers() {
    let (_dir, store) = http_cred_store();
    store
        .upsert(&HttpCredential::header("apikey", "X-API-Key", "topsecret"))
        .unwrap();
    let cred = resolve_http_credential(&store, Some("http_cred:apikey"))
        .unwrap()
        .unwrap();

    let mut request = json!({
        "method": "POST",
        "url": "https://api.example.com",
        "headers": { "Content-Type": "application/json" }
    });
    inject_http_credential(&mut request, &cred).unwrap();
    assert_eq!(
        request["headers"]["Content-Type"],
        json!("application/json")
    );
    assert_eq!(request["headers"]["X-API-Key"], json!("topsecret"));
}

/// A basic credential injects `Authorization: Basic ...` even when the flow
/// author set no `headers` object at all.
#[test]
fn http_cred_injects_basic_into_absent_headers() {
    let (_dir, store) = http_cred_store();
    store
        .upsert(&HttpCredential::basic("acme", "alice", "pw"))
        .unwrap();
    let cred = resolve_http_credential(&store, Some("http_cred:acme"))
        .unwrap()
        .unwrap();

    let mut request = json!({ "method": "GET", "url": "https://x.example.com" });
    inject_http_credential(&mut request, &cred).unwrap();
    let value = request["headers"]["Authorization"]
        .as_str()
        .expect("Authorization header injected");
    assert!(
        value.starts_with("Basic "),
        "unexpected basic header: {value}"
    );
}

/// A `http_cred:<name>` naming a credential that does not exist FAILS the
/// request closed — it must never proceed silently unauthenticated.
#[test]
fn unknown_http_cred_fails_closed() {
    let (_dir, store) = http_cred_store();
    let result = resolve_http_credential(&store, Some("http_cred:ghost"));
    assert!(result.is_err(), "unknown http_cred must fail closed");
}

/// A malformed `http_cred:` ref (empty or whitespace-only name) must fail
/// closed the same as an unknown credential name — it must never be
/// treated as "no connection_ref" and silently sent unauthenticated
/// (Codex P2 finding).
#[test]
fn malformed_http_cred_name_fails_closed() {
    let (_dir, store) = http_cred_store();
    assert!(
        resolve_http_credential(&store, Some("http_cred:")).is_err(),
        "an empty http_cred name must fail closed, not fall through as no-op"
    );
    assert!(
        resolve_http_credential(&store, Some("http_cred:   ")).is_err(),
        "a whitespace-only http_cred name must fail closed, not fall through as no-op"
    );
}

/// No `connection_ref`, or a non-`http_cred:` prefix, injects nothing and
/// is not an error.
#[test]
fn no_http_cred_ref_injects_nothing() {
    let (_dir, store) = http_cred_store();
    assert!(resolve_http_credential(&store, None).unwrap().is_none());
    assert!(
        resolve_http_credential(&store, Some("composio:gmail:conn_1"))
            .unwrap()
            .is_none()
    );
}

/// The secret is server-side-only: the approval-gate redaction (computed on
/// the pre-injection request) never contains it, and after injection it
/// lives ONLY in the outbound `Authorization` header.
#[test]
fn injected_secret_never_reaches_the_audit_redaction() {
    let (_dir, store) = http_cred_store();
    let secret = "sk_live_never_log_me";
    store
        .upsert(&HttpCredential::bearer("stripe", secret))
        .unwrap();
    let cred = resolve_http_credential(&store, Some("http_cred:stripe"))
        .unwrap()
        .unwrap();

    let mut request = json!({ "method": "GET", "url": "https://api.example.com" });
    // Pre-injection redaction — what the approval UI / audit trail sees.
    let redacted = crate::security::approval::redact_args(&request);
    assert!(!serde_json::to_string(&redacted).unwrap().contains(secret));

    inject_http_credential(&mut request, &cred).unwrap();
    assert_eq!(
        request["headers"]["Authorization"],
        json!(format!("Bearer {secret}"))
    );
}
