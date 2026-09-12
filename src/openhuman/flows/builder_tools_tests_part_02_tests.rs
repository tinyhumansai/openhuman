use super::*;

/// The scope gate runs BEFORE any client/network call, so a Write-scope
/// action is refused entirely offline — this must never depend on a live
/// Composio backend to prove the probe can't perform a real mutation.
#[tokio::test]
async fn get_tool_output_sample_refuses_a_write_scope_action() {
    let tmp = TempDir::new().unwrap();
    let tool = GetToolOutputSampleTool::new(test_config(&tmp));
    let result = tool
        .execute(json!({ "slug": "GMAIL_SEND_EMAIL" }))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("READ-only"), "{}", result.output());
}

/// The connected-toolkit gate runs before the real call too — in a test
/// environment with no backend session, `fetch_connected_integrations`
/// degrades to empty (best-effort, per its own doc), so a Read-scope action
/// against an unconnected toolkit is refused without ever reaching a client.
#[tokio::test]
async fn get_tool_output_sample_refuses_an_unconnected_toolkit() {
    let tmp = TempDir::new().unwrap();
    let tool = GetToolOutputSampleTool::new(test_config(&tmp));
    let result = tool
        .execute(json!({ "slug": "GITHUB_LIST_REPOSITORY_ISSUES" }))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(
        result.output().contains("not connected") || result.output().contains("no active"),
        "{}",
        result.output()
    );
}

// ── dry_run_workflow ─────────────────────────────────────────────────────────

#[test]
fn dry_run_is_side_effect_free_and_ungated() {
    let tool = DryRunWorkflowTool::new(test_config(&TempDir::new().unwrap()));
    assert_eq!(tool.name(), "dry_run_workflow");
    // Mock-only + side-effect-free → PermissionLevel::None, available on every
    // tier including read-only (audit F7).
    assert_eq!(tool.permission_level(), PermissionLevel::None);
    assert!(!tool.external_effect());
}

#[tokio::test]
async fn dry_run_allowed_under_readonly_tier() {
    // F7: dry_run is mock-only and side-effect-free, so a read-only agent must
    // be able to self-verify its own proposal (previously refused).
    let tool = DryRunWorkflowTool::new(test_config(&TempDir::new().unwrap()));
    assert_eq!(tool.permission_level(), PermissionLevel::None);
    let result = tool
        .execute(json!({ "graph": valid_graph() }))
        .await
        .unwrap();
    // Not refused for tier reasons — it actually runs against the mocks.
    assert!(!result.is_error, "{}", result.output());
    assert!(!result.output().to_lowercase().contains("read-only"));
}

#[tokio::test]
async fn dry_run_supervised_runs_against_mock_and_labels_sandbox() {
    let tool = DryRunWorkflowTool::new(test_config(&TempDir::new().unwrap()));
    let result = tool
        .execute(json!({ "graph": valid_graph(), "input": { "x": 1 } }))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(parsed["sandbox"], true);
    assert_eq!(parsed["ok"], true);
    assert!(parsed["note"]
        .as_str()
        .unwrap()
        .to_lowercase()
        .contains("sandbox"));
}

#[tokio::test]
async fn dry_run_exercises_agent_ref_node_via_mock_agent_runner() {
    // A draft whose `agent` node selects a named agent kind (`agent_ref`) routes
    // to the `AgentRunner` capability, not the plain LLM. Before wiring the mock
    // runner the sandbox left `agent: None`, so such a draft errored on a missing
    // capability; now `mock_capabilities_with_agent(MockAgentRunner)` echoes the
    // ref and the dry run goes green — proving the builder can self-test drafts
    // that use agent-kind nodes.
    let tool = DryRunWorkflowTool::new(test_config(&TempDir::new().unwrap()));
    let graph = json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "a", "kind": "agent", "name": "Plan",
              "config": { "agent_ref": "researcher", "prompt": "outline it" } }
        ],
        "edges": [ { "from_node": "t", "to_node": "a" } ]
    });
    let result = tool
        .execute(json!({ "graph": graph, "input": { "topic": "x" } }))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(parsed["sandbox"], true);
    assert_eq!(
        parsed["ok"], true,
        "agent_ref dry-run must be green: {parsed}"
    );
}

