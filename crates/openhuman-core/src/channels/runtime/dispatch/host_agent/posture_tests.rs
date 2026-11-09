use super::*;
use crate::agent::HostTurnTools;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tinytools::{ToolCallOptions, ToolResult};

fn external() -> AgentTurnOrigin {
    AgentTurnOrigin::ExternalChannel {
        channel: "telegram".into(),
        sender: Some("alice".into()),
        sender_name: None,
        reply_target: "42".into(),
        message_id: "m1".into(),
        history_key: Some("telegram_alice".into()),
        channel_session: None,
    }
}

/// Records how often it actually ran.
struct Probe {
    level: PermissionLevel,
    effect: bool,
    runs: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl Tool for Probe {
    fn name(&self) -> &str {
        "probe"
    }
    fn description(&self) -> &str {
        "a probe"
    }
    fn parameters_schema(&self) -> Value {
        json!({ "type": "object", "properties": {} })
    }
    fn permission_level(&self) -> PermissionLevel {
        self.level
    }
    fn external_effect(&self) -> bool {
        self.effect
    }
    fn external_effect_with_args(&self, _args: &Value) -> bool {
        self.effect
    }
    async fn execute(&self, _args: Value) -> anyhow::Result<ToolResult> {
        self.runs.fetch_add(1, Ordering::SeqCst);
        Ok(ToolResult::success("ran"))
    }
}

fn guarded(level: PermissionLevel, effect: bool) -> (CeilingGuard, Arc<AtomicUsize>) {
    let runs = Arc::new(AtomicUsize::new(0));
    let probe = Probe {
        level,
        effect,
        runs: Arc::clone(&runs),
    };
    (
        CeilingGuard::new(Box::new(probe), PermissionLevel::ReadOnly),
        runs,
    )
}

#[test]
fn an_external_channel_turn_is_capped_at_read_only() {
    assert_eq!(tool_ceiling(&external()), Some(PermissionLevel::ReadOnly));
}

#[test]
fn other_origins_keep_the_agents_own_limits() {
    assert_eq!(tool_ceiling(&AgentTurnOrigin::Cli), None);
    assert_eq!(tool_ceiling(&AgentTurnOrigin::DirectChat), None);
}

#[test]
fn the_channel_is_capped_in_the_turns_config() {
    let mut config = Config::default();
    config
        .agent
        .channel_permissions
        .insert("telegram".into(), "execute".into());
    cap_channel(&mut config, "telegram", PermissionLevel::ReadOnly);
    assert_eq!(config.agent.channel_permissions["telegram"], "readonly");

    // Never widened: an operator's `none` stays `none`.
    config
        .agent
        .channel_permissions
        .insert("telegram".into(), "none".into());
    cap_channel(&mut config, "telegram", PermissionLevel::ReadOnly);
    assert_eq!(config.agent.channel_permissions["telegram"], "none");
}

#[tokio::test]
async fn a_read_only_call_runs() {
    let (tool, runs) = guarded(PermissionLevel::ReadOnly, false);
    let result = tool.execute(json!({})).await.unwrap();
    assert!(!result.is_error);
    assert_eq!(runs.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_write_call_is_refused_without_running() {
    let (tool, runs) = guarded(PermissionLevel::Write, false);
    let result = tool
        .execute_with_options(json!({}), ToolCallOptions::default())
        .await
        .unwrap();
    assert!(result.is_error);
    let text = result.output_for_llm(true);
    assert!(text.contains("probe"), "{text}");
    assert!(text.contains("not available"), "{text}");
    assert_eq!(runs.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn an_external_effect_is_refused_and_never_parks() {
    let (tool, runs) = guarded(PermissionLevel::ReadOnly, true);
    assert!(
        !tool.external_effect_with_args(&json!({})),
        "the guard answers the call itself, so the approval gate is never asked"
    );
    let result = tool
        .execute_with_context(json!({}), ToolCallOptions::default(), None)
        .await
        .unwrap();
    assert!(result.is_error);
    assert_eq!(runs.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn guarded_host_tools_keep_the_rest_of_the_belt() {
    let runs = Arc::new(AtomicUsize::new(0));
    let probe_runs = Arc::clone(&runs);
    let host: HostTools = Arc::new(move |_turn| {
        let probe_runs = Arc::clone(&probe_runs);
        Box::pin(async move {
            HostTurnTools::advertised(vec![Box::new(Probe {
                level: PermissionLevel::Write,
                effect: false,
                runs: probe_runs,
            })])
        })
    });
    let guarded = guard_host_tools(host, PermissionLevel::ReadOnly);
    let belt = guarded(crate::agent::TurnContext::new("teeny", Some("s1"))).await;
    assert_eq!(belt.tools.len(), 1);
    assert!(belt.visible.contains("probe"));
    let result = belt.tools[0].execute(json!({})).await.unwrap();
    assert!(result.is_error, "the belt's tools are guarded");
    assert_eq!(runs.load(Ordering::SeqCst), 0);
}
