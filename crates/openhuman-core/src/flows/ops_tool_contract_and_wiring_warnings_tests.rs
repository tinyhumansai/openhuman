use super::*;
use crate::integrations::composio::catalog::{seed_live_catalog_cache, ToolContract};

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

#[tokio::test]
async fn flows_get_tool_contract_rpc_warns_for_runtime_gated_uncurated_action() {
    seed_live_catalog_cache(
        "airtable",
        vec![ToolContract {
            slug: "AIRTABLE_OBSCURE_RPC_ACTION".to_string(),
            toolkit: "airtable".to_string(),
            description: None,
            required_args: vec![],
            input_schema: None,
            output_fields: vec![],
            output_schema: None,
            primary_array_path: None,
            is_curated: false,
        }],
    );

    let outcome = flows_get_tool_contract(&Config::default(), "AIRTABLE_OBSCURE_RPC_ACTION")
        .await
        .expect("seeded catalog lookup should succeed");

    assert_eq!(
        outcome.value["contract"]["slug"],
        "AIRTABLE_OBSCURE_RPC_ACTION"
    );
    assert_eq!(outcome.value["contract"]["is_curated"], false);
    assert!(
        outcome.value["runtime_gate"]
            .as_str()
            .is_some_and(|warning| warning.contains("REJECTED on every real run")),
        "RPC should explain the runtime rejection: {}",
        outcome.value
    );
}

// ── TinyFlows wiring warnings and the single hard required-arg gate ────────

/// Required args have one authoritative authoring check: the hard host gate.
/// The TinyFlows advisory surface reports portable wiring diagnostics only,
/// so this condition must not produce a duplicate warning.
#[tokio::test]
async fn wiring_warnings_omits_missing_required_arg_advisory_but_host_gate_rejects() {
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
    let warnings =
        graph_wiring_warnings(&crate::core::runtime::DispatchCtx::unbound(), &config, &g)
            .await
            .expect("TinyFlows wiring warnings should succeed");
    assert!(warnings.is_empty(), "{warnings:?}");
    let graph_json = serde_json::to_value(&g).unwrap();
    let errors = crate::flows::host_gates::validate_tool_contracts(&config, &graph_json).await;
    assert!(
        errors
            .iter()
            .any(|error| error.contains("text") && error.contains("post")),
        "the hard host gate must remain authoritative: {errors:?}"
    );
}

#[tokio::test]
async fn graph_wiring_warnings_flags_a_downstream_field_not_in_output_fields() {
    seed_live_catalog_cache("slack", vec![seeded_slack_send_contract()]);
    let config = Config::default();
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "SLACK_SEND_MESSAGE",
                "args": { "channel": "#general", "text": "hi" } } },
            { "id": "xform", "kind": "transform", "name": "Log",
              // Correctly `data.`-prefixed (a real tool_call's payload is
              // always nested under `data`), but the field itself isn't in
              // SLACK_SEND_MESSAGE's real output_fields (`ts`/`channel`) —
              // must WARN, not reject.
              "config": { "set": { "note": "=nodes.post.item.json.data.not_a_real_field" } } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "post" },
            { "from_node": "post", "to_node": "xform" }
        ]
    }));
    let warnings =
        graph_wiring_warnings(&crate::core::runtime::DispatchCtx::unbound(), &config, &g)
            .await
            .expect("TinyFlows wiring warnings should succeed");
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("not_a_real_field") && w.contains("post")),
        "{warnings:?}"
    );
}

#[tokio::test]
async fn graph_wiring_warnings_is_silent_when_the_downstream_field_is_real() {
    seed_live_catalog_cache("slack", vec![seeded_slack_send_contract()]);
    let config = Config::default();
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "SLACK_SEND_MESSAGE",
                "args": { "channel": "#general", "text": "hi" } } },
            { "id": "xform", "kind": "transform", "name": "Log",
              // `data.ts` — correctly dereferences the Composio execute
              // envelope's `data` wrapper before the real field name.
              "config": { "set": { "note": "=nodes.post.item.json.data.ts" } } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "post" },
            { "from_node": "post", "to_node": "xform" }
        ]
    }));
    let warnings =
        graph_wiring_warnings(&crate::core::runtime::DispatchCtx::unbound(), &config, &g)
            .await
            .expect("TinyFlows wiring warnings should succeed");
    assert!(
        !warnings.iter().any(|w| w.contains("not in")),
        "a real output field must not warn: {warnings:?}"
    );
}

