use super::*;
use crate::config::Config;
use crate::flows::ops::FlowConnection;
use crate::integrations::composio::catalog::{seed_live_catalog_cache, ToolContract};
use serde_json::{json, Value};
use tempfile::TempDir;

fn test_config(tmp: &TempDir) -> Config {
    let config = Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    std::fs::create_dir_all(&config.workspace_dir).unwrap();
    config
}

fn graph(value: Value) -> Value {
    let graph = tinyflows::migrate::deserialize_graph(value).expect("test graph deserializes");
    tinyflows::validate::validate(&graph).expect("test graph is structurally valid");
    tinyflows::compat::ensure_compatible(&graph).expect("test graph is compatible");
    serde_json::to_value(graph).expect("validated graph serializes")
}

fn seeded_alias_contract() -> ToolContract {
    ToolContract {
        slug: "GOOGLEDRIVE_FIND_FILE".to_string(),
        toolkit: "googledrive".to_string(),
        description: None,
        required_args: vec![],
        input_schema: None,
        output_fields: vec!["files".to_string()],
        output_schema: Some(json!({
            "type": "object",
            "properties": { "files": { "type": "array" } }
        })),
        primary_array_path: Some("data.files".to_string()),
        is_curated: true,
    }
}

fn seeded_slack_send_contract() -> ToolContract {
    ToolContract {
        slug: "SLACK_SEND_MESSAGE".to_string(),
        toolkit: "slack".to_string(),
        description: None,
        required_args: vec!["channel".to_string(), "text".to_string()],
        input_schema: None,
        output_fields: vec!["ts".to_string(), "channel".to_string()],
        output_schema: Some(json!({
            "type": "object",
            "properties": { "ts": {"type": "string"}, "channel": {"type": "string"} }
        })),
        primary_array_path: None,
        is_curated: true,
    }
}

fn seeded_slack_send_message_contract_with_schema() -> ToolContract {
    ToolContract {
        slug: "SLACKARGNAMETEST_SEND_MESSAGE".to_string(),
        toolkit: "slackargnametest".to_string(),
        description: None,
        required_args: vec![],
        input_schema: Some(json!({
            "type": "object",
            "properties": {
                "channel": { "type": "string" },
                "markdown_text": { "type": "string" }
            }
        })),
        output_fields: vec![],
        output_schema: None,
        primary_array_path: None,
        is_curated: false,
    }
}

fn ws3_flow_conn(toolkit: &str, id: &str) -> FlowConnection {
    FlowConnection {
        connection_ref: format!("composio:{toolkit}:{id}"),
        kind: "composio".to_string(),
        display: toolkit.to_string(),
        toolkit: Some(toolkit.to_string()),
        scheme: None,
        platform_user_id: None,
    }
}

fn ws3_transcript_connections() -> Vec<FlowConnection> {
    vec![
        ws3_flow_conn("twitter", "ca_JX6QU88UfSk4"),
        ws3_flow_conn("gmail", "ca_vX_WA8FsqNmE"),
        ws3_flow_conn("tiktok", "ca_LPCp3WQpaDma"),
    ]
}

fn ws3_tool_call_graph(slug: &str, connection_ref: Option<&str>) -> Value {
    let mut config = json!({ "slug": slug, "args": {} });
    if let Some(connection_ref) = connection_ref {
        config["connection_ref"] = json!(connection_ref);
    }
    graph(json!({
        "nodes": [
            {"id":"t","kind":"trigger","name":"Manual"},
            {"id":"act","kind":"tool_call","name":"Act","config":config}
        ],
        "edges": [{"from_node":"t","to_node":"act"}]
    }))
}

fn upload_graph(path: Value) -> Value {
    graph(json!({
        "nodes": [
            {"id":"t","kind":"trigger","name":"Manual"},
            {"id":"up","kind":"tool_call","name":"Upload","config":{"slug":"oh:storage_upload_file","args":{"path":path}}}
        ],
        "edges": [{"from_node":"t","to_node":"up"}]
    }))
}

#[tokio::test]
async fn validate_tool_contracts_rejects_a_hallucinated_slug() {
    seed_live_catalog_cache("slack", vec![seeded_slack_send_contract()]);
    let config = Config::default();
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "SLACK_POST_MESSAGE_TO_CHANNEL",
                "args": { "channel": "#general", "markdown_text": "hi" } } }
        ],
        "edges": [ { "from_node": "t", "to_node": "post" } ]
    }));
    let errors = validate_tool_contracts(&config, &g).await;
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("post"), "{}", errors[0]);
    assert!(
        errors[0].contains("SLACK_POST_MESSAGE_TO_CHANNEL"),
        "{}",
        errors[0]
    );
    assert!(errors[0].contains("search_tool_catalog"), "{}", errors[0]);
}

