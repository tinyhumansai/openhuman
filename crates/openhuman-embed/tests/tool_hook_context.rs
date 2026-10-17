//! Hook attribution and cwd agree with the builtin tool's actual working directory.

mod common;

use common::{
    chat_completion, offline_config, route, runtime, scripted_provider, stub_backend,
    tool_call_completion,
};
use openhuman_embed::seams::{ToolHook, ToolHookContext};
use openhuman_embed::{Access, AgentDefinitionSpec, AgentSpec, Runtime, ToolScopeSpec, Workspace};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

struct Capture(Arc<Mutex<Vec<Value>>>);
#[async_trait::async_trait]
impl ToolHook for Capture {
    fn name(&self) -> &str {
        "context-probe"
    }
    async fn before_tool(&self, ctx: &ToolHookContext) -> anyhow::Result<()> {
        self.0.lock().unwrap().push(serde_json::to_value(ctx)?);
        Ok(())
    }
    async fn after_tool(&self, ctx: &ToolHookContext) -> anyhow::Result<()> {
        self.0.lock().unwrap().push(serde_json::to_value(ctx)?);
        Ok(())
    }
}

#[test]
fn cwd_and_identity_follow_the_turn_and_then_return_to_the_agent_defaults() {
    runtime().block_on(async { tokio::spawn(scenario()).await.unwrap() });
}

async fn scenario() {
    let backend = stub_backend().await;
    let provider = scripted_provider(
        (0..2)
            .flat_map(|_| {
                [
                    tool_call_completion("shell", &json!({"command":"pwd; printf '\\nowner=%s home=%s\\n' \"$TURN_OWNER\" \"$HOME\""}).to_string()),
                    chat_completion("done"),
                ]
            })
            .collect(),
        "done",
    )
    .await;
    let runtime = Runtime::builder()
        .config(offline_config())
        .workspace(Workspace::Ephemeral)
        .backend_url(backend.uri())
        .build()
        .await
        .unwrap();
    let default_dir = tempfile::tempdir().unwrap();
    let turn_dir = tempfile::tempdir().unwrap();
    let turn_cwd = turn_dir.path().canonicalize().unwrap();
    let observed = Arc::new(Mutex::new(Vec::new()));
    let agent = runtime
        .agent(
            AgentSpec::new("cwd-worker")
                .provider(route(&provider, "fixture"))
                .access(Access::full())
                .action_dir(default_dir.path())
                .tool_hook(Arc::new(Capture(observed.clone())))
                .definition(
                    AgentDefinitionSpec::new().tools(ToolScopeSpec::Named(vec!["shell".into()])),
                ),
        )
        .unwrap();
    agent
        .turn("run pwd")
        .session("override-session")
        .cwd(turn_dir.path())
        .tool_env([("TURN_OWNER".into(), "isolated".into())])
        .send()
        .await
        .unwrap();
    agent
        .turn("run pwd again")
        .session("default-session")
        .send()
        .await
        .unwrap();
    let observed = observed.lock().unwrap();
    assert_eq!(observed.len(), 4);
    assert!(
        observed[1]["output"]
            .as_str()
            .unwrap()
            .contains("owner=isolated"),
        "unexpected shell output: {}",
        observed[1]["output"]
    );
    for (pair, session, cwd) in [
        (&observed[..2], "override-session", turn_cwd.as_path()),
        (&observed[2..], "default-session", default_dir.path()),
    ] {
        for context in pair {
            assert_eq!(context["agent_id"], "cwd-worker", "{context}");
            assert_eq!(context["session_id"], session, "{context}");
            assert_eq!(context["cwd"], cwd.to_string_lossy().as_ref(), "{context}");
        }
        assert!(
            pair[1]["output"]
                .as_str()
                .unwrap()
                .contains(cwd.to_string_lossy().as_ref()),
            "builtin shell disagrees with hook cwd: {}",
            pair[1]
        );
    }
}
