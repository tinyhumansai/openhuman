use super::*;

#[tokio::test]
async fn flows_cancel_run_of_a_completed_with_warnings_run_errors() {
    // A settled `completed_with_warnings` run (run honesty, PR2) must be just
    // as terminal as a plain `completed` run — otherwise `flows_cancel_run`
    // falls through to its not-in-flight path and overwrites the row (and the
    // flow summary) as `"cancelled"`, silently discarding the warning status
    // the run already recorded.
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let created = flows_create(&config, "demo".to_string(), trigger_only_graph(), false)
        .await
        .unwrap();

    let run = flows_run(
        &config,
        &created.value.id,
        json!({}),
        serde_json::Map::new(),
        FlowRunTrigger::Rpc,
    )
    .await
    .unwrap();
    let thread_id = run.value["thread_id"].as_str().unwrap().to_string();

    // Force the settled row to the warning status directly — an end-to-end
    // null-binding graph isn't needed to exercise this guard.
    // Fixture-only forcing write: the run above already settled `completed`, so
    // `finish_flow_run`'s liveness guard (correctly) refuses a terminal →
    // terminal transition. Staging a row at an arbitrary terminal status is a
    // test concern, not a production one.
    store::force_run_status_for_test(&config, &thread_id, "completed_with_warnings", None).unwrap();

    let err = flows_cancel_run(&config, &thread_id)
        .await
        .expect_err("cancelling a completed_with_warnings run must be a clear error");
    assert!(err.contains("already terminal"), "got: {err}");

    // And the row must still read back as the warning status, not overwritten.
    let run_row = flows_get_run(&config, &thread_id).await.unwrap();
    assert_eq!(run_row.value.status, "completed_with_warnings");
}

#[tokio::test]
async fn flows_cancel_run_of_an_interrupted_run_errors() {
    // An `interrupted` run (bug B42 — reconciled by the drop-guard / boot
    // sweep) is terminal: cancelling it must be a clear error, never fall
    // through to the not-in-flight path and clobber the row to `"cancelled"`,
    // discarding the interruption reason it already carries.
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let created = flows_create(&config, "demo".to_string(), trigger_only_graph(), false)
        .await
        .unwrap();

    let run = flows_run(
        &config,
        &created.value.id,
        json!({}),
        serde_json::Map::new(),
        FlowRunTrigger::Rpc,
    )
    .await
    .unwrap();
    let thread_id = run.value["thread_id"].as_str().unwrap().to_string();

    // Force the settled row to `interrupted` directly.
    // Fixture-only forcing write — see the sibling test above: the run has
    // already settled, and `finish_flow_run` now (correctly) refuses a
    // terminal -> terminal transition.
    store::force_run_status_for_test(
        &config,
        &thread_id,
        "interrupted",
        Some("interrupted mid-flight"),
    )
    .unwrap();

    let err = flows_cancel_run(&config, &thread_id)
        .await
        .expect_err("cancelling an interrupted run must be a clear error");
    assert!(err.contains("already terminal"), "got: {err}");

    // And the row must still read back as `interrupted`, not overwritten.
    let run_row = flows_get_run(&config, &thread_id).await.unwrap();
    assert_eq!(run_row.value.status, "interrupted");
    assert_eq!(
        run_row.value.error.as_deref(),
        Some("interrupted mid-flight")
    );
}

#[tokio::test]
async fn flows_cancel_run_missing_run_errors() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let err = flows_cancel_run(&config, "no-such-run")
        .await
        .expect_err("must error for an unknown run");
    assert!(err.contains("not found"));
}

// ── parked-run TTL sweep (issue G4) ───────────────────────────────────────