#[tokio::test]
async fn dry_run_plain_agent_with_output_parser_schema_is_green() {
    // Regression for the transcript false-failure: a builder-generated `agent`
    // node carries NO `agent_ref`, so the vendored engine routes it to the
    // `llm` slot (not the `AgentRunner`). Before `SchemaAwareMockLlm` the plain
    // `MockLlm` echo (`{ completion, connection }`) failed the node's
    // `output_parser.schema` sub-port with `output_parser: value failed schema
    // validation after auto-fix: missing required property ...`, sinking a
    // correctly-built graph. Now the mock LLM synthesizes a schema-valid object,
    // and a downstream node binds the typed placeholders (non-null).
    let tool = DryRunWorkflowTool::new(test_config(&TempDir::new().unwrap()));
    let graph = json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Schedule",
              "config": { "trigger_kind": "schedule" } },
            { "id": "a", "kind": "agent", "name": "Extract",
              "config": { "prompt": "extract the fields",
                "output_parser": { "schema": { "type": "object",
                    "required": ["subject", "priority", "recipients"],
                    "properties": {
                        "subject": { "type": "string" },
                        "priority": { "type": "integer" },
                        "recipients": { "type": "array" }
                    } } } } },
            // Downstream node binds the schema'd agent fields: proves the
            // placeholders are addressable and resolve to typed (non-null)
            // values, not the vendored echo's opaque `{ completion, ... }`.
            { "id": "down", "kind": "transform", "name": "Route",
              "config": { "set": {
                  "subject": "=nodes.a.item.json.subject",
                  "priority": "=nodes.a.item.json.priority",
                  "recipients": "=nodes.a.item.json.recipients" } } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "a" },
            { "from_node": "a", "to_node": "down" }
        ]
    });
    let result = tool
        .execute(json!({ "graph": graph, "input": { "topic": "launch" } }))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.output());
    let out = result.output();
    assert!(
        !out.to_lowercase().contains("schema validation"),
        "plain agent with a valid schema must not hit the output_parser failure: {out}"
    );
    let parsed: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(parsed["sandbox"], true);
    assert_eq!(
        parsed["ok"], true,
        "plain-agent-with-schema dry-run must be green: {parsed}"
    );
    // The agent envelope's `json` carries the schema-synthesized placeholders.
    // (In the run OUTPUT each Item serializes as `{ json: <value> }`, and the
    // agent's value is the `{json,text,raw}` envelope — hence the double hop.)
    let agent_json = &parsed["output"]["nodes"]["a"]["items"][0]["json"]["json"];
    assert_eq!(agent_json["subject"], "", "{parsed}");
    assert_eq!(agent_json["priority"], 0, "{parsed}");
    assert_eq!(agent_json["recipients"], json!([]), "{parsed}");
    // The downstream node's bindings resolved to those typed placeholders —
    // none of them null.
    let down_json = &parsed["output"]["nodes"]["down"]["items"][0]["json"];
    assert!(!down_json["subject"].is_null(), "{parsed}");
    assert_eq!(down_json["priority"], 0, "{parsed}");
    assert_eq!(down_json["recipients"], json!([]), "{parsed}");
}

#[tokio::test]
async fn dry_run_invalid_graph_is_error() {
    let tool = DryRunWorkflowTool::new(test_config(&TempDir::new().unwrap()));
    let result = tool
        .execute(json!({ "graph": { "nodes": [], "edges": [] } }))
        .await
        .unwrap();
    assert!(result.is_error);
}