#[tokio::test]
async fn validate_tool_contracts_rejects_a_missing_required_arg() {
    seed_live_catalog_cache("slack", vec![seeded_slack_send_contract()]);
    let config = Config::default();
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "SLACK_SEND_MESSAGE",
                "args": { "channel": "#general" } } }
        ],
        "edges": [ { "from_node": "t", "to_node": "post" } ]
    }));
    let errors = validate_tool_contracts(&config, &g).await;
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("`text`"), "{}", errors[0]);
    assert!(errors[0].contains("get_tool_contract"), "{}", errors[0]);
}

#[tokio::test]
async fn validate_tool_contracts_passes_a_fully_wired_real_slug() {
    seed_live_catalog_cache("slack", vec![seeded_slack_send_contract()]);
    let config = Config::default();
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "SLACK_SEND_MESSAGE",
                "args": { "channel": "#general", "text": "hi" } } }
        ],
        "edges": [ { "from_node": "t", "to_node": "post" } ]
    }));
    let errors = validate_tool_contracts(&config, &g).await;
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn connection_refs_reject_the_transcript_wrong_id_naming_the_right_ref() {
    // Twitter node carrying the TIKTOK connection id: toolkit segment matches
    // (twitter == twitter) but the id belongs to no Twitter account.
    let g = ws3_tool_call_graph(
        "TWITTER_CREATION_OF_A_POST",
        Some("composio:twitter:ca_LPCp3WQpaDma"),
    );
    let conns = ws3_transcript_connections();
    let errors = validate_connection_refs_against(&g, Some(&conns));
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("act"), "{}", errors[0]);
    assert!(
        errors[0].contains("composio:twitter:ca_JX6QU88UfSk4"),
        "must name the correct ref verbatim: {}",
        errors[0]
    );
    assert!(errors[0].contains("did you mean"), "{}", errors[0]);
}

#[test]
fn connection_refs_reject_a_toolkit_mismatch_naming_the_right_ref() {
    // A literal `composio:tiktok:...` ref stamped onto a Twitter node.
    let g = ws3_tool_call_graph(
        "TWITTER_CREATION_OF_A_POST",
        Some("composio:tiktok:ca_LPCp3WQpaDma"),
    );
    let conns = ws3_transcript_connections();
    let errors = validate_connection_refs_against(&g, Some(&conns));
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("tiktok"), "{}", errors[0]);
    assert!(
        errors[0].contains("composio:twitter:ca_JX6QU88UfSk4"),
        "{}",
        errors[0]
    );
}

#[test]
fn connection_refs_reject_an_unknown_id_when_the_toolkit_has_no_connection() {
    // Gmail slug, but no gmail account connected at all → point at composio_connect.
    let g = ws3_tool_call_graph("GMAIL_SEND_EMAIL", Some("composio:gmail:ca_missing"));
    let conns = vec![ws3_flow_conn("twitter", "ca_JX6QU88UfSk4")];
    let errors = validate_connection_refs_against(&g, Some(&conns));
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("composio_connect"), "{}", errors[0]);
    assert!(!errors[0].contains("did you mean"), "{}", errors[0]);
}

#[test]
fn connection_refs_pass_the_correct_ref() {
    let g = ws3_tool_call_graph(
        "TWITTER_CREATION_OF_A_POST",
        Some("composio:twitter:ca_JX6QU88UfSk4"),
    );
    let conns = ws3_transcript_connections();
    let errors = validate_connection_refs_against(&g, Some(&conns));
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn connection_refs_reject_a_malformed_ref() {
    let g = ws3_tool_call_graph("GMAIL_SEND_EMAIL", Some("gmail-ca_vX_WA8FsqNmE"));
    let conns = ws3_transcript_connections();
    let errors = validate_connection_refs_against(&g, Some(&conns));
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("malformed"), "{}", errors[0]);
}

#[test]
fn connection_refs_skip_oh_and_refless_and_expression_nodes() {
    // Native oh: tool with a ref → skipped.
    let g_oh = ws3_tool_call_graph("oh:memory_search", Some("composio:twitter:whatever"));
    assert!(
        validate_connection_refs_against(&g_oh, Some(&ws3_transcript_connections())).is_empty()
    );
    // Composio tool_call with NO connection_ref stays allowed (prompts at run).
    let g_refless = ws3_tool_call_graph("TWITTER_CREATION_OF_A_POST", None);
    assert!(
        validate_connection_refs_against(&g_refless, Some(&ws3_transcript_connections()))
            .is_empty()
    );
    // `=`-derived slug → skipped.
    let g_expr = ws3_tool_call_graph("=item.slug", Some("composio:twitter:ca_LPCp3WQpaDma"));
    assert!(
        validate_connection_refs_against(&g_expr, Some(&ws3_transcript_connections())).is_empty()
    );
}

