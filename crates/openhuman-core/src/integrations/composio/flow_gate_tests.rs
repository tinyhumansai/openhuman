use super::*;
use crate::agent::prompts::types::IntegrationConnection;
#[cfg(feature = "flows")]
use crate::integrations::composio::catalog::seed_live_catalog_cache_expired;
use crate::integrations::composio::catalog::{seed_live_catalog_cache, ToolContract};
use crate::integrations::composio::{ComposioExecuteResponse, ConnectedIntegration};
use crate::security::policy::CommandClass;
use serde_json::json;

fn integration(
    toolkit: &str,
    connected: bool,
    connections: Vec<IntegrationConnection>,
) -> ConnectedIntegration {
    ConnectedIntegration {
        toolkit: toolkit.to_string(),
        description: String::new(),
        tools: Vec::new(),
        gated_tools: Vec::new(),
        connected,
        connections,
        non_active_status: None,
    }
}

fn connection(id: &str, label: Option<&str>, is_default: bool) -> IntegrationConnection {
    IntegrationConnection {
        connection_id: id.to_string(),
        label: label.map(str::to_string),
        is_default,
    }
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
    // Reaches the connector module, whose route is process-wide: hold the module
    // lock (before TEST_ENV_LOCK, the established order) for the whole test.
    let _module = crate::integrations::composio::module_client::module_guard().await;
    use crate::integrations::composio::contract::catalog_for_toolkit;
    let config = Config::default();
    // Precondition: `flowstestkit` is genuinely uncatalogued, so the decision
    // flows through the connected-set path (not the static curated path).
    assert!(catalog_for_toolkit("flowstestkit").is_none());

    // No connected set at all → fail-closed reject.
    assert!(
        !flow_tool_allowed(
            &crate::core::runtime::DispatchCtx::ambient(),
            &config,
            "FLOWSTESTKIT_DO_THING",
            None
        )
        .await
    );
    // Connected set present but does not include this toolkit → reject.
    assert!(
        !flow_tool_allowed(
            &crate::core::runtime::DispatchCtx::ambient(),
            &config,
            "FLOWSTESTKIT_DO_THING",
            Some(&["gmail".to_string()])
        )
        .await
    );
    // A blank slug is always rejected.
    assert!(
        !flow_tool_allowed(
            &crate::core::runtime::DispatchCtx::ambient(),
            &config,
            "",
            Some(&["flowstestkit".to_string()])
        )
        .await
    );
}

/// A real Composio toolkit OpenHuman ships no static catalog for now PASSES
/// once the user has an ACTIVE connection for it (the TODO(0.3) fix) AND
/// the slug is a genuine action in its LIVE catalog (systemic tool-contract
/// fix) — seeded here so the test never touches a live Composio backend.
/// The exact same slug rejects above without a connection.
#[tokio::test]
async fn connected_uncatalogued_toolkit_now_passes() {
    // Reaches the connector module, whose route is process-wide: hold the module
    // lock (before TEST_ENV_LOCK, the established order) for the whole test.
    let _module = crate::integrations::composio::module_client::module_guard().await;
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
            &crate::core::runtime::DispatchCtx::ambient(),
            &config,
            "FLOWSTESTKIT_DO_THING",
            Some(&["flowstestkit".to_string()])
        )
        .await
    );
    // Case-insensitive match on the toolkit slug.
    assert!(
        flow_tool_allowed(
            &crate::core::runtime::DispatchCtx::ambient(),
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
#[cfg(feature = "flows")]
#[tokio::test]
async fn expired_live_catalog_entry_is_treated_as_a_cache_miss() {
    // Reaches the connector module, whose route is process-wide: hold the module
    // lock (before TEST_ENV_LOCK, the established order) for the whole test.
    let _module = crate::integrations::composio::module_client::module_guard().await;
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
            &crate::core::runtime::DispatchCtx::ambient(),
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
    // Reaches the connector module, whose route is process-wide: hold the module
    // lock (before TEST_ENV_LOCK, the established order) for the whole test.
    let _module = crate::integrations::composio::module_client::module_guard().await;
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
            &crate::core::runtime::DispatchCtx::ambient(),
            &config,
            "FLOWSTESTKIT_MADE_UP_ACTION",
            Some(&["flowstestkit".to_string()])
        )
        .await,
        "a hallucinated slug for a connected-but-uncurated toolkit must still reject"
    );
}