#[tokio::test]
async fn parked_run_ttl_sweep_expires_stale_runs_but_spares_fresh_ones() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let created = flows_create(&config, "gated".to_string(), approval_gated_graph(), false)
        .await
        .unwrap();

    // Seed a parked run whose "parked since" (finished_at) is far in the past,
    // so it is well beyond the TTL.
    let stale_id = format!("flow:{}:stale-run", created.value.id);
    let ancient = "2000-01-01T00:00:00+00:00";
    store::insert_flow_run(&config, &stale_id, &created.value.id, &stale_id, ancient).unwrap();
    store::finish_flow_run(
        &config,
        &stale_id,
        "pending_approval",
        ancient,
        &[],
        &["gate".to_string()],
        None,
        None,
    )
    .unwrap();

    // A genuinely fresh parked run (just paused now) must survive the sweep.
    let fresh = flows_run(
        &config,
        &created.value.id,
        json!({}),
        serde_json::Map::new(),
        FlowRunTrigger::Rpc,
    )
    .await
    .unwrap();
    let fresh_id = fresh.value["thread_id"].as_str().unwrap().to_string();

    let swept = sweep_expired_parked_runs(&config).await;
    assert_eq!(swept, 1, "only the stale parked run should be swept");

    let stale_row = store::get_flow_run(&config, &stale_id).unwrap().unwrap();
    assert_eq!(stale_row.status, "cancelled");
    assert!(
        stale_row.error.unwrap_or_default().contains("expired"),
        "an expired run's error must note the TTL expiry"
    );

    let fresh_row = store::get_flow_run(&config, &fresh_id).unwrap().unwrap();
    assert_eq!(
        fresh_row.status, "pending_approval",
        "a run parked within the TTL must not be swept"
    );

    // The swept run is no longer resumable.
    let err = flows_resume(
        &config,
        &created.value.id,
        &stale_id,
        vec!["gate".to_string()],
        vec![],
    )
    .await
    .expect_err("an expired parked run must not be resumable");
    assert!(err.contains("not pending approval") || err.contains("no paused run"));
}

#[test]
fn flows_validate_warns_on_unfired_webhook_trigger() {
    let outcome = flows_validate(webhook_trigger_graph());
    assert!(outcome.value.valid, "a webhook graph is structurally valid");
    assert!(outcome.value.errors.is_empty());
    assert_eq!(
        outcome.value.warnings.len(),
        1,
        "an unfired webhook trigger must produce exactly one warning: {:?}",
        outcome.value.warnings
    );
    assert!(
        outcome.value.warnings[0].contains("webhook")
            && outcome.value.warnings[0].contains("does not fire"),
        "warning must name the kind and explain it does not fire: {:?}",
        outcome.value.warnings
    );
}

#[test]
fn flows_validate_does_not_warn_on_schedule_trigger() {
    let outcome = flows_validate(schedule_trigger_graph("0 9 * * *"));
    assert!(outcome.value.valid);
    assert!(
        outcome.value.warnings.is_empty(),
        "a schedule trigger fires — it must not warn: {:?}",
        outcome.value.warnings
    );
}

#[test]
fn flows_validate_reports_error_for_graph_without_trigger() {
    let graph = json!({
        "name": "bad",
        "nodes": [ { "id": "a", "kind": "output_parser", "name": "A" } ],
        "edges": []
    });
    let outcome = flows_validate(graph);
    assert!(!outcome.value.valid);
    assert_eq!(outcome.value.errors.len(), 1);
    assert!(outcome.value.errors[0].contains("trigger"));
    assert!(
        outcome.value.warnings.is_empty(),
        "an invalid graph reports no warnings"
    );
}

