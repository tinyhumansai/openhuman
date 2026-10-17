use super::*;
use crate::flows::tinyflows::caps::OpenHumanCode;

// ── build_capabilities smoke ────────────────────────────────────────────

#[test]
fn build_capabilities_constructs_every_slot_without_panicking() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let caps = build_capabilities(
        &crate::core::runtime::DispatchCtx::ambient(),
        config,
        "test:build",
        None,
    );
    assert!(caps.tasks.is_some(), "Tinyflows task nodes need a runner");
    assert!(
        caps.approvals.is_none(),
        "OpenHuman resumes approvals through its existing flow-run surface"
    );
}

#[tokio::test]
async fn http_adapter_blocks_loopback_host_as_capability_error() {
    let adapter = http_adapter(vec![]); // open allowlist mode
    let err = adapter
        .request(
            json!({ "method": "GET", "url": "http://127.0.0.1:1/" }),
            None,
        )
        .await
        .expect_err("loopback host must be blocked by the SSRF guard");
    let msg = err.to_string();
    assert!(
        msg.to_lowercase().contains("private") || msg.to_lowercase().contains("local"),
        "expected an SSRF-guard message, got: {msg}"
    );
}

#[tokio::test]
async fn http_adapter_rejects_host_outside_strict_allowlist() {
    let adapter = http_adapter(vec!["example.com".to_string()]);
    let err = adapter
        .request(
            json!({ "method": "GET", "url": "https://not-allowed.test/" }),
            None,
        )
        .await
        .expect_err("host outside the strict allowlist must be rejected");
    assert!(
        err.to_string().contains("not-allowed.test")
            || err.to_string().to_lowercase().contains("allowed"),
        "expected an allowlist rejection message, got: {err}"
    );
}

// ── Engine smoke: real seam end to end ───────────────────────────────────

#[tokio::test]
async fn engine_run_drives_trigger_to_http_request_through_the_real_seam() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let caps = build_capabilities(
        &crate::core::runtime::DispatchCtx::ambient(),
        config,
        "test:smoke",
        None,
    );

    // A deterministically-blocked loopback URL with `on_error: continue` so
    // the run completes even though the (real, SSRF-guarded) HTTP adapter
    // necessarily rejects it — see the module doc for why a real network
    // round-trip isn't testable here.
    let graph = WorkflowGraph {
        nodes: vec![
            node("t", NodeKind::Trigger, serde_json::Value::Null),
            node(
                "http",
                NodeKind::HttpRequest,
                json!({ "method": "GET", "url": "http://127.0.0.1:1/", "on_error": "continue" }),
            ),
        ],
        edges: vec![edge("t", "http")],
        ..Default::default()
    };
    let compiled = tinyflows::compiler::compile(&graph).expect("compile");

    let outcome = tinyflows::engine::run(&compiled, json!({ "seed": 1 }), &caps)
        .await
        .expect("run should complete (on_error: continue)");

    assert!(outcome.pending_approvals.is_empty());
    assert_eq!(
        outcome.output["nodes"]["http"]["items"][0]["json"]["error"]["node"],
        json!("http")
    );
}

// ── Code adapter ──────────────────────────────────────────────────────────

/// Requires `node` on `PATH` (the code node runs the host's Node under the
/// sandbox). TinyBox's native backend only provides the required isolation on
/// Linux; macOS Seatbelt and other fallback backends deliberately do not meet
/// the untrusted-code floor.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn code_adapter_javascript_runs_when_isolated_and_refuses_when_isolation_is_unavailable() {
    if std::process::Command::new("node")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("SKIPPED (not run, not asserted): no `node` binary on PATH");
        return;
    }
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let security = Arc::new(SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
        &config.action_dir,
    ));
    let runner = OpenHumanCode {
        ctx: crate::core::runtime::DispatchCtx::ambient(),
        config,
        security,
        approval: Default::default(),
    };

    let input = json!([{ "json": { "n": 7 } }]);
    match runner
        .run(CodeLanguage::JavaScript, "return input;", input.clone())
        .await
    {
        Ok(result) => assert_eq!(result, input),
        Err(error) => assert!(
            error
                .to_string()
                .contains("TinyBox native jail cannot meet the required isolation floor"),
            "unexpected JavaScript code-runner failure: {error}"
        ),
    }
}

#[tokio::test]
async fn tools_invoke_rejects_a_non_curated_slug_for_a_known_toolkit() {
    let tmp = TempDir::new().unwrap();
    let tools = tools_adapter(test_config(&tmp));

    // "gmail" has a curated catalog; this action is not in it, so curation
    // must reject regardless of the user's read/write/admin scope prefs.
    let err = tools
        .invoke("GMAIL_NOT_A_REAL_CURATED_ACTION", json!({}), None)
        .await
        .expect_err("a non-curated action for a curated toolkit must be rejected");
    let msg = err.to_string();
    assert!(
        msg.contains("tool not permitted"),
        "expected a curation rejection message, got: {msg}"
    );
    assert!(msg.contains("GMAIL_NOT_A_REAL_CURATED_ACTION"));
}

