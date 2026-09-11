use super::*;

#[test]
fn schema_requires_question_and_makes_focus_optional() {
    let tool = AgentPrepareContextTool::new();
    let schema = tool.parameters_schema();
    let required = schema
        .get("required")
        .and_then(|v| v.as_array())
        .expect("schema has required array");
    assert!(required.iter().any(|v| v.as_str() == Some("question")));
    assert!(
        required.iter().all(|v| v.as_str() != Some("focus")),
        "focus must be optional"
    );
    let props = schema.get("properties").expect("schema has properties");
    assert!(props.get("question").is_some());
    assert!(props.get("focus").is_some());
}

#[test]
fn description_skips_when_context_is_already_prepared() {
    let tool = AgentPrepareContextTool::new();
    let description = tool.description();

    assert!(description.contains("agent context has already been prepared"));
    assert!(description.contains("do not call this tool again"));
}

#[test]
fn build_scout_prompt_includes_request_focus_and_catalog() {
    let prompt = AgentPrepareContextTool::build_scout_prompt(
        "summarise my unread gmail",
        Some("last 24h"),
        "- delegate_to_integrations_agent: route to a connected integration\n",
    );
    assert!(prompt.contains("[Request]"));
    assert!(prompt.contains("summarise my unread gmail"));
    assert!(prompt.contains("[Focus]"));
    assert!(prompt.contains("last 24h"));
    assert!(prompt.contains("[Orchestrator tools]"));
    assert!(prompt.contains("delegate_to_integrations_agent"));
    assert!(prompt.contains("[context_bundle]"));
}

#[test]
fn build_scout_prompt_handles_empty_catalog() {
    let prompt = AgentPrepareContextTool::build_scout_prompt("do a thing", None, "");
    assert!(prompt.contains("(none available"));
    assert!(!prompt.contains("[Focus]"));
}

#[test]
fn parse_proposed_goal_extracts_objective_or_none() {
    let bundle = "[context_bundle]\nhas_enough_context: true\n\
                  proposed_goal: Ship the desktop release to all platforms\n\
                  summary: ...\n[/context_bundle]";
    assert_eq!(
        AgentPrepareContextTool::parse_proposed_goal(bundle).as_deref(),
        Some("Ship the desktop release to all platforms")
    );

    // Explicit `none` → no goal.
    let none_bundle = "[context_bundle]\nproposed_goal: none\nsummary: x\n[/context_bundle]";
    assert!(AgentPrepareContextTool::parse_proposed_goal(none_bundle).is_none());

    // Missing line → no goal.
    let no_line = "[context_bundle]\nhas_enough_context: true\n[/context_bundle]";
    assert!(AgentPrepareContextTool::parse_proposed_goal(no_line).is_none());

    // Case-insensitive prefix.
    let cased = "Proposed_Goal:  Land the migration  ";
    assert_eq!(
        AgentPrepareContextTool::parse_proposed_goal(cased).as_deref(),
        Some("Land the migration")
    );

    // Lines starting with a multibyte char must not panic the byte-prefix
    // match (regression for the `l[..14]` non-boundary slice).
    let multibyte = "[context_bundle]\n日本語の要約 summary line\nproposed_goal: 目標を達成する\n[/context_bundle]";
    assert_eq!(
        AgentPrepareContextTool::parse_proposed_goal(multibyte).as_deref(),
        Some("目標を達成する")
    );
}

#[tokio::test]
async fn missing_question_returns_error() {
    let tool = AgentPrepareContextTool::new();
    let result = tool.execute(json!({})).await.unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("question"));
}

#[tokio::test]
async fn execute_short_circuits_when_context_already_prepared() {
    let tool = AgentPrepareContextTool::new();
    let result = crate::openhuman::agent::harness::with_agent_context_prepared_sources(
        vec![AgentContextPreparedSource {
            source: "memory agent context retrieval".to_string(),
            has_enough_context: Some(false),
        }],
        tool.execute(json!({"question": "prepare context again"})),
    )
    .await
    .unwrap();

    assert!(!result.is_error, "{}", result.output());
    assert!(result.output().contains("[context_bundle]"));
    assert!(result.output().contains("has_enough_context: false"));
    assert!(result.output().contains("already been prepared once"));
    assert!(result.output().contains("memory agent context retrieval"));
    assert!(result
        .output()
        .contains("does not assert that enough context is available"));
    assert!(result.output().contains("[/context_bundle]"));
}