#[tokio::test]
async fn dry_run_catches_unwired_required_composio_arg() {
    // Seed the preflight schema cache so no live Composio backend is needed.
    // NOTE: the cache is process-global and other tests seed the `gmail`
    // toolkit too — keep every seeding of GMAIL_SEND_EMAIL identical
    // (`to` + `body`) so test order can't change the outcome.
    seed_live_catalog_cache("gmail", vec![seeded_gmail_send_contract()]);

    let tmp = TempDir::new().unwrap();
    let tool = DryRunWorkflowTool::new(test_config(&tmp));

    let graph_with = |args: Value| {
        json!({
            "nodes": [
                { "id": "t", "kind": "trigger", "name": "Manual" },
                { "id": "send", "kind": "tool_call", "name": "Send email",
                  "config": { "slug": "GMAIL_SEND_EMAIL", "args": args } }
            ],
            "edges": [ { "from_node": "t", "to_node": "send" } ]
        })
    };

    // `to` is a `=`-expression that misses (trigger input has no `email`):
    // the dry run must fail BEFORE the (mock) tool call, naming the field.
    let result = tool
        .execute(json!({
            "graph": graph_with(json!({ "to": "=item.email", "body": "hello" })),
            "input": {}
        }))
        .await
        .unwrap();
    let out = result.output();
    assert!(
        out.contains("`to`") && out.contains("required"),
        "dry run must name the unwired required arg: {out}"
    );

    // The same flow with `to` wired from the trigger passes the preflight.
    let result = tool
        .execute(json!({
            "graph": graph_with(json!({ "to": "=item.email", "body": "hello" })),
            "input": { "email": "a@b.com" }
        }))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(parsed["sandbox"], true);
    assert_eq!(
        parsed["ok"], true,
        "wired flow must dry-run green: {parsed}"
    );
}

// ── dry_run_workflow: null-resolution check ─────────────────────────────────

#[tokio::test]
async fn dry_run_flags_tool_call_arg_null_resolved_from_unschemad_agent() {
    // The `summarize` agent has no `output_parser.schema`, so (via the
    // schema-aware mock agent) its structured output has no `channel` field —
    // the exact "builds but does nothing" shape this check exists to catch.
    let tool = DryRunWorkflowTool::new(test_config(&TempDir::new().unwrap()));
    let graph = json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "summarize", "kind": "agent", "name": "Summarize",
              "config": { "agent_ref": "researcher", "prompt": "summarize" } },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "oh:noop",
                "args": { "channel": "=nodes.summarize.item.json.channel" } } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "summarize" },
            { "from_node": "summarize", "to_node": "post" }
        ]
    });

    let result = tool.execute(json!({ "graph": graph })).await.unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(
        parsed["sandbox"], true,
        "still labeled a sandbox result: {parsed}"
    );
    assert_eq!(
        parsed["ok"], false,
        "a null-resolved tool_call arg must fail the dry run: {parsed}"
    );
    let null_resolutions = parsed["null_resolutions"]
        .as_array()
        .expect("null_resolutions array");
    assert_eq!(null_resolutions.len(), 1, "{parsed}");
    assert_eq!(null_resolutions[0]["node_id"], "post");
    assert_eq!(null_resolutions[0]["location"], "args.channel");
    assert_eq!(
        null_resolutions[0]["expression"],
        "=nodes.summarize.item.json.channel"
    );
    assert!(
        parsed["message"]
            .as_str()
            .unwrap()
            .to_lowercase()
            .contains("output_parser"),
        "{parsed}"
    );
}