#[tokio::test]
async fn tools_invoke_rejects_an_unrecognized_toolkit_slug() {
    // Issue B2 finding #2 (deny-by-default): a made-up toolkit prefix that
    // isn't in any curated catalog must be rejected — not passed through on
    // a permissive "unknown toolkit" heuristic. Live testing confirmed this
    // used to reach Composio (and only failed there for lack of a signed-in
    // session), which is not a hard allowlist.
    let tmp = TempDir::new().unwrap();
    let tools = tools_adapter(test_config(&tmp));

    let err = tools
        .invoke("madeupkit_dostuff", json!({}), None)
        .await
        .expect_err("an unrecognized toolkit slug must be rejected by curation");
    let msg = err.to_string();
    assert!(
        msg.contains("tool not permitted"),
        "expected a curation rejection message, got: {msg}"
    );
    assert!(msg.contains("madeupkit_dostuff"));
}

#[tokio::test]
async fn tools_invoke_rejects_a_prefix_less_slug() {
    // "noop" has no curated catalog (`catalog_for_toolkit` returns `None`
    // for the single-segment "toolkit" `toolkit_from_slug` degrades it to),
    // so the hard allowlist in `is_curated_flow_tool` rejects it outright —
    // unlike the general agent tool-call path's `is_action_visible_with_pref`,
    // which falls back to the permissive `classify_unknown` heuristic and
    // would let this slug through.
    let tmp = TempDir::new().unwrap();
    let tools = tools_adapter(test_config(&tmp));

    let err = tools
        .invoke("noop", json!({}), None)
        .await
        .expect_err("a prefix-less/unrecognized slug must be rejected by curation");
    assert!(
        err.to_string().contains("tool not permitted"),
        "expected a curation rejection message, got: {err}"
    );
}

#[tokio::test]
async fn tools_invoke_does_not_reject_a_known_curated_slug_at_the_curation_gate() {
    // A real curated action for a known toolkit must clear the curation
    // gate — it may still fail further downstream (no composio client
    // configured in this test environment), but that failure must NOT be
    // the "tool not permitted" curation-rejection message.
    let tmp = TempDir::new().unwrap();
    let tools = tools_adapter(test_config(&tmp));

    let err = tools
        .invoke("GMAIL_SEND_EMAIL", json!({}), None)
        .await
        .expect_err("no composio client is configured in the test environment");
    assert!(
        !err.to_string().contains("tool not permitted"),
        "a known curated slug must not be rejected by curation, got: {err}"
    );
}

#[test]
fn composio_connection_id_parses_toolkit_prefixed_ref() {
    assert_eq!(
        super::super::caps::composio_connection_id("composio:slack:acct_123"),
        Some("acct_123")
    );
    // Trailing segment only — works even without a toolkit segment present.
    assert_eq!(
        super::super::caps::composio_connection_id("composio::acct_1"),
        Some("acct_1")
    );
}

#[test]
fn composio_connection_id_returns_none_for_non_composio_ref_or_empty_id() {
    assert_eq!(
        super::super::caps::composio_connection_id("http_cred:my-secret"),
        None
    );
    assert_eq!(
        super::super::caps::composio_connection_id("composio:"),
        None
    );
    assert_eq!(
        super::super::caps::composio_connection_id("composio:slack:"),
        None
    );
}

#[test]
fn http_cred_name_parses_and_trims() {
    assert_eq!(
        super::super::caps::http_cred_name("http_cred:my-secret"),
        Some("my-secret")
    );
    assert_eq!(
        super::super::caps::http_cred_name("http_cred: spaced "),
        Some("spaced")
    );
}

#[test]
fn http_cred_name_returns_none_for_non_http_cred_ref_or_empty_name() {
    assert_eq!(
        super::super::caps::http_cred_name("composio:slack:acct_1"),
        None
    );
    assert_eq!(super::super::caps::http_cred_name("http_cred:"), None);
}

// ── structured agent output (parse_llm_json) ────────────────────────────

// ── tool_call required-arg preflight ─────────────────────────────────────