#[tokio::test]
async fn execute_preserves_prior_prepared_context_sufficiency_when_true() {
    let tool = AgentPrepareContextTool::new();
    let result = crate::openhuman::agent::harness::with_agent_context_prepared_sources(
        vec![AgentContextPreparedSource {
            source: "memory agent context retrieval".to_string(),
            has_enough_context: Some(true),
        }],
        tool.execute(json!({"question": "prepare context again"})),
    )
    .await
    .unwrap();

    assert!(!result.is_error, "{}", result.output());
    assert!(result.output().contains("has_enough_context: true"));
    assert!(result.output().contains("reported enough context"));
}

#[test]
fn extracts_a_single_well_formed_bundle() {
    let out = "[context_bundle]\nhas_enough_context: true\nsummary: ok\n[/context_bundle]";
    assert_eq!(extract_context_bundle(out).as_deref(), Some(out));
}

#[test]
fn rejects_free_form_prose_without_a_bundle() {
    assert_eq!(
        extract_context_bundle("Sure! Here's what I found about your request..."),
        None
    );
}

#[test]
fn rejects_unterminated_or_reversed_envelope() {
    // Open tag with no close.
    assert_eq!(
        extract_context_bundle("[context_bundle]\nsummary: ..."),
        None
    );
    // Close before open — out of order.
    assert_eq!(
        extract_context_bundle("[/context_bundle] stray [context_bundle]"),
        None
    );
}

#[test]
fn rejects_duplicated_envelope() {
    // Two envelopes — we can't tell which is authoritative, so reject.
    assert_eq!(
        extract_context_bundle(
            "[context_bundle]a[/context_bundle][context_bundle]b[/context_bundle]"
        ),
        None
    );
}

#[test]
fn extracts_envelope_from_surrounding_prose() {
    // Regression for the "scout runs, bundle missing" bug: a fast chat-tier
    // scout wraps the envelope in a preamble and/or a closing line. We must
    // extract just the envelope, not drop it and not inject the prose.
    let leading = "Sure, here's what I found:\n[context_bundle]\nsummary: x\n[/context_bundle]";
    assert_eq!(
        extract_context_bundle(leading).as_deref(),
        Some("[context_bundle]\nsummary: x\n[/context_bundle]")
    );
    let trailing = "[context_bundle]\nsummary: x\n[/context_bundle]\nHope that helps!";
    assert_eq!(
        extract_context_bundle(trailing).as_deref(),
        Some("[context_bundle]\nsummary: x\n[/context_bundle]")
    );
    let both = "Here you go:\n[context_bundle]\nsummary: x\n[/context_bundle]\n\nLet me know!";
    assert_eq!(
        extract_context_bundle(both).as_deref(),
        Some("[context_bundle]\nsummary: x\n[/context_bundle]")
    );
}

#[test]
fn extracts_envelope_with_surrounding_whitespace() {
    // Leading/trailing whitespace is trimmed, not treated as prose.
    assert_eq!(
        extract_context_bundle("\n  [context_bundle]\nsummary: x\n[/context_bundle]\n  ")
            .as_deref(),
        Some("[context_bundle]\nsummary: x\n[/context_bundle]")
    );
}

// ── Credits-exhausted scout failures stay off Sentry (#5308) ────────

/// Verbatim wire body from Sentry TAURI-RUST-HMW's breadcrumb — the
/// managed backend's budget-exhausted 400. Pinning the exact string makes a
/// backend phrasing drift fail CI rather than silently re-flood Sentry.
const CREDITS_400_BODY: &str = "OpenHuman returned HTTP 400: \
    {\"success\":false,\"error\":\"Insufficient budget\",\
    \"errorCode\":\"USER_INSUFFICIENT_CREDITS\"}";

#[test]
fn classifies_managed_credits_400_and_byo_402_as_expected_billing() {
    assert!(
        is_expected_billing_failure(CREDITS_400_BODY),
        "the managed USER_INSUFFICIENT_CREDITS 400 is user-state, not a defect"
    );
    // The BYO sibling: a provider 402 whose body names the credit shortfall.
    assert!(is_expected_billing_failure(
        "openrouter API error (402 Payment Required): {\"error\":\"Insufficient credits\"}"
    ));
}

