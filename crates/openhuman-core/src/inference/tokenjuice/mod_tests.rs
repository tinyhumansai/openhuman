use super::*;
use serde_json::json;
use tinytools::Tool;

#[test]
fn host_tools_keep_the_published_recovery_names_and_schemas() {
    // Pin the actual tool the registry constructs before removing the local
    // forwarding wrapper; this protects the model-facing contract byte for
    // byte across the ownership move to tinyjuice-host.
    let retrieve =
        tinyjuice_host::retrieve_tool::RetrieveToolOutputTool::new(std::sync::Arc::new(|_, _| {
            Box::pin(async { Ok(None) })
        }));
    assert_eq!(retrieve.name(), "juice_retrieve");
    assert_eq!(
        retrieve.permission_level(),
        tinytools::PermissionLevel::ReadOnly
    );
    assert!(!retrieve.external_effect());
    assert_eq!(
        serde_json::to_vec(&retrieve.parameters_schema()).unwrap(),
        serde_json::to_vec(&json!({
            "type": "object",
            "properties": {
                "token": {
                    "type": "string",
                    "description": "The token named in the output's footer (or a ⟦tj:…⟧ marker's hash)."
                },
                "range": {
                    "type": "object",
                    "description": "Optional slice of the original to return.",
                    "properties": {
                        "start": { "type": "integer", "minimum": 0 },
                        "end": { "type": "integer", "minimum": 0 },
                        "unit": { "type": "string", "enum": ["bytes", "lines"] }
                    },
                    "required": ["start", "end"]
                }
            },
            "required": ["token"]
        }))
        .unwrap()
    );

    let expected = [
        (
            "juice_find",
            json!({
                "type": "object",
                "properties": {
                    "handle": { "type": "string", "description": "the handle named in the output's footer" },
                    "query": { "type": "string" },
                    "mode": { "type": "string", "enum": ["text", "grep", "regex", "rank", "sed", "awk", "jq"] },
                    "ignore_case": { "type": "boolean" },
                    "context": { "type": "integer" },
                    "top_k": { "type": "integer" },
                    "scope": { "type": "string" },
                    "unit": { "type": "string", "enum": ["lines", "chars"] }
                },
                "required": ["handle", "query"]
            }),
        ),
        (
            "juice_extract",
            json!({
                "type": "object",
                "properties": {
                    "handle": { "type": "string", "description": "the handle named in the output's footer" },
                    "what": { "type": "string", "enum": ["links", "headings"] },
                    "scope": { "type": "string" },
                    "unit": { "type": "string", "enum": ["lines", "chars"] }
                },
                "required": ["handle", "what"]
            }),
        ),
        (
            "juice_summarize",
            json!({
                "type": "object",
                "properties": {
                    "handle": { "type": "string", "description": "the handle named in the output's footer" },
                    "hint": { "type": "string", "description": "what you need from it" },
                    "max_chars": { "type": "integer" },
                    "scope": { "type": "string" },
                    "unit": { "type": "string", "enum": ["lines", "chars"] }
                },
                "required": ["handle"]
            }),
        ),
    ];
    let tools = repl_tools();
    assert_eq!(
        tools.iter().map(|tool| tool.name()).collect::<Vec<_>>(),
        expected.iter().map(|(name, _)| *name).collect::<Vec<_>>()
    );
    for (tool, (name, schema)) in tools.iter().zip(expected) {
        assert_eq!(tool.name(), name);
        let actual = serde_json::to_vec(&tool.parameters_schema()).unwrap();
        let expected = serde_json::to_vec(&schema).unwrap();
        assert_eq!(
            actual,
            expected,
            "schema changed for {name}: actual={}, expected={}",
            String::from_utf8_lossy(&actual),
            String::from_utf8_lossy(&expected),
        );
    }
}

#[test]
fn recovery_tool_aliases_remain_stable() {
    assert!(is_recovery_tool(tinyjuice_bus::tools::RETRIEVE));
    assert!(is_recovery_tool(tinyjuice_bus::tools::LEGACY_RETRIEVE));
    assert!(is_recovery_tool("retrieve_tool_output"));
    assert!(!is_recovery_tool("shell"));
}