/// B1 regression test: the exact "hollow run" bug. Before this fix, a
/// binding like `=nodes.post.item.json.ts` (a REAL field name, but missing
/// the `data.` segment every Composio `tool_call`'s runtime output wraps its
/// payload in) was silently accepted here — it looks like a legitimate
/// binding to a known output field, but resolves `null` at runtime because
/// the real value lives one level deeper, under `data`. This must now WARN.
#[tokio::test]
async fn graph_wiring_warnings_flags_a_downstream_binding_missing_the_data_prefix() {
    seed_live_catalog_cache("slack", vec![seeded_slack_send_contract()]);
    let config = Config::default();
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "SLACK_SEND_MESSAGE",
                "args": { "channel": "#general", "text": "hi" } } },
            { "id": "xform", "kind": "transform", "name": "Log",
              // `ts` IS a real SLACK_SEND_MESSAGE output field — but without
              // the `data.` prefix this is GUARANTEED to resolve null.
              "config": { "set": { "note": "=nodes.post.item.json.ts" } } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "post" },
            { "from_node": "post", "to_node": "xform" }
        ]
    }));
    let warnings =
        graph_wiring_warnings(&crate::core::runtime::DispatchCtx::unbound(), &config, &g)
            .await
            .expect("TinyFlows wiring warnings should succeed");
    assert!(
        warnings.iter().any(|w| w.contains("item.json.data.ts")
            && w.contains("post")
            && w.contains("payload is wrapped in `data`")),
        "{warnings:?}"
    );
}

/// Codex feedback on this PR: a binding to the WHOLE payload
/// (`=nodes.post.item.json.data`, e.g. wiring an agent's `input_context` off
/// the entire tool_call result) must NOT be flagged as "missing the `data.`
/// segment" — it already IS the `data` field, there's nothing to strip a
/// prefix off of. Before this fix the code suggested rewiring to the
/// nonsense `item.json.data.data`.
#[tokio::test]
async fn graph_wiring_warnings_is_silent_for_a_whole_payload_binding() {
    seed_live_catalog_cache("slack", vec![seeded_slack_send_contract()]);
    let config = Config::default();
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "SLACK_SEND_MESSAGE",
                "args": { "channel": "#general", "text": "hi" } } },
            { "id": "xform", "kind": "transform", "name": "Log",
              "config": { "set": { "note": "=nodes.post.item.json.data" } } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "post" },
            { "from_node": "post", "to_node": "xform" }
        ]
    }));
    assert!(
        graph_wiring_warnings(&crate::core::runtime::DispatchCtx::unbound(), &config, &g)
            .await
            .expect("TinyFlows wiring warnings should succeed")
            .is_empty(),
        "{:?}",
        graph_wiring_warnings(&crate::core::runtime::DispatchCtx::unbound(), &config, &g)
            .await
            .expect("TinyFlows wiring warnings should succeed")
    );
}

/// Codex feedback on this PR: `ComposioExecuteResponse`'s OTHER top-level
/// envelope fields (`successful`, `error`, `costUsd`, `markdownFormatted`)
/// live alongside `data`, not inside it — a binding straight to one of
/// these is real and legitimate. Before this fix the code flagged
/// `.item.json.successful` / `.item.json.error` as missing the `data.`
/// segment and suggested the nonsense `item.json.data.successful`.
#[tokio::test]
async fn graph_wiring_warnings_is_silent_for_composio_envelope_metadata_fields() {
    seed_live_catalog_cache("slack", vec![seeded_slack_send_contract()]);
    let config = Config::default();
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "SLACK_SEND_MESSAGE",
                "args": { "channel": "#general", "text": "hi" } } },
            { "id": "xform", "kind": "transform", "name": "Log",
              "config": { "set": {
                "ok": "=nodes.post.item.json.successful",
                "err": "=nodes.post.item.json.error"
              } } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "post" },
            { "from_node": "post", "to_node": "xform" }
        ]
    }));
    assert!(
        graph_wiring_warnings(&crate::core::runtime::DispatchCtx::unbound(), &config, &g)
            .await
            .expect("TinyFlows wiring warnings should succeed")
            .is_empty(),
        "{:?}",
        graph_wiring_warnings(&crate::core::runtime::DispatchCtx::unbound(), &config, &g)
            .await
            .expect("TinyFlows wiring warnings should succeed")
    );
}