#[tokio::test]
async fn dry_run_flags_composio_upstream_binding_as_unverifiable_not_a_wiring_bug() {
    // WS6: `post`'s `body` binds to the OUTPUT of an upstream Composio
    // `tool_call` (`get_me`). The echo sandbox renders `get_me` as
    // `{tool, args, connection}` and can NEVER produce `.item.json.data.username`,
    // so the binding resolves `null` here even when it's wired correctly. The
    // dry run still fails (`ok: false` — a null could hide a typo), but the
    // diagnostic must be HONEST: mark it `unverifiable` and point at
    // get_tool_contract / get_tool_output_sample rather than telling the agent
    // its (possibly-correct) wiring is broken — the exact false negative that
    // sent the transcript agent re-wiring an already-correct binding 3 times.
    // Seed bespoke toolkits (no other test touches `ws6up`/`ws6dl`) with NO
    // required args, so the required-arg preflight passes and the run settles
    // into the `null_resolutions` path deterministically — independent of the
    // process-global catalog cache other tests seed for gmail/slack/etc.
    seed_live_catalog_cache("ws6up", vec![seeded_ws6_contract("WS6UP_LOOKUP", "ws6up")]);
    seed_live_catalog_cache("ws6dl", vec![seeded_ws6_contract("WS6DL_SEND", "ws6dl")]);
    let tool = DryRunWorkflowTool::new(test_config(&TempDir::new().unwrap()));
    let graph = json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "get_me", "kind": "tool_call", "name": "Who am I",
              "config": { "slug": "WS6UP_LOOKUP", "args": {} } },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "WS6DL_SEND",
                "args": { "recipient_email": "a@b.com", "subject": "hi",
                  "body": "=nodes.get_me.item.json.data.username" } } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "get_me" },
            { "from_node": "get_me", "to_node": "post" }
        ]
    });
    let result = tool.execute(json!({ "graph": graph })).await.unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(parsed["ok"], false, "{parsed}");
    let null_resolutions = parsed["null_resolutions"]
        .as_array()
        .expect("null_resolutions array");
    let entry = null_resolutions
        .iter()
        .find(|e| e["node_id"] == "post" && e["location"] == "args.body")
        .unwrap_or_else(|| panic!("expected a post.body null resolution: {parsed}"));
    assert_eq!(entry["unverifiable"], true, "{parsed}");
    assert_eq!(entry["upstream_tool_call"], "get_me", "{parsed}");
    let suggestion = entry["suggestion"].as_str().expect("suggestion string");
    assert!(suggestion.contains("UNVERIFIABLE"), "{suggestion}");
    assert!(suggestion.contains("get_tool_contract"), "{suggestion}");
    assert!(
        suggestion.contains("get_tool_output_sample"),
        "{suggestion}"
    );
}

#[tokio::test]
async fn dry_run_keeps_generic_null_text_for_a_non_tool_call_upstream_binding() {
    // WS6 contrast: `post`'s arg binds to a `transform` node's output (whose
    // real output the echo sandbox DOES produce), and the transform never sets
    // the referenced field, so the null IS a genuine wiring bug. This entry must
    // stay the plain `{ node_id, location, expression }` shape — no
    // `unverifiable` flag — so the honest-uncertainty treatment doesn't leak
    // onto real mistakes.
    seed_live_catalog_cache("ws6dl", vec![seeded_ws6_contract("WS6DL_SEND", "ws6dl")]);
    let tool = DryRunWorkflowTool::new(test_config(&TempDir::new().unwrap()));
    let graph = json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "build", "kind": "transform", "name": "Build",
              "config": { "set": { "unrelated": "x" } } },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "WS6DL_SEND",
                "args": { "recipient_email": "a@b.com", "subject": "hi",
                  "body": "=nodes.build.item.json.missing" } } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "build" },
            { "from_node": "build", "to_node": "post" }
        ]
    });
    let result = tool.execute(json!({ "graph": graph })).await.unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(parsed["ok"], false, "{parsed}");
    let entry = parsed["null_resolutions"]
        .as_array()
        .expect("null_resolutions array")
        .iter()
        .find(|e| e["node_id"] == "post" && e["location"] == "args.body")
        .unwrap_or_else(|| panic!("expected a post.body null resolution: {parsed}"));
    assert!(
        entry.get("unverifiable").is_none(),
        "a non-tool_call upstream must keep the generic diagnostic: {parsed}"
    );
    assert!(
        entry.get("suggestion").is_none(),
        "generic entry carries no unverifiable suggestion: {parsed}"
    );
}