#[tokio::test]
async fn preflight_fails_before_dispatch_naming_the_missing_field() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    // Seed the schema cache so no live Composio backend is needed. Uses its
    // own toolkit/slug (never `"gmail"`) — the process-global
    // `LIVE_CATALOG_CACHE` is shared with every `#[tokio::test]` in this
    // file, and `preflight_invoker_gates_the_mock_tool_path` below seeds a
    // DIFFERENT required-args list under the same slug; under parallel
    // execution one test could overwrite the other's cache entry between
    // seed and assert, making both flaky.
    super::super::caps::seed_live_catalog_cache(
        "preflightfailtest",
        vec![seeded_required_args_contract(
            "PREFLIGHTFAILTEST_SEND_EMAIL",
            "preflightfailtest",
            &["to", "subject", "body"],
        )],
    );

    // `to` resolved to null (the classic mis-wired agent → tool_call case).
    let err = super::super::caps::preflight_composio_args(
        &config,
        "PREFLIGHTFAILTEST_SEND_EMAIL",
        &json!({ "to": null, "subject": "hi", "body": "text" }),
    )
    .await
    .expect_err("null required arg must fail preflight");
    let msg = err.to_string();
    assert!(msg.contains("`to`"), "error must name the field: {msg}");
    assert!(
        msg.contains("=nodes.<node_id>.item.json.<field>"),
        "error must suggest the wiring fix: {msg}"
    );
    assert!(
        msg.contains("output schema"),
        "error must mention the agent output schema rule: {msg}"
    );

    // Fully-wired args pass.
    super::super::caps::preflight_composio_args(
        &config,
        "PREFLIGHTFAILTEST_SEND_EMAIL",
        &json!({ "to": "a@b.com", "subject": "hi", "body": "text" }),
    )
    .await
    .expect("wired args must pass preflight");
}

#[tokio::test]
async fn preflight_skips_when_no_schema_is_available() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    // A slug whose toolkit is unknown to the curation catalog has no schema
    // source at all — the preflight must skip, never block.
    super::super::caps::preflight_composio_args(&config, "NOT_A_REAL_TOOLKIT_ACTION", &json!({}))
        .await
        .expect("preflight must be best-effort when no schema is available");
}

#[tokio::test]
async fn preflight_applies_static_arg_rules_without_a_catalog() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    // No catalog seeding for this slug — `GMAIL_ADD_LABEL_TO_EMAIL` is never
    // seeded anywhere, so `composio_required_args` returns `None` whether or
    // not another test has cached the `gmail` toolkit. That used to make the
    // whole preflight inert (#6154): the node sailed through and only failed
    // at dispatch, inside `prepare_execute_arguments`.
    //
    // Those dispatch-time rules are static — no catalog, no network, no key —
    // so the preflight must apply them regardless of catalog availability.
    let err = super::super::caps::preflight_composio_args(
        &config,
        "GMAIL_ADD_LABEL_TO_EMAIL",
        &json!({ "add_label_ids": ["INBOX"] }),
    )
    .await
    .expect_err("a statically-known missing required arg must fail preflight");
    let msg = err.to_string();
    assert!(
        msg.contains("message_id"),
        "error must name the missing field: {msg}"
    );

    // Args that satisfy the static rules still pass (no catalog needed).
    super::super::caps::preflight_composio_args(
        &config,
        "GMAIL_ADD_LABEL_TO_EMAIL",
        &json!({ "message_id": "abc123", "add_label_ids": ["INBOX"] }),
    )
    .await
    .expect("statically-valid args must pass preflight");
}

#[tokio::test]
async fn preflight_invoker_gates_the_mock_tool_path() {
    use tinyflows::caps::ToolInvoker as _;

    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    // Own toolkit/slug (never `"gmail"`) — see the comment in
    // `preflight_fails_before_dispatch_naming_the_missing_field` above for
    // why sharing the `"gmail"` cache key across parallel tests is flaky.
    super::super::caps::seed_live_catalog_cache(
        "preflightgatetest",
        vec![seeded_required_args_contract(
            "PREFLIGHTGATETEST_SEND_EMAIL",
            "preflightgatetest",
            &["to"],
        )],
    );

    let mock = tinyflows::caps::mock::mock_capabilities();
    let invoker = super::super::caps::PreflightToolInvoker {
        config,
        inner: mock.tools.clone(),
    };

    // Unwired required arg: fails with the named field even though the inner
    // mock would echo anything.
    let err = invoker
        .invoke("PREFLIGHTGATETEST_SEND_EMAIL", json!({ "to": null }), None)
        .await
        .expect_err("dry-run preflight must catch the unwired arg");
    assert!(err.to_string().contains("`to`"));

    // Wired arg: delegates to the mock echo.
    let ok = invoker
        .invoke(
            "PREFLIGHTGATETEST_SEND_EMAIL",
            json!({ "to": "a@b.com" }),
            None,
        )
        .await
        .expect("wired arg passes through to the mock");
    assert_eq!(ok["tool"], "PREFLIGHTGATETEST_SEND_EMAIL");

    // Native `oh:` slugs bypass the Composio preflight (no Composio schema).
    // The mock echoes them unchecked.
    let ok = invoker
        .invoke("oh:web_search", json!({}), None)
        .await
        .expect("native slug bypasses composio preflight");
    assert_eq!(ok["tool"], "oh:web_search");
}

