use super::*;

/// The negative-probe cache (design correction, item 3): a definitive
/// `provider_not_configured` result must be served from cache within the TTL
/// exactly like a `"ready"` result, so an edit -> validate -> propose -> run
/// authoring/run burst hits the mock backend once, not once per call (the judge's
/// live run observed 4 network round trips in a single ~80s turn before this
/// fix). Uses a real local axum server (no real network) that counts requests
/// so a cache hit is provable, not just plausible.
#[tokio::test]
async fn cached_probe_inference_readiness_caches_a_negative_result() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let tmp = TempDir::new().unwrap();
    let mut config = test_config(&tmp);
    seed_app_session_for_gate_test(&tmp);

    let hit_count = std::sync::Arc::new(AtomicUsize::new(0));
    let counter = hit_count.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    let app = axum::Router::new().route(
        "/openai/v1/chat/completions",
        axum::routing::post(move || {
            let counter = counter.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                use axum::response::IntoResponse;
                (
                    axum::http::StatusCode::BAD_REQUEST,
                    axum::Json(json!({
                        "success": false,
                        "error": "API key not configured for provider",
                        "errorCode": "BAD_REQUEST"
                    })),
                )
                    .into_response()
            }
        }),
    );
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    config.api_url = Some(format!("http://{addr}"));

    // First call: a real (mock) network round trip, definitively rejected.
    let first = cached_probe_inference_readiness("summarization", &config).await;
    let err = first.expect_err("a confirmed provider-not-configured 400 must reject");
    assert!(
        err.to_ascii_lowercase()
            .contains("api key not configured for provider"),
        "error must surface the backend's own message: {err}"
    );
    assert_eq!(
        hit_count.load(Ordering::SeqCst),
        1,
        "the first call must hit the (mock) network exactly once"
    );

    // Second call, same (role, config_path) key, well within the TTL: must be
    // served from cache — the mock server's hit count must NOT increase.
    let second = cached_probe_inference_readiness("summarization", &config).await;
    assert!(
        second.is_err(),
        "the cached negative result must still be an Err"
    );
    assert_eq!(
        hit_count.load(Ordering::SeqCst),
        1,
        "a repeat probe within the TTL must be served from cache, not hit the network again"
    );
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

// ── validate_required_arg_resolvability (issue B18) ─────────────────────────
//
// `validate_tool_contracts`'s `missing_required_args` only proves an arg is
// PRESENT (absent/literal-null) — it says nothing about whether an arg wired
// to a real-looking `=`-expression actually RESOLVES to a value at runtime,
// nor about an arg the schema doesn't individually mark `required` even
// though the provider enforces it as a business rule (the real B18 bug:
// `GMAIL_SEND_EMAIL.subject`/`.body` are each optional in the schema, but
// Gmail rejects a send where both are empty). These tests sandbox-run the
// graph the same way `dry_run_workflow` does and prove ANY tool_call arg
// that resolves `null` (because it's bound to a field that doesn't exist
// upstream) is a hard reject, while a fully-resolved graph passes clean. No
// live-catalog seeding needed — this check doesn't consult the Composio
// schema at all, only the sandbox's own traced diagnostics.

#[tokio::test]
async fn validate_required_arg_resolvability_rejects_a_null_resolved_arg() {
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "prep", "kind": "code", "name": "Prep",
              "config": { "language": "javascript", "source": "return {};" } },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "GMAIL_SEND_EMAIL",
                "args": { "recipient_email": "a@b.com", "subject": "=item.nonexistent_field" } } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "prep" },
            { "from_node": "prep", "to_node": "post" }
        ]
    }));
    let errors = validate_required_arg_resolvability(&g).await;
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("post"), "{}", errors[0]);
    assert!(errors[0].contains("`subject`"), "{}", errors[0]);
    assert!(errors[0].contains("GMAIL_SEND_EMAIL"), "{}", errors[0]);
}