/// Exercise the configured on-demand mode through the real module bus. Even
/// with a registered model callback and a result above the summary threshold,
/// ingest must return a recovery handle without calling the model.
/// Runs where a tinyjuice library is provided (`scripts/modules/build.mjs`
/// or the staged release on `OPENHUMAN_MODULE_PATH`).
#[tokio::test]
async fn the_module_defers_a_summary_until_requested() {
    if crate::modules::local_artifact("tinyjuice").is_none() {
        eprintln!(
            "SKIPPED (not run, not asserted): no tinyjuice library. Run \
             `node scripts/modules/build.mjs tinyjuice` and set OPENHUMAN_MODULE_SOURCE_DIR \
             and OPENHUMAN_MODULE_SOURCE=source, or use scripts/test-rust-with-mock.sh"
        );
        return;
    }
    let seen = std::sync::Arc::new(std::sync::Mutex::new(
        None::<tinyjuice_bus::wire::GenerateRequest>,
    ));
    let sink = seen.clone();
    let ticket = generate::register(Box::new(
        move |request: tinyjuice_bus::wire::GenerateRequest| {
            *sink.lock().unwrap() = Some(request);
            Box::pin(async { Ok("the rate limit is 60 requests a minute".to_string()) })
        },
    ));
    // Over the default 4000-token threshold the test config installs.
    let content = "Rate limiting. Requests are limited per key. ".repeat(500);

    let output = compact_tool_output(ToolOutputCompaction {
        content: content.clone(),
        tool_name: "web_fetch",
        enabled: true,
        profile: AgentTokenjuiceCompression::Full,
        runtime_config: None,
        arguments: None,
        focus: Some("the rate limits".into()),
        context_token: Some(ticket.token().to_string()),
        scope: Some("module-summary-test".into()),
    })
    .await;

    assert_eq!(output.summarized_from_bytes, None);
    assert!(!output
        .text
        .contains("the rate limit is 60 requests a minute"));
    assert!(
        output.text.contains(tinyjuice_bus::tools::RETRIEVE),
        "the original stays retrievable: {}",
        output.text
    );
    assert!(
        seen.lock().unwrap().is_none(),
        "ingest called the summary model"
    );
}

/// A result that was registered for a summary and never reached the module
/// still says it is unsummarized, exactly as a module-side failure would.
#[tokio::test]
async fn an_unreachable_module_discloses_a_wanted_summary() {
    if crate::modules::local_artifact("tinyjuice").is_some() {
        return; // A fixture forces the module on; this pins the host path.
    }
    let mut config = crate::config::Config::default();
    config.modules.enabled = false;
    let config = std::sync::Arc::new(config);
    let call = |context_token: Option<String>| ToolOutputCompaction {
        content: "raw".to_string(),
        tool_name: "web_fetch",
        enabled: false,
        profile: AgentTokenjuiceCompression::Light,
        runtime_config: Some(&config),
        arguments: None,
        focus: Some("the pricing".to_string()),
        context_token,
        scope: None,
    };

    let wanted = compact_tool_output(call(Some("token".to_string()))).await;
    assert_eq!(wanted.text, "raw");
    assert_eq!(wanted.notice, Some(summary_failed_notice()));

    let unwanted = compact_tool_output(ToolOutputCompaction {
        enabled: true,
        ..call(None)
    })
    .await;
    assert_eq!(unwanted.notice, None);
}

/// An agent whose profile bypasses TinyJuice's router entirely still
/// discloses a summary that was registered anyway (openhuman#6581 review):
/// the early `Off` return must not silently drop the notice the way an
/// ordinary `unchanged` return would.
#[tokio::test]
async fn an_off_profile_still_discloses_a_registered_summary() {
    let output = compact_tool_output(ToolOutputCompaction {
        content: "raw".to_string(),
        tool_name: "web_fetch",
        enabled: true,
        profile: AgentTokenjuiceCompression::Off,
        runtime_config: None,
        arguments: None,
        focus: None,
        context_token: Some("token".to_string()),
        scope: None,
    })
    .await;
    assert_eq!(output.text, "raw");
    assert_eq!(output.notice, Some(summary_failed_notice()));

    let silent = compact_tool_output(ToolOutputCompaction {
        content: "raw".to_string(),
        tool_name: "web_fetch",
        enabled: true,
        profile: AgentTokenjuiceCompression::Off,
        runtime_config: None,
        arguments: None,
        focus: None,
        context_token: None,
        scope: None,
    })
    .await;
    assert_eq!(silent.text, "raw");
    assert_eq!(silent.notice, None);
}