#[test]
fn harness_model_default_override_normalises_tiers_to_hint_roles() {
    // Bare managed tiers → the `hint:<role>` form the session builder routes on
    // (a bare tier would otherwise fall through to the chat workload).
    assert_eq!(
        harness_model_default_override("reasoning-v1"),
        "hint:reasoning"
    );
    assert_eq!(harness_model_default_override("chat-v1"), "hint:chat");
    // `hint:*` aliases pass through their role.
    assert_eq!(
        harness_model_default_override("hint:reasoning"),
        "hint:reasoning"
    );
}

#[test]
fn harness_model_default_override_forwards_raw_byok_models_verbatim() {
    // Raw/BYOK ids a user pins on an agent node are forwarded verbatim (issue
    // #4598) — normalising them to `hint:chat` would collapse the explicit
    // per-node model onto the managed chat tier. They reach the harness `chat`
    // role, which inherits `config.default_model`, and `make_openhuman_backend`
    // forwards the non-tier id to the backend unchanged.
    assert_eq!(
        harness_model_default_override("claude-opus-4"),
        "claude-opus-4"
    );
    assert_eq!(
        harness_model_default_override("openai:gpt-4o"),
        "openai:gpt-4o"
    );
    // Empty / whitespace is not a raw id — falls back to the chat workload.
    assert_eq!(harness_model_default_override("   "), "hint:chat");
}

#[test]
fn route_for_agent_ref_selects_harness_for_definitions_else_fallback() {
    // Ensure the global registry is populated (idempotent no-op if another test
    // already initialised it; builtins are always present either way).
    let _ = crate::agent::harness::host_definition::AgentDefinitionRegistry::init_global_builtins();

    // A shipped harness definition → full-loop harness path.
    assert_eq!(route_for_agent_ref("workflow_builder"), AgentRoute::Harness);
    assert_eq!(route_for_agent_ref("planner"), AgentRoute::Harness);

    // An id with no harness definition → the custom-registry completion fallback.
    assert_eq!(
        route_for_agent_ref("totally_unknown_custom_agent_xyz"),
        AgentRoute::RegistryFallback
    );
}

#[test]
fn route_custom_entry_lookup_routes_enabled_custom_entry_to_harness() {
    let entry = custom_registry_entry(true);
    assert_eq!(
        route_custom_entry_lookup(Some(&entry)),
        AgentRoute::Harness,
        "a known, enabled custom-registry agent must run through the harness (real tools), \
         not the persona-only completion fallback"
    );
}

#[test]
fn route_custom_entry_lookup_falls_back_for_disabled_custom_entry() {
    let entry = custom_registry_entry(false);
    assert_eq!(
        route_custom_entry_lookup(Some(&entry)),
        AgentRoute::RegistryFallback,
        "a disabled custom entry must still go through run_via_registry_fallback, which \
         rejects it with a clear \"is disabled\" error"
    );
}

#[test]
fn route_custom_entry_lookup_falls_back_when_no_entry_exists() {
    assert_eq!(
        route_custom_entry_lookup(None),
        AgentRoute::RegistryFallback,
        "an agent_ref unknown to both the harness registry and the custom config registry \
         must still resolve through the fallback (which reports \"unknown agent_ref\")"
    );
}

// ── Handed context ───────────────────────────────────────────────────────

/// An adapter acts for the context its run was started with, never for the
/// scope of whichever task happens to poll it: two runs under two contexts do
/// not cross, even when a third context is ambient.
#[tokio::test]
async fn adapters_act_for_the_context_they_were_handed_not_the_ambient_one() {
    use crate::core::runtime::{context::CoreContext, ComponentSet, DispatchCtx, DomainSet};

    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let security = Arc::new(SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
        &config.action_dir,
    ));
    let sandbox_off = ComponentSet {
        sandbox: false,
        ..ComponentSet::default()
    };
    let handed_off = DispatchCtx::new(CoreContext::for_test_with_components(sandbox_off));
    let handed_on = DispatchCtx::new(CoreContext::for_test_with_components(
        ComponentSet::default(),
    ));
    let ambient = CoreContext::for_test(DomainSet::full(), None);

    let build = |ctx: &DispatchCtx| OpenHumanCode {
        ctx: ctx.clone(),
        config: config.clone(),
        security: security.clone(),
        approval: Default::default(),
    };
    let (off, on) = (build(&handed_off), build(&handed_on));

    CoreContext::scope(ambient, async {
        assert!(!off.ctx.components().sandbox, "the run's own switch wins");
        assert!(on.ctx.components().sandbox, "two runs do not cross");
        assert!(
            DispatchCtx::ambient().components().sandbox,
            "the ambient scope is a third, unrelated context"
        );
    })
    .await;
}