#[tokio::test]
async fn dry_run_passes_when_agent_schema_matches_tool_call_binding() {
    // The FALSE-POSITIVE-PREVENTION case: `summarize` DOES declare a schema
    // covering `channel`, and `post` binds exactly that field. Without the
    // schema-aware mock agent (i.e. with the vendored `MockAgentRunner`, which
    // always echoes `{ agent, request, connection }` regardless of schema)
    // this would incorrectly fail — proving the mock is what makes the check
    // accurate rather than perpetually red for correctly-built graphs.
    let tool = DryRunWorkflowTool::new(test_config(&TempDir::new().unwrap()));
    let graph = json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "summarize", "kind": "agent", "name": "Summarize",
              "config": { "agent_ref": "researcher", "prompt": "summarize",
                "output_parser": { "schema": { "type": "object",
                    "required": ["channel"],
                    "properties": { "channel": { "type": "string" } } } } } },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "oh:noop",
                "args": { "channel": "=nodes.summarize.item.json.channel" } } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "summarize" },
            { "from_node": "summarize", "to_node": "post" }
        ]
    });

    let result = tool.execute(json!({ "graph": graph })).await.unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(
        parsed["ok"], true,
        "schema-aware mock must satisfy the declared schema: {parsed}"
    );
    assert!(
        parsed["null_resolutions"].as_array().unwrap().is_empty(),
        "{parsed}"
    );
}

#[tokio::test]
async fn dry_run_passes_when_tool_call_binds_to_upstream_tool_output() {
    // A `tool_call` binding to another `tool_call`'s real output (not an
    // agent at all) must not be affected by the agent-schema machinery above.
    let tool = DryRunWorkflowTool::new(test_config(&TempDir::new().unwrap()));
    let graph = json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "lookup", "kind": "tool_call", "name": "Lookup",
              "config": { "slug": "oh:lookup", "args": {} } },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "oh:noop",
                "args": { "channel": "=nodes.lookup.item.json.tool" } } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "lookup" },
            { "from_node": "lookup", "to_node": "post" }
        ]
    });

    let result = tool.execute(json!({ "graph": graph })).await.unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(parsed["ok"], true, "{parsed}");
    assert!(
        parsed["null_resolutions"].as_array().unwrap().is_empty(),
        "{parsed}"
    );
}

#[tokio::test]
async fn dry_run_flags_tool_call_error_when_on_error_is_route() {
    // `on_error: "route"` converts the preflight failure into a routed error
    // ITEM so the SANDBOX RUN as a whole still completes (`Ok(outcome)`) —
    // exactly the case the naive `null_resolutions`-only check would miss,
    // because the failing node's diagnostics stay empty (the engine never
    // got far enough to trace an `=`-expression before the preflight error).
    // Seed the same schema as `dry_run_catches_unwired_required_composio_arg`
    // (process-global cache; keep the arg list identical across tests).
    //
    // The graph must give `post`'s `error` port a real destination: vendored
    // tinyflows' author-time `validate()` (added alongside per-node error
    // handling — a graph with `on_error: "route"` but no outgoing `error`-port
    // edge is now rejected up front, since a route with nowhere to go is
    // always a dead-end) would otherwise reject this graph before the sandbox
    // run ever starts, which is a different failure mode than the one this
    // test targets. `recover` is a no-op sink, same convention as
    // `dry_run_passes_when_tool_call_binds_to_upstream_tool_output` above.
    seed_live_catalog_cache("gmail", vec![seeded_gmail_send_contract()]);

    let tool = DryRunWorkflowTool::new(test_config(&TempDir::new().unwrap()));
    let graph = json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "post", "kind": "tool_call", "name": "Send email",
              "config": { "slug": "GMAIL_SEND_EMAIL", "on_error": "route",
                "args": { "to": "=item.email", "body": "hello" } } },
            { "id": "recover", "kind": "tool_call", "name": "Recover",
              "config": { "slug": "oh:noop", "args": {} } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "post" },
            { "from_node": "post", "from_port": "error", "to_node": "recover" }
        ]
    });

    // `to` misses (trigger input has no `email`) — a real run would fail the
    // preflight; `on_error: "route"` must not let that slip through as `ok: true`.
    let result = tool
        .execute(json!({ "graph": graph, "input": {} }))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(
        parsed["ok"], false,
        "on_error: route must not mask a real tool_call failure: {parsed}"
    );
    let node_errors = parsed["node_errors"].as_array().expect("node_errors array");
    assert_eq!(node_errors.len(), 1, "{parsed}");
    assert_eq!(node_errors[0]["node_id"], "post");
    assert!(
        node_errors[0]["error"].as_str().unwrap().contains("to"),
        "error must name the missing field: {parsed}"
    );
}