#[test]
fn real_scout_failures_are_not_classified_as_billing() {
    for message in [
        "provider call failed: connection reset by peer",
        "agent definition 'context_scout' not found in registry",
        "sub-agent exceeded maximum iterations (8)",
        "OpenHuman returned HTTP 400: {\"error\":\"model not found\"}",
        // A healthy-balance readout must not be swallowed.
        "You have 100 remaining credits this month",
        "",
    ] {
        assert!(
            !is_expected_billing_failure(message),
            "{message:?} must keep paging Sentry"
        );
    }
}

#[test]
fn scout_failure_signal_flattens_the_provider_cause_chain() {
    // The wire body arrives as a *cause* under the runner's own context, so
    // `to_string()` alone loses it — the classifier would then miss the very
    // failure the demotion exists for.
    let inner = anyhow::Error::msg(CREDITS_400_BODY).context("chat completion failed");
    let err = SubagentRunError::Provider(inner);

    assert!(
        !err.to_string().contains("USER_INSUFFICIENT_CREDITS"),
        "precondition: the flat Display drops the cause carrying the wire body"
    );
    let signal = scout_failure_signal(&err);
    assert!(signal.contains("USER_INSUFFICIENT_CREDITS"));
    assert!(is_expected_billing_failure(&signal));

    // Non-`Provider` variants are already flat thiserror messages.
    assert_eq!(
        scout_failure_signal(&SubagentRunError::MaxIterationsExceeded(8)),
        "sub-agent exceeded maximum iterations (8)"
    );
}

/// The regression this issue is about: a background `context_scout` run that
/// fails on exhausted credits must NOT become a Sentry error event, while
/// every other failure still does.
///
/// Asserted end-to-end through the real `sentry-tracing` bridge (mirroring
/// `core::logging::sentry_tracing_layer`'s level mapping) rather than by
/// inspecting the log level, because the level→event mapping *is* the
/// mechanism: `ERROR` → `EventFilter::Event`, `WARN` → `Breadcrumb`.
#[cfg(feature = "crash-reporting")]
#[test]
fn credits_exhausted_scout_failure_does_not_reach_sentry() {
    use sentry::test::TestTransport;
    use std::sync::Arc;
    use tracing::Level;
    use tracing_subscriber::layer::SubscriberExt;

    let transport = TestTransport::new();
    let options = sentry::ClientOptions {
        dsn: Some("https://public@sentry.invalid/1".parse().unwrap()),
        transport: Some(Arc::new(transport.clone())),
        ..Default::default()
    };
    let hub = Arc::new(sentry::Hub::new(
        Some(Arc::new(options.into())),
        Arc::new(Default::default()),
    ));
    let _hub_guard = sentry::HubSwitchGuard::new(hub);

    let subscriber = tracing_subscriber::registry().with(
        sentry::integrations::tracing::layer().event_filter(|metadata| match *metadata.level() {
            Level::ERROR => sentry::integrations::tracing::EventFilter::Event,
            Level::WARN | Level::INFO => sentry::integrations::tracing::EventFilter::Breadcrumb,
            _ => sentry::integrations::tracing::EventFilter::Ignore,
        }),
    );
    let _subscriber_guard = tracing::subscriber::set_default(subscriber);

    log_scout_failure("provider call failed", CREDITS_400_BODY);
    assert!(
        transport.fetch_and_clear_events().is_empty(),
        "an out-of-credits background scout must not page Sentry (TAURI-RUST-HMW)"
    );

    log_scout_failure("provider call failed", "connection reset by peer");
    let events = transport.fetch_and_clear_events();
    assert_eq!(
        events.len(),
        1,
        "a genuine scout failure must still reach Sentry"
    );
    assert_eq!(events[0].level, sentry::Level::Error);
}

// ─────────────────────────────────────────────────────────────────────────────
// Parent tool catalogue: rendered from the parent's own advertised surface
// ─────────────────────────────────────────────────────────────────────────────

/// A spec with the given name and description; the schema is irrelevant here.
fn catalog_spec(
    name: &str,
    description: &str,
) -> std::sync::Arc<crate::openhuman::tools::ToolSpec> {
    std::sync::Arc::new(crate::openhuman::tools::ToolSpec {
        name: name.to_string(),
        description: description.to_string(),
        parameters: serde_json::json!({"type": "object"}),
    })
}

