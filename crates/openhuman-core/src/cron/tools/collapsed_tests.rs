use super::*;
use crate::agent::tinyagents::host::OpenHumanRunContext;
use crate::core::runtime::turn_origin::AgentTurnOrigin;
use crate::core::runtime::{ComponentSet, CoreContext, DomainSet};
use tinyagents_harness::{context::RunConfig, ids::CallId, tool::ToolExecutionContext};

fn tool() -> CronTool {
    CronTool::new(
        Arc::new(Config::default()),
        Arc::new(SecurityPolicy::default()),
    )
}

fn execution_context(run: OpenHumanRunContext) -> ToolExecutionContext {
    let run = run.into_tinyagents(RunConfig::new("cron-collapsed-context"));
    ToolExecutionContext::from_run_context(&run, CallId::new("cron-collapsed-call"))
}

#[test]
fn the_schema_advertises_every_action() {
    let schema = tool().parameters_schema();
    let actions = schema["properties"]["action"]["enum"]
        .as_array()
        .expect("enum")
        .iter()
        .map(|v| v.as_str().unwrap_or_default().to_string())
        .collect::<Vec<_>>();
    assert_eq!(
        actions,
        vec!["list", "add", "update", "remove", "run", "runs"]
    );
}

#[test]
fn the_schema_carries_the_members_parameters() {
    // `job_id` comes from four members and `patch` only from `update`.
    // Their presence is what proves the merge read the members rather than
    // a hand-written union that could drift from them.
    let schema = tool().parameters_schema();
    let props = schema["properties"].as_object().expect("properties");
    assert!(props.contains_key("job_id"));
    assert!(props.contains_key("patch"));
}

#[test]
fn a_read_only_action_is_not_reported_as_execute() {
    // The whole point of `permission_level_with_args`: collapsing must not
    // silently promote `list` to the privilege `add` needs.
    let tool = tool();
    let listing = serde_json::json!({"action": "list"});
    assert!(
        tool.permission_level_with_args(&listing) < tool.permission_level(),
        "list must resolve below the family's strictest level"
    );
}

#[test]
fn an_unknown_action_falls_back_to_the_strictest_level() {
    let tool = tool();
    let nonsense = serde_json::json!({"action": "definitely_not_an_action"});
    assert_eq!(
        tool.permission_level_with_args(&nonsense),
        tool.permission_level()
    );
}

#[test]
fn a_missing_action_falls_back_to_the_strictest_level() {
    let tool = tool();
    assert_eq!(
        tool.permission_level_with_args(&serde_json::json!({})),
        tool.permission_level()
    );
}

#[tokio::test]
async fn an_unknown_action_is_an_error_result_naming_the_valid_ones() {
    let result = tool()
        .execute(serde_json::json!({"action": "nope"}))
        .await
        .expect("dispatch does not fail the call");
    assert!(result.is_error);
    let text = format!("{result:?}");
    assert!(text.contains("nope"), "names what was passed: {text}");
    assert!(text.contains("list|add"), "names the valid actions: {text}");
}

#[test]
fn the_collapsed_tool_is_deferred_and_its_members_are_advertised() {
    // The load-bearing assertion. Shipping both the collapsed `cron` and its
    // members up front would put every scheduler schema on the wire twice;
    // the pack advertises the members, so `cron` itself waits for
    // `tool_search`.
    assert_eq!(tool().exposure(), ToolExposure::Deferred);
    for entry in tool().actions() {
        assert_eq!(
            entry.tool.exposure(),
            ToolExposure::Direct,
            "`{}` is not advertised; the scheduling pack lists it",
            entry.tool.name()
        );
    }
}