/// A module disabled in configuration cannot install, even when the process
/// also carries a tinyjuice library: the config check runs
/// first. Exercises the config-driven passthrough independent of whatever
/// module happens to be installed for other tests in this binary.
#[tokio::test]
async fn a_module_disabled_in_configuration_discloses_a_wanted_summary() {
    let mut config = crate::config::Config::default();
    config.modules.enabled = false;
    let fixture_aware_call = client(&config)
        .call::<()>("tinyjuice", methods::INSTALL, (install_request(&config),))
        .await;
    assert!(
        fixture_aware_call.is_err(),
        "a test module artifact must not override modules.enabled=false"
    );

    let config = std::sync::Arc::new(config);

    let output = compact_tool_output(ToolOutputCompaction {
        content: "raw".to_string(),
        tool_name: "web_fetch",
        enabled: true,
        profile: AgentTokenjuiceCompression::Light,
        runtime_config: Some(&config),
        arguments: None,
        focus: None,
        context_token: Some("token".to_string()),
        scope: None,
    })
    .await;

    assert_eq!(output.text, "raw");
    assert_eq!(
        output.notice,
        Some(summary_failed_notice()),
        "installation failure must disclose the unsummarized notice"
    );
}

#[test]
fn passthrough_discloses_a_notice_only_when_a_summary_was_wanted() {
    let silent = CompactedToolOutput::passthrough("raw".to_string(), false);
    assert_eq!(silent.text, "raw");
    assert_eq!(silent.notice, None);
    assert_eq!(silent.summarized_from_bytes, None);

    let disclosed = CompactedToolOutput::passthrough("raw".to_string(), true);
    assert_eq!(disclosed.text, "raw");
    assert_eq!(disclosed.notice, Some(summary_failed_notice()));
    assert_eq!(disclosed.summarized_from_bytes, None);
}

#[test]
fn install_request_turns_on_the_handle_preview_by_default() {
    let config = crate::config::Config::default();
    assert!(repl_handle_active(&config));
    let request = install_request(&config);
    assert!(request.options.router_enabled);
    assert!(request.options.ccr_enabled);
    assert!(request.options.repl_handle);
    assert!(!request.ccr_durable);
}

#[test]
fn install_request_handle_mode_follows_every_switch_it_needs() {
    for flip in [
        (|c: &mut crate::config::Config| c.context.compaction_enabled = false)
            as fn(&mut crate::config::Config),
        |c| c.tokenjuice.router_enabled = false,
        |c| c.tokenjuice.ccr_enabled = false,
        |c| c.tokenjuice.repl_handle_enabled = false,
    ] {
        let mut config = crate::config::Config::default();
        flip(&mut config);
        let request = install_request(&config);
        assert!(!request.options.repl_handle);
        assert!(!request.ccr_durable);
    }
}

#[test]
fn install_request_enables_durable_ccr_when_configured() {
    let mut config = crate::config::Config::default();
    config.workspace_dir = std::path::PathBuf::from("/ws");
    config.tokenjuice.ccr_disk_enabled = true;
    let request = install_request(&config);
    assert!(request.ccr_durable);
}

#[test]
fn install_request_summarizes_only_on_request_under_a_64k_cap() {
    let config = crate::config::Config::default();
    let request = install_request(&config);
    assert!(request.options.llm_summary_enabled);
    assert_eq!(
        request.options.llm_summary_mode,
        tinyjuice_bus::types::LlmSummaryMode::OnDemand,
        "ingest must never call the summary model on its own"
    );
    assert_eq!(request.options.llm_summary_max_input_tokens, 64_000);
}