#[tokio::test]
async fn graph_wiring_warnings_suggests_the_real_split_out_path() {
    let mut contract = seeded_slack_send_contract();
    contract.slug = "SLACKFANOUT_SEND_MESSAGE".to_string();
    contract.toolkit = "slackfanout".to_string();
    contract.primary_array_path = Some("data.messages".to_string());
    seed_live_catalog_cache("slackfanout", vec![contract]);
    let config = Config::default();
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "SLACKFANOUT_SEND_MESSAGE",
                "args": { "channel": "#general", "text": "hi" } } },
            { "id": "split", "kind": "split_out", "name": "Split",
              "config": { "path": "items" } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "post" },
            { "from_node": "post", "to_node": "split" }
        ]
    }));
    let warnings =
        graph_wiring_warnings(&crate::core::runtime::DispatchCtx::unbound(), &config, &g)
            .await
            .expect("TinyFlows wiring warnings should succeed");
    assert!(
        warnings.iter().any(|w| w.contains("json.data.messages")),
        "{warnings:?}"
    );
}

/// B12 enforcement: a `split_out.path` that resolves to a NON-array (an
/// object, here) against a KNOWN output schema is flagged even though the
/// action names no array anywhere (`primary_array_path` is `None`) — there
/// is nothing to *suggest*, but a definite non-array hit is still a strong
/// "wrong array path" signal worth catching at build time.
#[tokio::test]
async fn graph_wiring_warnings_flags_a_split_out_path_that_resolves_to_a_non_array() {
    // seeded_slack_send_contract's output_schema names only scalar fields
    // (ts/channel) — a real, known schema with no array in it anywhere.
    let mut contract = seeded_slack_send_contract();
    contract.slug = "NONARRAYFANOUT_SEND_MESSAGE".to_string();
    contract.toolkit = "nonarrayfanout".to_string();
    seed_live_catalog_cache("nonarrayfanout", vec![contract]);
    let config = Config::default();
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "NONARRAYFANOUT_SEND_MESSAGE",
                "args": { "channel": "#general", "text": "hi" } } },
            { "id": "split", "kind": "split_out", "name": "Split",
              "config": { "path": "json.data" } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "post" },
            { "from_node": "post", "to_node": "split" }
        ]
    }));
    let warnings =
        graph_wiring_warnings(&crate::core::runtime::DispatchCtx::unbound(), &config, &g)
            .await
            .expect("TinyFlows wiring warnings should succeed");
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("split") && w.contains("known non-array value there")),
        "{warnings:?}"
    );
}

/// The non-array enforcement stays SILENT when the action's output schema is
/// genuinely unknown (not just "known but arrayless") — nothing real to check
/// the path against, so no false positive.
#[tokio::test]
async fn graph_wiring_warnings_is_silent_on_split_out_when_schema_is_wholly_unknown() {
    let contract = ToolContract {
        slug: "UNKNOWNSCHEMA_DO_THING".to_string(),
        toolkit: "unknownschema".to_string(),
        description: None,
        required_args: vec![],
        input_schema: None,
        output_fields: vec![],
        output_schema: None,
        primary_array_path: None,
        is_curated: true,
    };
    seed_live_catalog_cache("unknownschema", vec![contract]);
    let config = Config::default();
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "UNKNOWNSCHEMA_DO_THING", "args": {} } },
            { "id": "split", "kind": "split_out", "name": "Split",
              "config": { "path": "json.data" } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "post" },
            { "from_node": "post", "to_node": "split" }
        ]
    }));
    assert!(
        graph_wiring_warnings(&crate::core::runtime::DispatchCtx::unbound(), &config, &g)
            .await
            .expect("TinyFlows wiring warnings should succeed")
            .is_empty(),
        "{:?}",
        graph_wiring_warnings(&crate::core::runtime::DispatchCtx::unbound(), &config, &g)
            .await
            .expect("TinyFlows wiring warnings should succeed")
    );
}