#[tokio::test]
async fn run_uses_the_explicit_core_context_and_preserves_markdown_options() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = tmp.path().join("workspace");
    tokio::fs::create_dir_all(&workspace).await.unwrap();
    let config = Arc::new(Config {
        workspace_dir: workspace.clone(),
        action_dir: workspace,
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    });
    let job = crate::cron::add_flow_schedule_job(
        &config,
        "cron-collapsed-context-flow",
        tinyflows_bus::schedule::types::Schedule::Every { every_ms: 300_000 },
    )
    .unwrap();

    let mut components = ComponentSet::all();
    components.flows = false;
    let explicit = CoreContext::for_test_with_components(components);
    let ambient = CoreContext::for_test(DomainSet::full(), None);
    let context =
        execution_context(OpenHumanRunContext::new().with_core(Some(Arc::clone(&explicit))));
    let cron = CronTool::new(config, Arc::new(SecurityPolicy::default()));

    let result = CoreContext::scope(ambient, async {
        cron.execute_with_context(
            serde_json::json!({"action": "run", "job_id": job.id}),
            ToolCallOptions::prefer_markdown(),
            Some(&context),
        )
        .await
        .unwrap()
    })
    .await;

    assert!(
        !result.is_error,
        "a disabled flow schedule is skipped: {}",
        result.output()
    );
    assert!(
        result.output().contains("COMPONENT_DISABLED:flows"),
        "{}",
        result.output()
    );
    assert!(result
        .markdown_formatted
        .as_deref()
        .is_some_and(|md| md.contains("status")));
}

/// The trusted durable-session identity a channel turn carries. Jobs created
/// from a channel conversation record it; a turn without one has no
/// conversation to bind a job to.
fn channel_session(reply_target: &str) -> tinyflows_bus::schedule::ChannelSessionIdentity {
    tinyflows_bus::schedule::ChannelSessionIdentity {
        tenant_key: "profile:alice".into(),
        agent_id: "orchestrator".into(),
        session_key: format!("main:telegram:bot-a:dm:{reply_target}"),
        provider: "telegram".into(),
        account_id: Some("telegram:bot-a".into()),
        conversation: tinyflows_bus::schedule::ChannelConversationIdentity {
            kind: "dm".into(),
            id: reply_target.into(),
            scope_id: None,
            parent_id: None,
            thread_id: None,
            topic_id: None,
        },
    }
}

#[tokio::test]
async fn add_uses_the_explicit_channel_origin_without_ambient_turn_state() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = tmp.path().join("workspace");
    tokio::fs::create_dir_all(&workspace).await.unwrap();
    let mut cfg = Config {
        workspace_dir: workspace.clone(),
        action_dir: workspace,
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    cfg.cron.agent_jobs_only = true;
    let config = Arc::new(cfg);
    let origin = AgentTurnOrigin::ExternalChannel {
        sender_name: None,
        channel: "telegram".into(),
        sender: Some("alice".into()),
        reply_target: "42".into(),
        message_id: "message-1".into(),
        history_key: Some("telegram-alice-42".into()),
        channel_session: Some(channel_session("42")),
    };
    let mut run = OpenHumanRunContext::new();
    run.origin = Some(origin);
    let context = execution_context(run);
    let cron = CronTool::new(config.clone(), Arc::new(SecurityPolicy::default()));

    let result = cron
        .execute_with_context(
            serde_json::json!({
                "action": "add",
                "schedule": {"kind": "every", "every_ms": 300000},
                "job_type": "agent",
                "prompt": "send a reminder"
            }),
            ToolCallOptions::default(),
            Some(&context),
        )
        .await
        .unwrap();

    assert!(!result.is_error, "{}", result.output());
    let jobs = crate::cron::list_jobs(&config).unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(
        jobs[0].origin,
        Some(crate::cron::JobOrigin::Channel {
            channel: "telegram".into(),
            reply_target: "42".into(),
            history_key: "telegram-alice-42".into(),
            sender: Some("alice".into()),
            thread_id: None,
            channel_session: Some(Box::new(channel_session("42"))),
        })
    );
    assert_eq!(jobs[0].delivery.mode, "origin");
}

#[test]
fn description_says_conversation_jobs_reply_automatically_and_documents_overrides() {
    let cron = tool();
    let description = cron.description();
    assert!(description.contains("automatically"));
    assert!(description.contains("proactive"));
    assert!(description.contains("none"));
    assert!(!description.contains("announce"));
}