#[test]
fn flows_validate_accumulates_every_structural_error() {
    // A graph with several independent problems: no trigger, a duplicate node
    // id, and a dangling edge. Multi-error validation must surface all of them
    // in one call (fail-fast would report only the first).
    let graph = json!({
        "name": "riddled",
        "nodes": [
            { "id": "dup", "kind": "agent", "name": "One" },
            { "id": "dup", "kind": "agent", "name": "Two" }
        ],
        "edges": [ { "from_node": "dup", "to_node": "ghost" } ]
    });
    let outcome = flows_validate(graph);
    assert!(!outcome.value.valid);
    // errors[] and error_details[] must be 1:1.
    assert_eq!(
        outcome.value.errors.len(),
        outcome.value.error_details.len(),
        "errors and error_details must be parallel: {:?} vs {:?}",
        outcome.value.errors,
        outcome.value.error_details
    );
    assert!(
        outcome.value.errors.len() >= 3,
        "expected >=3 accumulated errors, got {:?}",
        outcome.value.errors
    );
    let codes: Vec<&str> = outcome
        .value
        .error_details
        .iter()
        .map(|e| e.code.as_str())
        .collect();
    assert!(codes.contains(&"missing_trigger"), "{codes:?}");
    assert!(codes.contains(&"duplicate_node_id"), "{codes:?}");
    assert!(codes.contains(&"unknown_node"), "{codes:?}");
    // A node-anchored error carries its node id; a graph-wide one does not.
    let dup = outcome
        .value
        .error_details
        .iter()
        .find(|e| e.code == "duplicate_node_id")
        .unwrap();
    assert_eq!(dup.node_id.as_deref(), Some("dup"));
    let missing = outcome
        .value
        .error_details
        .iter()
        .find(|e| e.code == "missing_trigger")
        .unwrap();
    assert_eq!(missing.node_id, None);
}

#[test]
fn flows_validate_reports_unparseable_graph_as_single_error() {
    // A pre-validation failure (an unknown node kind can't deserialize) is a
    // genuine single error, not a structural-error accumulation.
    let graph = json!({
        "name": "bad",
        "nodes": [ { "id": "a", "kind": "not_a_real_kind", "name": "A" } ],
        "edges": []
    });
    let outcome = flows_validate(graph);
    assert!(!outcome.value.valid);
    assert_eq!(outcome.value.errors.len(), 1);
    assert_eq!(outcome.value.error_details.len(), 1);
    assert_eq!(outcome.value.error_details[0].code, "unparseable_graph");
}

#[tokio::test]
async fn flows_set_enabled_surfaces_unfired_trigger_warning_at_enable() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let created = flows_create(
        &config,
        "hooked".to_string(),
        webhook_trigger_graph(),
        false,
    )
    .await
    .unwrap();

    // A webhook trigger is automatic (B29 Rule 1) so `flows_create` leaves it
    // disabled — enable it explicitly here to exercise the enable path's
    // warning.
    let enabled = flows_set_enabled(&config, &created.value.id, true)
        .await
        .unwrap();
    assert!(enabled.value.enabled);
    assert!(
        enabled
            .logs
            .iter()
            .any(|l| l.starts_with("warning:") && l.contains("webhook")),
        "enabling a webhook-trigger flow must surface a loud warning log, got: {:?}",
        enabled.logs
    );
}

#[tokio::test]
async fn flows_set_enabled_schedule_flow_has_no_warning() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let created = flows_create(
        &config,
        "scheduled".to_string(),
        schedule_trigger_graph("0 9 * * *"),
        false,
    )
    .await
    .unwrap();

    let enabled = flows_set_enabled(&config, &created.value.id, true)
        .await
        .unwrap();
    assert!(
        !enabled.logs.iter().any(|l| l.starts_with("warning:")),
        "a schedule-trigger flow must not surface an unfired-trigger warning: {:?}",
        enabled.logs
    );
}

