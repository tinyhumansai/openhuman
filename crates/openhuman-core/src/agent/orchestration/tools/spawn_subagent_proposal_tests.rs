use super::*;
use crate::flows::builder_tools::tests_support::{edit_workflow_tool, propose_workflow_tool};
use serde_json::Value;
use std::sync::Arc;
use tinytools::Tool;

/// Four steps whose configs differ only by value, so a config landing on a
/// neighbouring node (an index shift anywhere between the tool and the stored
/// proposal) is visible.
fn four_step_graph() -> Value {
    json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "fetch", "kind": "agent", "name": "Fetch", "config": { "prompt": "fetch step" } },
            { "id": "rank", "kind": "agent", "name": "Rank", "config": { "prompt": "rank step" } },
            { "id": "report", "kind": "agent", "name": "Report", "config": { "prompt": "report step" } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "fetch" },
            { "from_node": "fetch", "to_node": "rank" },
            { "from_node": "rank", "to_node": "report" }
        ]
    })
}

fn test_config(workspace: &std::path::Path) -> Arc<crate::config::Config> {
    let config = crate::config::Config {
        workspace_dir: workspace.join("workspace"),
        action_dir: workspace.join("workspace"),
        config_path: workspace.join("config.toml"),
        ..crate::config::Config::default()
    };
    std::fs::create_dir_all(&config.workspace_dir).unwrap();
    Arc::new(config)
}

/// Persist `tool_output` as the child's last tool result and return the
/// proposal the parent thread stored for it.
fn persisted_proposal(workspace: &std::path::Path, task_id: &str, tool_output: String) -> Value {
    use crate::threads::store::CreateConversationThread;
    let ctx = DispatchCtx::ambient();
    conversations::ensure_thread(
        &ctx,
        workspace.to_path_buf(),
        CreateConversationThread {
            id: "thread-parent".into(),
            title: "Main chat".into(),
            created_at: chrono::Utc::now().to_rfc3339(),
            parent_thread_id: None,
            labels: None,
            personality_id: None,
            working_dir: None,
        },
    )
    .expect("thread created");
    attach_workflow_proposal(
        &ctx,
        workspace,
        Some("thread-parent"),
        task_id,
        "workflow_builder",
        &[TranscriptMessage::tool(&tool_output)],
        "proposal ready".to_string(),
    );
    conversations::get_messages(&ctx, workspace.to_path_buf(), "thread-parent")
        .expect("messages")
        .into_iter()
        .find(|message| message.id == format!("workflow-proposal:{task_id}"))
        .expect("proposal message persisted")
        .extra_metadata["proposal"]
        .clone()
}

fn assert_prompts(proposal: &Value, expected: &[(&str, &str)]) {
    let nodes = proposal["graph"]["nodes"]
        .as_array()
        .expect("proposal nodes");
    assert_eq!(nodes.len(), 4, "{proposal}");
    for (id, prompt) in expected {
        let node = nodes
            .iter()
            .find(|node| node["id"] == *id)
            .unwrap_or_else(|| panic!("node `{id}` missing from {proposal}"));
        assert_eq!(
            node["config"]["prompt"], *prompt,
            "node `{id}` carries another step's config: {proposal}"
        );
    }
}

#[tokio::test]
async fn proposed_and_edited_steps_keep_their_own_config_in_the_stored_proposal() {
    let _module = crate::integrations::composio::module_client::module_guard().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let config = test_config(temp.path());

    let proposed = propose_workflow_tool(config.clone())
        .execute(json!({ "name": "Four steps", "graph": four_step_graph() }))
        .await
        .unwrap();
    assert!(!proposed.is_error, "{}", proposed.output());
    let stored = persisted_proposal(temp.path(), "propose-1", proposed.output());
    assert_prompts(
        &stored,
        &[
            ("fetch", "fetch step"),
            ("rank", "rank step"),
            ("report", "report step"),
        ],
    );

    let edited = edit_workflow_tool(config)
        .execute(json!({
            "graph": stored["graph"],
            "ops": [ { "op": "update_node_config", "id": "rank", "config": { "prompt": "rank edited" } } ]
        }))
        .await
        .unwrap();
    assert!(!edited.is_error, "{}", edited.output());
    let stored = persisted_proposal(temp.path(), "edit-1", edited.output());
    assert_prompts(
        &stored,
        &[
            ("fetch", "fetch step"),
            ("rank", "rank edited"),
            ("report", "report step"),
        ],
    );
}