/// Unit coverage of the classifier itself, independent of the gate: a
/// curated Read entry classifies as `Read`; curated Write/Admin entries,
/// an uncurated toolkit, and an unparseable/empty slug all classify as
/// `Network` (fail-safe default — never silently widen to Read).
#[test]
fn classify_composio_action_for_tier_matches_curated_scope_fail_safe() {
    assert_eq!(
        classify_composio_action_for_tier("TWITTER_RECENT_SEARCH"),
        CommandClass::Read
    );
    assert_eq!(
        classify_composio_action_for_tier("TWITTER_CREATION_OF_A_POST"),
        CommandClass::Network
    );
    assert_eq!(
        classify_composio_action_for_tier("TWITTER_POST_DELETE_BY_POST_ID"),
        CommandClass::Network
    );
    // Uncurated toolkit (no catalog at all for "unknown").
    assert_eq!(
        classify_composio_action_for_tier("UNKNOWN_SERVICE_DO_THING"),
        CommandClass::Network
    );
    // Unparseable / empty slug.
    assert_eq!(classify_composio_action_for_tier(""), CommandClass::Network);
}

fn seeded_required_args_contract(slug: &str, toolkit: &str, required: &[&str]) -> ToolContract {
    ToolContract {
        slug: slug.to_string(),
        toolkit: toolkit.to_string(),
        description: None,
        required_args: required.iter().map(|name| name.to_string()).collect(),
        input_schema: Some(serde_json::json!({
            "type": "object",
            "required": required,
        })),
        output_fields: Vec::new(),
        output_schema: None,
        primary_array_path: None,
        is_curated: false,
    }
}

#[tokio::test]
async fn preflight_fails_before_dispatch_naming_the_missing_field() {
    let config = Config::default();
    seed_live_catalog_cache(
        "preflightfailtest",
        vec![seeded_required_args_contract(
            "PREFLIGHTFAILTEST_SEND_EMAIL",
            "preflightfailtest",
            &["to", "subject", "body"],
        )],
    );

    let message = preflight_composio_args(
        &config,
        "PREFLIGHTFAILTEST_SEND_EMAIL",
        &serde_json::json!({ "to": null, "subject": "hi", "body": "text" }),
    )
    .await
    .expect_err("null required arg must fail preflight");
    assert!(
        message.contains("`to`"),
        "error must name the field: {message}"
    );
    assert!(
        message.contains("=nodes.<node_id>.item.json.<field>"),
        "error must suggest the wiring fix: {message}"
    );
    assert!(
        message.contains("output schema"),
        "error must mention the agent output schema rule: {message}"
    );

    preflight_composio_args(
        &config,
        "PREFLIGHTFAILTEST_SEND_EMAIL",
        &serde_json::json!({ "to": "a@b.com", "subject": "hi", "body": "text" }),
    )
    .await
    .expect("wired args must pass preflight");
}

#[tokio::test]
async fn preflight_skips_when_no_schema_is_available() {
    preflight_composio_args(
        &Config::default(),
        "NOT_A_REAL_TOOLKIT_ACTION",
        &serde_json::json!({}),
    )
    .await
    .expect("preflight must be best-effort when no schema is available");
}

#[tokio::test]
async fn preflight_applies_static_arg_rules_without_a_catalog() {
    let config = Config::default();
    let message = preflight_composio_args(
        &config,
        "GMAIL_ADD_LABEL_TO_EMAIL",
        &serde_json::json!({ "add_label_ids": ["INBOX"] }),
    )
    .await
    .expect_err("a statically-known missing required arg must fail preflight");
    assert!(
        message.contains("message_id"),
        "error must name the missing field: {message}"
    );

    preflight_composio_args(
        &config,
        "GMAIL_ADD_LABEL_TO_EMAIL",
        &serde_json::json!({ "message_id": "abc123", "add_label_ids": ["INBOX"] }),
    )
    .await
    .expect("statically-valid args must pass preflight");
}
