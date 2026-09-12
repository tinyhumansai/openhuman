use super::*;

/// (Codex feedback on this PR) `notion` ships a static curated catalog
/// (`catalog_for_toolkit`), so at RUNTIME `flow_tool_allowed`'s Path A
/// hard-rejects any slug `find_curated` doesn't recognize — even a real,
/// live action. Without this check, a real-but-uncurated action for a
/// statically-catalogued toolkit would pass authoring/save here and then
/// fail every single run as "tool not permitted". Uses its own toolkit key
/// (`notion`, not `slack`/`gmail`) since it seeds different `is_curated`
/// content than every other test sharing those keys.
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

// ── graph_wiring_warnings: required-arg advisory + output-field/split_out.path
//    advisories (Part 2c/2d) ────────────────────────────────────────────────

/// `graph_wiring_warnings`'s own required-arg check, exercised DIRECTLY
/// (rather than through `revise_workflow`/`save_workflow`, where the newer
/// `validate_tool_contracts` hard-rejects the identical condition first —
/// see `revise_workflow_rejects_a_missing_required_composio_arg` in
/// `builder_tools_tests.rs`). Keeps this advisory code path covered for any
/// caller that consults `graph_wiring_warnings` without also running the
/// hard gate first.
#[tokio::test]
async fn graph_wiring_warnings_flags_a_missing_required_arg() {
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
    let warnings = graph_wiring_warnings(&config, &g).await;
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("`text`") && w.contains("post")),
        "{warnings:?}"
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
    let warnings = graph_wiring_warnings(&config, &g).await;
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
    let warnings = graph_wiring_warnings(&config, &g).await;
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
    let warnings = graph_wiring_warnings(&config, &g).await;
    assert!(
        warnings.iter().any(|w| w.contains("item.json.data.ts")
            && w.contains("post")
            && w.contains("wraps its payload in `data`")),
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
        graph_wiring_warnings(&config, &g).await.is_empty(),
        "{:?}",
        graph_wiring_warnings(&config, &g).await
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
        graph_wiring_warnings(&config, &g).await.is_empty(),
        "{:?}",
        graph_wiring_warnings(&config, &g).await
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
    let warnings = graph_wiring_warnings(&config, &g).await;
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
    let warnings = graph_wiring_warnings(&config, &g).await;
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("split") && w.contains("does not name an array")),
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
        graph_wiring_warnings(&config, &g).await.is_empty(),
        "{:?}",
        graph_wiring_warnings(&config, &g).await
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
    let warnings = graph_wiring_warnings(&config, &g).await;
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
        graph_wiring_warnings(&config, &fixed).await.is_empty(),
        "{:?}",
        graph_wiring_warnings(&config, &fixed).await
    );
}