#[test]
fn build_flow_connections_emits_parseable_refs_for_both_kinds() {
    let composio = vec![composio_conn(
        "ca_abc",
        "Gmail",
        "ACTIVE",
        Some("user@example.com"),
    )];
    let http = vec![http_summary("stripe", "bearer")];

    let out = build_flow_connections(composio, http, &[]);
    assert_eq!(out.len(), 2);

    let gmail = &out[0];
    assert_eq!(gmail.kind, "composio");
    // Toolkit is normalized (lowercased) and the ref round-trips through the
    // exact parser the caps seam uses on execution.
    assert_eq!(gmail.connection_ref, "composio:gmail:ca_abc");
    assert_eq!(
        crate::openhuman::flows::tinyflows::caps::composio_connection_id(&gmail.connection_ref),
        Some("ca_abc")
    );
    assert_eq!(gmail.toolkit.as_deref(), Some("gmail"));
    assert_eq!(gmail.display, "Gmail · user@example.com");
    assert!(gmail.scheme.is_none());
    assert!(gmail.platform_user_id.is_none());

    let stripe = &out[1];
    assert_eq!(stripe.kind, "http");
    assert_eq!(stripe.connection_ref, "http_cred:stripe");
    assert_eq!(
        crate::openhuman::flows::tinyflows::caps::http_cred_name(&stripe.connection_ref),
        Some("stripe")
    );
    assert_eq!(stripe.scheme.as_deref(), Some("bearer"));
    assert_eq!(stripe.display, "stripe (bearer)");
    assert!(stripe.toolkit.is_none());
    assert!(stripe.platform_user_id.is_none());
}

#[test]
fn build_flow_connections_skips_non_active_composio_accounts() {
    let composio = vec![
        composio_conn("ca_ok", "notion", "ACTIVE", None),
        composio_conn("ca_pending", "slack", "PENDING", None),
    ];
    let out = build_flow_connections(composio, Vec::new(), &[]);
    assert_eq!(out.len(), 1, "only the ACTIVE connection is surfaced");
    assert_eq!(out[0].connection_ref, "composio:notion:ca_ok");
    // No cached identity → title-cased toolkit alone.
    assert_eq!(out[0].display, "Notion");
}

#[test]
fn build_flow_connections_never_carries_secret_fields() {
    let out = build_flow_connections(
        vec![composio_conn("ca_abc", "gmail", "ACTIVE", Some("u@x.io"))],
        vec![http_summary("stripe", "header")],
        &[],
    );
    let json = serde_json::to_string(&out).unwrap();
    // The serialized picker payload must expose only ref/kind/display/toolkit/
    // scheme/platform_user_id — no secret-bearing key names at all.
    for banned in [
        "secret", "token", "password", "\"key\"", "apiKey", "api_key",
    ] {
        assert!(
            !json
                .to_ascii_lowercase()
                .contains(&banned.to_ascii_lowercase()),
            "serialized FlowConnection leaked a secret-bearing field ({banned}): {json}"
        );
    }
}

#[test]
fn build_flow_connections_attaches_platform_user_id_from_a_seeded_identity() {
    use crate::openhuman::integrations::composio::providers::ConnectedIdentity;

    let composio = vec![composio_conn("ca_slack1", "slack", "ACTIVE", None)];
    let identities = vec![ConnectedIdentity {
        source: "slack".to_string(),
        identifier: "ca_slack1".to_string(),
        user_id: Some("U123ABC".to_string()),
        ..Default::default()
    }];

    let out = build_flow_connections(composio, Vec::new(), &identities);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].platform_user_id.as_deref(), Some("U123ABC"));
}

#[test]
fn build_flow_connections_platform_user_id_is_none_without_a_matching_identity() {
    use crate::openhuman::integrations::composio::providers::ConnectedIdentity;

    // No identities at all.
    let composio = vec![composio_conn("ca_slack1", "slack", "ACTIVE", None)];
    let out = build_flow_connections(composio, Vec::new(), &[]);
    assert_eq!(out.len(), 1);
    assert!(out[0].platform_user_id.is_none());

    // An identity exists, but for a different toolkit/connection — must not
    // cross-wire onto this connection.
    let composio = vec![composio_conn("ca_slack1", "slack", "ACTIVE", None)];
    let identities = vec![ConnectedIdentity {
        source: "gmail".to_string(),
        identifier: "ca_slack1".to_string(),
        user_id: Some("U123ABC".to_string()),
        ..Default::default()
    }];
    let out = build_flow_connections(composio, Vec::new(), &identities);
    assert_eq!(out.len(), 1);
    assert!(out[0].platform_user_id.is_none());
}