#[tokio::test]
async fn dry_run_flags_tool_call_error_when_on_error_is_continue() {
    // Same case as above, but `on_error: "continue"` — the other policy that
    // converts a node failure into routed data instead of failing the run.
    seed_live_catalog_cache("gmail", vec![seeded_gmail_send_contract()]);

    let tool = DryRunWorkflowTool::new(test_config(&TempDir::new().unwrap()));
    let graph = json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "post", "kind": "tool_call", "name": "Send email",
              "config": { "slug": "GMAIL_SEND_EMAIL", "on_error": "continue",
                "args": { "to": "=item.email", "body": "hello" } } }
        ],
        "edges": [ { "from_node": "t", "to_node": "post" } ]
    });

    let result = tool
        .execute(json!({ "graph": graph, "input": {} }))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(
        parsed["ok"], false,
        "on_error: continue must not mask a real tool_call failure: {parsed}"
    );
    assert_eq!(
        parsed["node_errors"].as_array().unwrap().len(),
        1,
        "{parsed}"
    );
}

#[tokio::test]
async fn dry_run_passes_when_agent_enum_schema_binds_to_tool_call() {
    // The agent declares an `enum`-constrained field; the schema-aware mock
    // must synthesize an ALLOWED value (not a generic `""` placeholder, which
    // would fail the vendored validator's `enum` check) so a correctly-built
    // graph using an enum schema dry-runs green instead of false-positiving.
    let tool = DryRunWorkflowTool::new(test_config(&TempDir::new().unwrap()));
    let graph = json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "triage", "kind": "agent", "name": "Triage",
              "config": { "agent_ref": "researcher", "prompt": "triage this",
                "output_parser": { "schema": { "type": "object",
                    "required": ["priority"],
                    "properties": {
                        "priority": { "type": "string", "enum": ["urgent", "normal"] }
                    } } } } },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "oh:noop",
                "args": { "priority": "=nodes.triage.item.json.priority" } } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "triage" },
            { "from_node": "triage", "to_node": "post" }
        ]
    });

    let result = tool.execute(json!({ "graph": graph })).await.unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(
        parsed["ok"], true,
        "enum-schema agent must dry-run green: {parsed}"
    );
    assert!(parsed["null_resolutions"].as_array().unwrap().is_empty());
    assert!(parsed["node_errors"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn dry_run_flags_null_resolved_agent_prompt() {
    // The exact root-cause bug PR A/B/C exist to catch: `prompt` itself is a
    // `=`-expression that reads as prose, not a valid jq program — the
    // vendored engine's own `resolve_traced` records it as a null resolution
    // at `location: "prompt"`, meaning the agent would run with an EMPTY
    // prompt. Unlike other agent-config nulls, this one must fail the dry run.
    let tool = DryRunWorkflowTool::new(test_config(&TempDir::new().unwrap()));
    let graph = json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "classify", "kind": "agent", "name": "Classify",
              "config": { "prompt": "=You are given an email: .item. Classify the following \
                  email as urgent/normal/low priority." } }
        ],
        "edges": [ { "from_node": "t", "to_node": "classify" } ]
    });

    let result = tool.execute(json!({ "graph": graph })).await.unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(
        parsed["ok"], false,
        "a null-resolved agent prompt must fail the dry run: {parsed}"
    );
    let agent_prompt_nulls = parsed["agent_prompt_nulls"]
        .as_array()
        .expect("agent_prompt_nulls array");
    assert_eq!(agent_prompt_nulls.len(), 1, "{parsed}");
    assert_eq!(agent_prompt_nulls[0]["node_id"], "classify");
    assert_eq!(agent_prompt_nulls[0]["location"], "prompt");
    assert!(
        agent_prompt_nulls[0]["suggestion"]
            .as_str()
            .unwrap()
            .contains("input_context"),
        "{parsed}"
    );
    assert!(
        parsed["message"]
            .as_str()
            .unwrap()
            .to_lowercase()
            .contains("input_context"),
        "{parsed}"
    );
}