#[tokio::test]
async fn validate_required_arg_resolvability_accepts_a_fully_resolved_graph() {
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "GMAIL_SEND_EMAIL",
                "args": { "recipient_email": "a@b.com", "subject": "hello", "body": "hi there" } } }
        ],
        "edges": [ { "from_node": "t", "to_node": "post" } ]
    }));
    let errors = validate_required_arg_resolvability(&g).await;
    assert!(errors.is_empty(), "{errors:?}");
}

#[tokio::test]
async fn validate_required_arg_resolvability_ignores_native_and_dynamic_slugs() {
    // `oh:` native tools and `=`-derived slugs have no external-provider
    // rejection mode this gate should be checking.
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "prep", "kind": "code", "name": "Prep",
              "config": { "language": "javascript", "source": "return {};" } },
            { "id": "native", "kind": "tool_call", "name": "Native",
              "config": { "slug": "oh:web_search",
                "args": { "query": "=item.nonexistent_field" } } },
            { "id": "dynamic", "kind": "tool_call", "name": "Dynamic",
              "config": { "slug": "=item.nonexistent_field",
                "args": { "x": "=item.nonexistent_field" } } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "prep" },
            { "from_node": "prep", "to_node": "native" },
            { "from_node": "native", "to_node": "dynamic" }
        ]
    }));
    let errors = validate_required_arg_resolvability(&g).await;
    assert!(errors.is_empty(), "{errors:?}");
}

#[tokio::test]
async fn mock_opaque_tool_call_upstream_ref_matches_native_and_composio_upstreams() {
    // Both a Composio curated action and a native `oh:` tool are opaque-echoed
    // by the mock sandbox, so a null bound to EITHER is unverifiable (Some).
    // An `agent` / `code` upstream's real output IS produced by the sandbox, and
    // a `=`-dynamic slug is unknowable, so a null bound to those is genuine (None).
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "code_up", "kind": "code", "name": "Code",
              "config": { "language": "javascript", "source": "return {};" } },
            { "id": "agent_up", "kind": "agent", "name": "Agent",
              "config": { "agent_ref": "researcher", "prompt": "x" } },
            { "id": "native_up", "kind": "tool_call", "name": "Link",
              "config": { "slug": "oh:storage_get_link", "args": { "file_id": "f" } } },
            { "id": "composio_up", "kind": "tool_call", "name": "Profile",
              "config": { "slug": "GMAIL_GET_PROFILE", "args": {} } },
            { "id": "dyn_up", "kind": "tool_call", "name": "Dyn",
              "config": { "slug": "=item.slug", "args": {} } },
            { "id": "sink", "kind": "tool_call", "name": "Sink",
              "config": { "slug": "GMAIL_SEND_EMAIL", "args": {} } }
        ],
        "edges": []
    }));
    let up = |expr: &str| mock_opaque_tool_call_upstream_ref(expr, &g, "sink").map(str::to_string);
    assert_eq!(
        up("=nodes.native_up.item.json.url").as_deref(),
        Some("native_up")
    );
    assert_eq!(
        up("=nodes.composio_up.item.json.data.emailAddress").as_deref(),
        Some("composio_up")
    );
    assert_eq!(up("=nodes.agent_up.item.json.field"), None);
    assert_eq!(up("=nodes.code_up.item.json.field"), None);
    assert_eq!(up("=nodes.dyn_up.item.json.x"), None);
}

#[tokio::test]
async fn validate_required_arg_resolvability_downgrades_null_from_native_tool_call_upstream() {
    // #5148's chain: a Composio `send` binds its `attachment` to a native
    // `oh:storage_get_link` node's `url`. That `url` is null in the echo sandbox
    // (native tools are opaque-echoed), but the wiring is correct, so the gate
    // must NOT reject it. Before the native-upstream carve-out it did — the loop
    // that halted the live "fix with agent" self-repair.
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "prep", "kind": "code", "name": "Prep",
              "config": { "language": "javascript", "source": "return {};" } },
            { "id": "get_link", "kind": "tool_call", "name": "Link",
              "config": { "slug": "oh:storage_get_link", "args": { "file_id": "f_1" } } },
            { "id": "send", "kind": "tool_call", "name": "Send",
              "config": { "slug": "GMAIL_SEND_EMAIL",
                "args": { "recipient_email": "a@b.com", "subject": "hi", "body": "there",
                          "attachment": "=nodes.get_link.item.json.url" } } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "prep" },
            { "from_node": "prep", "to_node": "get_link" },
            { "from_node": "get_link", "to_node": "send" }
        ]
    }));
    let errors = validate_required_arg_resolvability(&g).await;
    assert!(
        errors.is_empty(),
        "a native-upstream attachment null must be downgraded, got: {errors:?}"
    );
}