#[test]
fn title_case_toolkit_handles_underscores_and_dashes() {
    assert_eq!(title_case_toolkit("gmail"), "Gmail");
    assert_eq!(title_case_toolkit("google_calendar"), "Google Calendar");
    assert_eq!(title_case_toolkit("google-sheets"), "Google Sheets");
    assert_eq!(title_case_toolkit(""), "");
}

#[tokio::test]
async fn flows_list_connections_aggregates_http_creds_and_tolerates_composio() {
    let tmp = TempDir::new().unwrap();
    let mut config = test_config(&tmp);
    // Force Direct mode with no key so the composio source short-circuits to an
    // empty list offline (no network) — proving the aggregation still returns
    // the HTTP-credential half.
    config.composio.mode = crate::openhuman::config::schema::COMPOSIO_MODE_DIRECT.to_string();
    // Secrets in the clear at rest for the test (mirrors the E2E config).
    config.secrets.encrypt = false;

    // Seed one HTTP credential through the same store the op reads.
    let store = HttpCredentialsStore::from_config(&config);
    store
        .upsert(&HttpCredential::bearer("stripe", "sk_live_seed_secret"))
        .unwrap();

    let outcome = flows_list_connections(&config).await.unwrap();
    let refs: Vec<_> = outcome
        .value
        .iter()
        .map(|c| c.connection_ref.as_str())
        .collect();
    assert!(
        refs.contains(&"http_cred:stripe"),
        "http_cred must be surfaced: {refs:?}"
    );

    // The secret must never appear anywhere in the RPC payload.
    let json = serde_json::to_string(&outcome.value).unwrap();
    assert!(
        !json.contains("sk_live_seed_secret"),
        "secret leaked into flows_list_connections payload: {json}"
    );
}

#[tokio::test]
async fn list_suggestions_filters_by_status() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    seed_suggestion(&config, "s1");
    seed_suggestion(&config, "s2");

    let active = flows_list_suggestions(
        &config,
        Some(crate::openhuman::flows::SuggestionStatus::New),
    )
    .await
    .unwrap();
    assert_eq!(active.value.len(), 2);

    // Unfiltered returns all too.
    let all = flows_list_suggestions(&config, None).await.unwrap();
    assert_eq!(all.value.len(), 2);
}

#[tokio::test]
async fn dismiss_and_mark_built_move_suggestions_out_of_active() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    seed_suggestion(&config, "s1");
    seed_suggestion(&config, "s2");

    let d = flows_dismiss_suggestion(&config, "s1").await.unwrap();
    assert_eq!(d.value["dismissed"], json!(true));
    let b = flows_mark_suggestion_built(&config, "s2").await.unwrap();
    assert_eq!(b.value["built"], json!(true));

    // Neither is in the active (New) set anymore.
    let active = flows_list_suggestions(
        &config,
        Some(crate::openhuman::flows::SuggestionStatus::New),
    )
    .await
    .unwrap();
    assert!(active.value.is_empty());
}

#[tokio::test]
async fn dismiss_unknown_suggestion_reports_not_found() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let d = flows_dismiss_suggestion(&config, "missing").await.unwrap();
    assert_eq!(d.value["dismissed"], json!(false));
}

// ─────────────────────────────────────────────────────────────────────────────
// FlowStreamTarget (Phase B copilot/scout streaming) — pure param plumbing.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn flow_stream_target_none_without_thread_id() {
    // No thread → headless run, regardless of request_id.
    assert!(FlowStreamTarget::from_params(None, None).is_none());
    assert!(FlowStreamTarget::from_params(None, Some("r-1".to_string())).is_none());
}