#[test]
fn connection_refs_fail_open_on_unavailable_connections_but_keep_mismatch() {
    // Connections unavailable (None): the id-existence check is SKIPPED — a
    // toolkit-matched ref with an unknown id passes rather than false-reject.
    let g_ok = ws3_tool_call_graph(
        "TWITTER_CREATION_OF_A_POST",
        Some("composio:twitter:ca_anything"),
    );
    assert!(
        validate_connection_refs_against(&g_ok, None).is_empty(),
        "unknown id must be skipped when connections are unavailable"
    );
    // ...but the toolkit-mismatch check needs no I/O and still fires.
    let g_mismatch = ws3_tool_call_graph(
        "TWITTER_CREATION_OF_A_POST",
        Some("composio:tiktok:ca_LPCp3WQpaDma"),
    );
    let errors = validate_connection_refs_against(&g_mismatch, None);
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("tiktok"), "{}", errors[0]);
}

#[test]
fn validate_upload_paths_rejects_an_absolute_path() {
    // The live-observed bug: the model copies `/tmp/openhuman-flow/report.html`
    // from a prior flow, which the runtime rejects (uploads are confined to the
    // workspace). Catch it at author time with an actionable message.
    let errors = validate_upload_paths(&upload_graph(json!("/tmp/openhuman-flow/report.html")));
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("'up'"), "{}", errors[0]);
    assert!(errors[0].contains("workspace-relative"), "{}", errors[0]);
}

#[test]
fn validate_upload_paths_accepts_a_workspace_relative_path() {
    assert!(validate_upload_paths(&upload_graph(json!("report.html"))).is_empty());
    assert!(validate_upload_paths(&upload_graph(json!("out/report.html"))).is_empty());
}

#[test]
fn validate_upload_paths_rejects_a_parent_escape() {
    let errors = validate_upload_paths(&upload_graph(json!("../../etc/passwd")));
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("escaping with `..`"), "{}", errors[0]);
}

#[test]
fn validate_upload_paths_ignores_a_dynamic_path_expression() {
    // A `=`-expression resolves at runtime; the author-gate can't know its value,
    // so it must not reject it (the runtime check still applies).
    assert!(validate_upload_paths(&upload_graph(json!("=nodes.prep.item.json.path"))).is_empty());
}

#[tokio::test]
async fn validate_tool_contracts_accepts_a_separated_action_slug_alias() {
    seed_live_catalog_cache("googledrive", vec![seeded_alias_contract()]);
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "find", "kind": "tool_call", "name": "Find",
              "config": { "slug": "GOOGLE_DRIVE_FIND_FILE", "args": {} } }
        ],
        "edges": [ { "from_node": "t", "to_node": "find" } ]
    }));

    let errors = validate_tool_contracts(&Config::default(), &g).await;

    assert!(
        errors.is_empty(),
        "valid action alias was rejected: {errors:?}"
    );
}

#[tokio::test]
async fn validate_tool_contracts_rejects_a_real_but_uncurated_action_on_a_statically_catalogued_toolkit(
) {
    seed_live_catalog_cache(
        "notion",
        vec![ToolContract {
            slug: "NOTION_UNCURATED_ACTION".to_string(),
            toolkit: "notion".to_string(),
            description: None,
            required_args: vec![],
            input_schema: None,
            output_fields: vec![],
            output_schema: None,
            primary_array_path: None,
            // Real (a live catalog fetch found it), but NOT one of
            // OpenHuman's curated Notion actions.
            is_curated: false,
        }],
    );
    let config = Config::default();
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "NOTION_UNCURATED_ACTION", "args": {} } }
        ],
        "edges": [ { "from_node": "t", "to_node": "post" } ]
    }));
    let errors = validate_tool_contracts(&config, &g).await;
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(
        errors[0].contains("NOTION_UNCURATED_ACTION"),
        "{}",
        errors[0]
    );
    assert!(errors[0].contains("curated"), "{}", errors[0]);
}

#[tokio::test]
async fn validate_tool_contracts_skips_expression_derived_and_native_slugs() {
    let config = Config::default();
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "dynamic", "kind": "tool_call", "name": "Dynamic",
              "config": { "slug": "=item.tool", "args": {} } },
            { "id": "native", "kind": "tool_call", "name": "Native",
              "config": { "slug": "oh:web_search", "args": {} } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "dynamic" },
            { "from_node": "t", "to_node": "native" }
        ]
    }));
    let errors = validate_tool_contracts(&config, &g).await;
    assert!(errors.is_empty(), "{errors:?}");
}