/// A parent context whose inheritable registry (`all_tool_specs`) and own
/// advertised surface (`visible_tool_specs`) are chosen by the caller.
/// `visible_tool_names` is derived from the visible specs, as the turn
/// builder derives it.
fn parent_context_with_specs(
    all_tool_specs: Vec<std::sync::Arc<crate::openhuman::tools::ToolSpec>>,
    visible_tool_specs: Vec<std::sync::Arc<crate::openhuman::tools::ToolSpec>>,
) -> crate::openhuman::agent::harness::fork_context::ParentExecutionContext {
    use std::sync::Arc;
    let workspace = tempfile::TempDir::new().expect("temp workspace");
    let workspace_dir = workspace.path().to_path_buf();
    std::mem::forget(workspace);
    // The context needs *a* memory to be constructed with and never reads one
    // back, which is exactly what `noop_memory` is for.
    let memory: Arc<dyn crate::openhuman::memory::Memory> =
        crate::openhuman::memory::test_support::noop_memory();
    let model: Arc<dyn tinyinference::model::ChatModel<()>> =
        Arc::new(tinyagents_harness::testkit::ScriptedModel::new(Vec::new()));
    crate::openhuman::agent::harness::fork_context::ParentExecutionContext {
        workspace_descriptor: None,
        agent_definition_id: "orchestrator".into(),
        allowed_subagent_ids: std::collections::HashSet::new(),
        turn_model_source: crate::openhuman::agent::tinyagents::TurnModelSource::from_model(model),
        all_tools: Arc::new(Vec::new()),
        all_tool_specs: Arc::new(all_tool_specs),
        visible_tool_names: visible_tool_specs
            .iter()
            .map(|spec| spec.name.clone())
            .collect(),
        visible_tool_specs: Arc::new(visible_tool_specs),
        subagent_tool_ceiling_names: std::collections::HashSet::new(),
        model_name: "test-model".into(),
        temperature: 0.0,
        workspace_dir,
        memory,
        agent_config: crate::openhuman::config::AgentConfig::default(),
        workflows: Arc::new(Vec::new()),
        memory_context: Arc::new(None),
        session_id: "parent-session".into(),
        channel: "test".into(),
        connected_integrations: Vec::new(),
        tool_call_format: crate::openhuman::agent::context::prompt::ToolCallFormat::Native,
        session_key: "parent-key".into(),
        session_parent_prefix: None,
        on_progress: None,
        run_queue: None,
    }
}

/// The catalogue describes the parent's own advertised surface, which includes
/// its synthesised `delegate_*` tools — not `all_tool_specs`, which is what a
/// child may inherit and deliberately carries no delegate (#4452). The scout
/// exists to recommend exactly those delegates back to the parent.
#[tokio::test]
async fn catalog_lists_the_parents_synthesised_delegates_from_its_visible_specs() {
    let ctx = parent_context_with_specs(
        vec![
            catalog_spec("echo", "durable"),
            catalog_spec("agent_prepare_context", "this tool"),
        ],
        vec![
            catalog_spec("echo", "durable"),
            catalog_spec(
                "delegate_to_integrations_agent",
                "route to a connected integration",
            ),
            catalog_spec("agent_prepare_context", "this tool"),
        ],
    );
    let catalog = crate::openhuman::agent::harness::fork_context::with_parent_context(ctx, async {
        AgentPrepareContextTool::render_parent_tool_catalog()
    })
    .await;
    assert!(
        catalog.contains("- delegate_to_integrations_agent: route to a connected integration\n"),
        "the parent's delegate must be recommendable: {catalog:?}"
    );
    assert!(catalog.contains("- echo: durable\n"));
    assert!(
        !catalog.contains("agent_prepare_context"),
        "the scout must not recommend another scout pass"
    );
}

/// A context without a visible spec list (background builders, older
/// callers) keeps the previous behaviour: the inheritable registry, filtered
/// by the visible names.
#[tokio::test]
async fn catalog_falls_back_to_all_tool_specs_when_visible_specs_are_absent() {
    let mut ctx = parent_context_with_specs(
        vec![
            catalog_spec("echo", "durable"),
            catalog_spec("hidden", "not advertised"),
        ],
        Vec::new(),
    );
    ctx.visible_tool_names = std::iter::once("echo".to_string()).collect();
    let catalog = crate::openhuman::agent::harness::fork_context::with_parent_context(ctx, async {
        AgentPrepareContextTool::render_parent_tool_catalog()
    })
    .await;
    assert_eq!(catalog, "- echo: durable\n");
}