/// Run against the verified release with the host's pinned TinyBus loader.
#[tokio::test]
async fn released_queries_keep_cached_originals_in_the_module_and_accept_artifact_content() {
    if crate::modules::local_artifact("tinyjuice").is_none() {
        eprintln!("SKIPPED: no TinyJuice library provided");
        return;
    }
    use tinyjuice_bus::{
        repl::{FindMode, ReplLimits, ReplOp, ReplOutput, ScopeUnit},
        wire::{QueryError, QueryRequest, QueryResponse, QueryTarget},
    };
    let workspace = tempfile::tempdir().unwrap();
    let config = crate::config::Config {
        workspace_dir: workspace.path().into(),
        ..Default::default()
    };
    install_from_config(&config).await.unwrap();
    let content = format!("{}\nmodule-owned-needle\n", "boring row\n".repeat(2000));
    let compressed: tinyjuice_bus::types::CompressedOutput = client(&config)
        .call(
            "tinyjuice",
            methods::COMPRESS,
            (
                content.clone(),
                tinyjuice_bus::types::ContentHint::default(),
            ),
        )
        .await
        .unwrap();
    let handle = compressed
        .ccr_token
        .expect("released module retains original");
    let make_request = |target| QueryRequest {
        target,
        op: ReplOp::Find {
            query: "module-owned-needle".into(),
            mode: FindMode::Text,
            ignore_case: false,
            context: 0,
            top_k: None,
            scope: None,
            unit: ScopeUnit::Lines,
        },
        limits: ReplLimits::default(),
        context_token: None,
        scope: None,
    };
    for target in [
        QueryTarget::Handle { token: handle },
        QueryTarget::Content { content },
    ] {
        let reply: QueryResponse = client(&config)
            .call("tinyjuice", methods::QUERY, (make_request(target),))
            .await
            .unwrap();
        assert!(
            matches!(reply, Ok(ReplOutput::Lines {hits, ..}) if hits.iter().any(|hit| hit.text.contains("module-owned-needle")))
        );
    }
    let missing: QueryResponse = client(&config)
        .call(
            "tinyjuice",
            methods::QUERY,
            (make_request(QueryTarget::Handle {
                token: "neverstored123".into(),
            }),),
        )
        .await
        .unwrap();
    assert_eq!(missing, Err(QueryError::HandleNotFound));
    let reply: tinyjuice_bus::wire::HtmlResponse = client(&config).call("tinyjuice", methods::EXTRACT_HTML, ("<html><body><h1>Fixture</h1><script>secret_script</script><p>hello</p></body></html>",)).await.unwrap();
    let markdown = reply.unwrap();
    assert!(markdown.contains("hello"));
    assert!(!markdown.contains("secret_script"));
}

// ── [components] juice ────────────────────────────────────────────────

fn juice_off() -> crate::config::Config {
    let mut config = crate::config::Config::default();
    config.components.juice = false;
    config
}

#[test]
fn juice_off_drops_handles_summaries_and_companion_tools() {
    let on = crate::config::Config::default();
    assert!(enabled(&on));
    assert!(repl_handle_active(&on));
    assert!(!companion_tool_names("orchestrator", &on).is_empty());

    let off = juice_off();
    assert!(!enabled(&off));
    assert!(!repl_handle_active(&off));
    assert!(!summarizes_tool_output("orchestrator", &off));
    assert!(companion_tool_names("orchestrator", &off).is_empty());
}

// The module listing only exists with the `modules` feature.
#[cfg(feature = "modules")]
#[tokio::test]
async fn juice_off_install_never_touches_the_module() {
    let off = juice_off();
    install_from_config(&off).await.expect("no-op");
    // The resolution slot is process-global and other tests legitimately load
    // tinyjuice into it, so its absolute state (`Available`) cannot be asserted
    // here. The listing for a juice-off config reports `Disabled` with no
    // source whatever the slot holds, which is what "never touches the module"
    // means to a caller.
    let statuses = crate::modules::ops::list(&off);
    let status = statuses.iter().find(|s| s.id == "tinyjuice").unwrap();
    assert_eq!(status.state, crate::modules::types::ModuleState::Disabled);
    assert!(status.source.is_none());
}