#[tokio::test]
async fn validate_tool_contracts_skips_rather_than_rejects_when_the_catalog_is_unreachable() {
    // No seed for this toolkit and no live backend configured — the fetch
    // fails, and the node must be SKIPPED (never false-rejected).
    let config = Config::default();
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "SOMEUNSEEDEDTOOLKIT_DO_THING", "args": {} } }
        ],
        "edges": [ { "from_node": "t", "to_node": "post" } ]
    }));
    let errors = validate_tool_contracts(&config, &g).await;
    assert!(
        errors.is_empty(),
        "a live-catalog fetch failure must skip, not reject: {errors:?}"
    );
}

#[tokio::test]
async fn validate_tool_contracts_rejects_an_arg_name_not_in_the_input_schema() {
    seed_live_catalog_cache(
        "slackargnametest",
        vec![seeded_slack_send_message_contract_with_schema()],
    );
    let config = Config::default();
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "SLACKARGNAMETEST_SEND_MESSAGE",
                "args": { "channel": "#general", "text": "hi" } } }
        ],
        "edges": [ { "from_node": "t", "to_node": "post" } ]
    }));
    let errors = validate_tool_contracts(&config, &g).await;
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("post"), "{}", errors[0]);
    assert!(errors[0].contains("`text`"), "{}", errors[0]);
    assert!(errors[0].contains("markdown_text"), "{}", errors[0]);
    assert!(errors[0].contains("get_tool_contract"), "{}", errors[0]);
}

#[tokio::test]
async fn validate_tool_contracts_passes_the_real_arg_name_from_the_input_schema() {
    seed_live_catalog_cache(
        "slackargnametest",
        vec![seeded_slack_send_message_contract_with_schema()],
    );
    let config = Config::default();
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "SLACKARGNAMETEST_SEND_MESSAGE",
                "args": { "channel": "#general", "markdown_text": "hi" } } }
        ],
        "edges": [ { "from_node": "t", "to_node": "post" } ]
    }));
    let errors = validate_tool_contracts(&config, &g).await;
    assert!(errors.is_empty(), "{errors:?}");
}

/// Uses its own cache key/toolkit (never `"slack"`/`"gmail"`) since the
/// arg-name check must behave identically no matter which slug it's
/// exercised against, and a dedicated, unregistered toolkit sidesteps both
/// the process-global `LIVE_CATALOG_CACHE` sharing risk the other
/// `validate_tool_contracts` tests accept AND the static curated-catalog
/// gate (this toolkit has none, so `is_curated` is irrelevant here).

#[tokio::test]
async fn validate_tool_contracts_skips_arg_name_check_when_input_schema_is_unknown() {
    seed_live_catalog_cache(
        "argschemaunknown",
        vec![ToolContract {
            slug: "ARGSCHEMAUNKNOWN_DO_THING".to_string(),
            toolkit: "argschemaunknown".to_string(),
            description: None,
            required_args: vec![],
            input_schema: None,
            output_fields: vec![],
            output_schema: None,
            primary_array_path: None,
            is_curated: false,
        }],
    );
    let config = Config::default();
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "ARGSCHEMAUNKNOWN_DO_THING",
                "args": { "totally_made_up_field": "hi" } } }
        ],
        "edges": [ { "from_node": "t", "to_node": "post" } ]
    }));
    let errors = validate_tool_contracts(&config, &g).await;
    assert!(
        errors.is_empty(),
        "an unknown input_schema must skip the arg-name check, never reject: {errors:?}"
    );
}

#[tokio::test]
async fn validate_tool_contracts_allows_arbitrary_arg_names_when_schema_permits_additional_properties(
) {
    seed_live_catalog_cache(
        "argschemaadditional",
        vec![ToolContract {
            slug: "ARGSCHEMAADDITIONAL_DO_THING".to_string(),
            toolkit: "argschemaadditional".to_string(),
            description: None,
            required_args: vec![],
            input_schema: Some(json!({
                "type": "object",
                "properties": { "channel": { "type": "string" } },
                "additionalProperties": true
            })),
            output_fields: vec![],
            output_schema: None,
            primary_array_path: None,
            is_curated: false,
        }],
    );
    let config = Config::default();
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "ARGSCHEMAADDITIONAL_DO_THING",
                "args": { "channel": "#general", "any_extra_field": "hi" } } }
        ],
        "edges": [ { "from_node": "t", "to_node": "post" } ]
    }));
    let errors = validate_tool_contracts(&config, &g).await;
    assert!(
        errors.is_empty(),
        "additionalProperties: true must allow arbitrary arg names: {errors:?}"
    );
}