#[tokio::test]
async fn native_file_attachment_chain_passes_required_arg_resolvability() {
    // Drift check that was missing pre-merge: author #5148's OWN documented
    // `produce -> oh:storage_upload_file -> oh:storage_get_link -> send` chain
    // and assert the null-arg gate (the exact gate that rejected it in the live
    // "fix with agent" loop) now passes it. Targets `validate_required_arg_
    // resolvability` directly (deterministic, no live catalog) rather than
    // `run_builder_gates`, whose connection/contract gates need live Composio.
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "make_page", "kind": "code", "name": "Write",
              "config": { "language": "javascript", "source": "return {};" } },
            { "id": "upload", "kind": "tool_call", "name": "Upload",
              "config": { "slug": "oh:storage_upload_file", "args": { "path": "report.html" } } },
            { "id": "get_link", "kind": "tool_call", "name": "Link",
              "config": { "slug": "oh:storage_get_link",
                "args": { "file_id": "=nodes.upload.item.json.file_id", "expires_in_seconds": 900 } } },
            { "id": "send", "kind": "tool_call", "name": "Send",
              "config": { "slug": "GMAIL_SEND_EMAIL",
                "args": { "recipient_email": "a@b.com", "subject": "AI trends", "body": "attached",
                          "attachment": "=nodes.get_link.item.json.url" } } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "make_page" },
            { "from_node": "make_page", "to_node": "upload" },
            { "from_node": "upload", "to_node": "get_link" },
            { "from_node": "get_link", "to_node": "send" }
        ]
    }));
    let errors = validate_required_arg_resolvability(&g).await;
    assert!(
        errors.is_empty(),
        "the documented native attachment chain must pass the null-arg gate, got: {errors:?}"
    );
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

/// (Codex feedback on PR #4826) This gate sandbox-runs every graph against
/// `json!({})` as the trigger payload, so a `tool_call` arg wired straight to
/// the trigger's own data — `"to": "=item.email"` on a node whose only
/// predecessor is the trigger — always resolves `null` here, even though a
/// real webhook/app-event/manual trigger fires with a real payload. Hard-
/// rejecting that blocked every ordinary trigger-bound workflow. Contrast
/// with `validate_required_arg_resolvability_rejects_a_null_resolved_arg`
/// above, where the same `=item.<field>` shorthand addresses a real
/// (non-trigger) upstream node and stays a hard reject.
#[tokio::test]
async fn validate_required_arg_resolvability_allows_a_trigger_scoped_null_arg() {
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Webhook" },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "GMAIL_SEND_EMAIL",
                "args": { "recipient_email": "a@b.com", "subject": "hi", "body": "=item.email" } } }
        ],
        "edges": [ { "from_node": "t", "to_node": "post" } ]
    }));
    let errors = validate_required_arg_resolvability(&g).await;
    assert!(errors.is_empty(), "{errors:?}");
}

/// The `nodes.<id>...` explicit-addressing form of the real B18 bug: an arg
/// wired to a specific upstream (non-trigger) node's output path that never
/// exists there. Unlike the trigger-scoped case above, this stays broken
/// regardless of what the trigger payload looks like at runtime, so it must
/// still hard-reject.
#[tokio::test]
async fn validate_required_arg_resolvability_rejects_an_explicit_nodes_reference() {
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "build_body", "kind": "code", "name": "Build Body",
              "config": { "language": "javascript", "source": "return {};" } },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "GMAIL_SEND_EMAIL",
                "args": { "recipient_email": "a@b.com",
                  "subject": "=nodes.build_body.item.subject" } } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "build_body" },
            { "from_node": "build_body", "to_node": "post" }
        ]
    }));
    let errors = validate_required_arg_resolvability(&g).await;
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("`subject`"), "{}", errors[0]);
    assert!(errors[0].contains("nodes.build_body"), "{}", errors[0]);
}