#[test]
fn flow_stream_target_blank_thread_id_is_absent() {
    // Whitespace-only thread id is treated as no thread (callers pass raw input).
    assert!(FlowStreamTarget::from_params(Some("   ".to_string()), None).is_none());
    assert!(FlowStreamTarget::from_params(Some(String::new()), None).is_none());
}

#[test]
fn flow_stream_target_trims_and_keeps_request_id() {
    let t = FlowStreamTarget::from_params(Some("  t-1  ".to_string()), Some("  r-1  ".to_string()))
        .expect("stream target");
    assert_eq!(t.thread_id, "t-1");
    assert_eq!(t.request_id, "r-1");
}

#[test]
fn flow_stream_target_generates_request_id_when_absent_or_blank() {
    // Absent request id → a fresh uuid is minted.
    let a = FlowStreamTarget::from_params(Some("t-1".to_string()), None).expect("target");
    assert!(!a.request_id.is_empty());
    assert_ne!(a.request_id, a.thread_id);
    // Blank request id is treated the same way.
    let b = FlowStreamTarget::from_params(Some("t-1".to_string()), Some("  ".to_string()))
        .expect("target");
    assert!(!b.request_id.is_empty());
    // Two mints are distinct uuids.
    assert_ne!(a.request_id, b.request_id);
}

#[test]
fn binding_to_agent_schema_missing_field_is_rejected() {
    // TinyFlows deliberately permits a schema-less agent because its host
    // runner can return arbitrary structured JSON. A declared schema that
    // omits `channel`, however, proves this binding is unaddressable.
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "summarize", "kind": "agent", "name": "Summarize",
              "config": { "agent_ref": "researcher", "prompt": "summarize",
                "output_parser": { "schema": { "type": "object",
                  "properties": { "summary": { "type": "string" } } } } } },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "SLACK_SEND_MESSAGE",
                "args": { "channel": "=nodes.summarize.item.json.channel" } } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "summarize" },
            { "from_node": "summarize", "to_node": "post" }
        ]
    }));
    let errors = validate_binding_resolvability(&g);
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("post"), "{}", errors[0]);
    assert!(errors[0].contains("channel"), "{}", errors[0]);
    assert!(errors[0].contains("summarize"), "{}", errors[0]);
    assert!(errors[0].contains("output_parser.schema"), "{}", errors[0]);
}

#[test]
fn binding_to_agent_without_any_schema_is_unverifiable_not_rejected() {
    // TinyFlows deliberately permits a schema-less agent (see the test above):
    // with no `output_parser.schema` at all the runtime response shape is
    // host-defined, the bound field MAY exist, and the gate treats the
    // binding as unverifiable rather than invalid. Pinned so the rejection
    // tests above cannot silently drift back to the pre-v0.8.2 contract.
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "summarize", "kind": "agent", "name": "Summarize",
              "config": { "agent_ref": "researcher", "prompt": "summarize" } },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "SLACK_SEND_MESSAGE",
                "args": { "channel": "=nodes.summarize.item.json.channel" } } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "summarize" },
            { "from_node": "summarize", "to_node": "post" }
        ]
    }));
    let errors = validate_binding_resolvability(&g);
    assert_eq!(errors, Vec::<String>::new(), "unverifiable is not invalid");
}

#[test]
fn binding_to_agent_with_schema_missing_field_is_rejected() {
    // A schema IS declared, but it doesn't cover the field the binding reads.
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "summarize", "kind": "agent", "name": "Summarize",
              "config": { "prompt": "summarize",
                "output_parser": { "schema": { "type": "object",
                    "properties": { "summary": { "type": "string" } } } } } },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "SLACK_SEND_MESSAGE",
                "args": { "channel": "=nodes.summarize.item.json.channel" } } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "summarize" },
            { "from_node": "summarize", "to_node": "post" }
        ]
    }));
    let errors = validate_binding_resolvability(&g);
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("channel"), "{}", errors[0]);
}