/// B12 end-to-end: the EXACT live bug shape (flow "funny reminders v2").
/// `GITHUB_LIST_REPOSITORY_ISSUES`-equivalent contract has NO schema at all
/// (`output_schema: None`, `primary_array_path: None` — verified live for
/// every GitHub action), so before a probe the enforcement above has nothing
/// to check the configured `"json.data"` against and stays silent. Once
/// `get_tool_output_sample` has probed the slug (seeded here via
/// `seed_probe_cache`, standing in for a real bounded call), the cached
/// `primary_array_path` overrides the schema-derived (absent) hint and the
/// EXISTING mismatch-suggestion path fires with the real nested path.
#[tokio::test]
async fn graph_wiring_warnings_suggests_the_probed_split_out_path_when_schema_is_unknown() {
    let contract = ToolContract {
        slug: "GHPROBEFANOUT_LIST_REPOSITORY_ISSUES".to_string(),
        toolkit: "ghprobefanout".to_string(),
        description: None,
        required_args: vec!["owner".to_string(), "repo".to_string()],
        input_schema: None,
        output_fields: vec![],
        output_schema: None,
        primary_array_path: None,
        is_curated: true,
    };
    seed_live_catalog_cache("ghprobefanout", vec![contract]);
    seed_probe_cache(
        "GHPROBEFANOUT_LIST_REPOSITORY_ISSUES",
        ProbedOutputSample {
            primary_array_path: Some("data.issues".to_string()),
            output_fields: vec!["issues".to_string(), "total_count".to_string()],
            sample: json!({ "data": { "issues": [], "total_count": 0 } }),
        },
    );
    let config = Config::default();
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "GHPROBEFANOUT_LIST_REPOSITORY_ISSUES",
                "args": { "owner": "acme", "repo": "widgets" } } },
            // The exact wrong guess observed live: whole-payload access
            // instead of the real nested `data.issues`.
            { "id": "split", "kind": "split_out", "name": "Split",
              "config": { "path": "json.data" } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "post" },
            { "from_node": "post", "to_node": "split" }
        ]
    }));
    let warnings =
        graph_wiring_warnings(&crate::core::runtime::DispatchCtx::unbound(), &config, &g)
            .await
            .expect("TinyFlows wiring warnings should succeed");
    assert!(
        warnings.iter().any(|w| w.contains("json.data.issues")),
        "{warnings:?}"
    );

    // Fixed: once config.path matches the probed real path, the warning
    // clears.
    let fixed = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "GHPROBEFANOUT_LIST_REPOSITORY_ISSUES",
                "args": { "owner": "acme", "repo": "widgets" } } },
            { "id": "split", "kind": "split_out", "name": "Split",
              "config": { "path": "json.data.issues" } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "post" },
            { "from_node": "post", "to_node": "split" }
        ]
    }));
    assert!(
        graph_wiring_warnings(
            &crate::core::runtime::DispatchCtx::unbound(),
            &config,
            &fixed
        )
        .await
        .expect("TinyFlows wiring warnings should succeed")
        .is_empty(),
        "{:?}",
        graph_wiring_warnings(
            &crate::core::runtime::DispatchCtx::unbound(),
            &config,
            &fixed
        )
        .await
        .expect("TinyFlows wiring warnings should succeed")
    );
}

#[tokio::test]
async fn graph_wiring_warnings_resolves_a_separated_action_slug_alias() {
    seed_live_catalog_cache("googledrive", vec![seeded_alias_contract()]);
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "find", "kind": "tool_call", "name": "Find",
              "config": { "slug": "GOOGLE_DRIVE_FIND_FILE", "args": {} } },
            { "id": "split", "kind": "split_out", "name": "Split",
              "config": { "path": "json.data.wrong" } },
            { "id": "shape", "kind": "transform", "name": "Shape",
              "config": { "set": { "file": "=nodes.find.item.json.data.wrong" } } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "find" },
            { "from_node": "find", "to_node": "split" },
            { "from_node": "find", "to_node": "shape" }
        ]
    }));

    let warnings = graph_wiring_warnings(
        &crate::core::runtime::DispatchCtx::unbound(),
        &Config::default(),
        &g,
    )
    .await
    .expect("TinyFlows wiring warnings should succeed");

    assert_eq!(
        warnings.len(),
        2,
        "alias should reach both wiring checks: {warnings:?}"
    );
    assert!(
        warnings
            .iter()
            .any(|warning| warning.contains("json.data.files")),
        "{warnings:?}"
    );
    assert!(
        warnings.iter().any(|warning| warning.contains("wrong")),
        "{warnings:?}"
    );
}