/// A required tool arg wired to a PLAIN agent node's (`no agent_ref`)
/// `output_parser.schema` field must pass this sandbox gate: the schema-aware
/// mock LLM (wired above via `caps.llm = SchemaAwareMockLlm`) synthesizes a
/// schema-valid completion, so the agent's output-parser sub-port succeeds and
/// the downstream `=nodes.<agent>.item.json.<field>` binding resolves to a typed
/// placeholder (non-null) instead of the run aborting on a schema-validation
/// failure. Without the mock LLM this gate would sink `propose_workflow`/`save`
/// on a correctly-built graph (the vendored `MockLlm` echo fails the sub-port).
#[tokio::test]
async fn validate_required_arg_resolvability_accepts_a_schema_agent_field_binding() {
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "summarize", "kind": "agent", "name": "Summarize",
              "config": { "prompt": "summarize the thread",
                "output_parser": { "schema": { "type": "object",
                    "required": ["channel"],
                    "properties": { "channel": { "type": "string" } } } } } },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "SLACK_SEND_MESSAGE",
                "args": { "channel": "=nodes.summarize.item.json.channel" } } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "summarize" },
            { "from_node": "summarize", "to_node": "post" }
        ]
    }));
    let errors = validate_required_arg_resolvability(&g).await;
    assert!(errors.is_empty(), "{errors:?}");
}

/// WS6: a required arg wired to the OUTPUT of an upstream Composio `tool_call`
/// must NOT be hard-rejected by this gate. The echo sandbox renders a Composio
/// `tool_call` as `{tool, args, connection}` and can never produce its real
/// output fields, so `=nodes.<composio>.item.json.data.<field>` resolves `null`
/// here even when the wiring is perfectly correct — rejecting it would block a
/// possibly-correct graph from ever being proposed (the transcript false
/// negative). Contrast `..._rejects_an_explicit_nodes_reference` above, where
/// the same explicit-`nodes` form addresses a `code` node (whose real output
/// the sandbox DOES produce) and stays a hard reject.
#[tokio::test]
async fn validate_required_arg_resolvability_downgrades_a_composio_tool_call_upstream_binding() {
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "get_me", "kind": "tool_call", "name": "Who am I",
              "config": { "slug": "TWITTER_USER_LOOKUP_ME", "args": {} } },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "GMAIL_SEND_EMAIL",
                "args": { "recipient_email": "a@b.com", "subject": "hi",
                  "body": "=nodes.get_me.item.json.data.username" } } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "get_me" },
            { "from_node": "get_me", "to_node": "post" }
        ]
    }));
    let errors = validate_required_arg_resolvability(&g).await;
    assert!(
        errors.is_empty(),
        "a binding to a Composio tool_call's output is UNVERIFIABLE, not a hard reject: {errors:?}"
    );
}

/// WS6 companion: the implicit `=item...` form of the same case — `post`'s only
/// predecessor is a Composio `tool_call`, so `=item.json.data.username`
/// addresses that node's (echo-only) output and is likewise unverifiable, not a
/// reject.
#[tokio::test]
async fn validate_required_arg_resolvability_downgrades_an_item_scoped_composio_upstream_binding() {
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "get_me", "kind": "tool_call", "name": "Who am I",
              "config": { "slug": "TWITTER_USER_LOOKUP_ME", "args": {} } },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "GMAIL_SEND_EMAIL",
                "args": { "recipient_email": "a@b.com", "subject": "hi",
                  "body": "=item.json.data.username" } } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "get_me" },
            { "from_node": "get_me", "to_node": "post" }
        ]
    }));
    let errors = validate_required_arg_resolvability(&g).await;
    assert!(errors.is_empty(), "{errors:?}");
}
