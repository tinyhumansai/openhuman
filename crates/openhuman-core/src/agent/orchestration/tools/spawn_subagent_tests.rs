use super::*;
#[test]
fn parameters_schema_hides_legacy_mode_flags_and_keeps_required_agent_id() {
    let tool = SpawnSubagentTool::new();
    let schema = tool.parameters_schema();
    let props = schema.get("properties").expect("schema has properties");
    for legacy in ["blocking", "dedicated_thread", "archetype", "toolkit"] {
        assert!(
            props.get(legacy).is_none(),
            "legacy `{legacy}` is not advertised"
        );
    }
    assert!(schema["required"]
        .as_array()
        .is_some_and(|required| { required.iter().any(|key| key.as_str() == Some("agent_id")) }));
}

#[test]
fn parameters_schema_advertises_optional_model_override() {
    let tool = SpawnSubagentTool::new();
    let schema = tool.parameters_schema();
    let props = schema.get("properties").expect("schema has properties");
    let model = props.get("model").expect("model override advertised");
    assert_eq!(model.get("type").and_then(|v| v.as_str()), Some("string"));
    assert!(schema
        .get("required")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().all(|s| s.as_str() != Some("model")))
        .unwrap_or(true));
}

#[tokio::test]
async fn missing_agent_id_returns_error() {
    let tool = SpawnSubagentTool::new();
    let result = tool
        .execute(json!({
            "prompt": "do thing"
        }))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("agent_id"));
}

#[tokio::test]
async fn missing_prompt_returns_error() {
    let tool = SpawnSubagentTool::new();
    let result = tool
        .execute(json!({
            "agent_id": "task_manager_agent"
        }))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("prompt"));
}

#[tokio::test]
async fn no_registry_returns_clear_error() {
    // The global registry has not been initialised in this test.
    let tool = SpawnSubagentTool::new();
    let result = tool
        .execute(json!({
            "agent_id": "task_manager_agent",
            "prompt": "find x",
        }))
        .await
        .unwrap();
    // Either: registry uninitialised → clear init error, OR
    // registry was initialised by a previous test → "no parent context"
    // because we're not running inside an OpenHumanSessionHost::turn. Both are
    // acceptable: the tool gracefully refuses.
    assert!(result.is_error);
}

#[tokio::test]
async fn unknown_agent_id_lists_available() {
    // Drive the real lookup under a detached parent and its scoped registry;
    // the ordinary Tool::execute path refuses earlier because it has no turn.
    let workspace = tempfile::tempdir().expect("workspace");
    let mut parent = super::tests::parent_context(workspace.path());
    parent.session_id = "unknown-agent-session".into();
    let (live, core) = super::tests::headless_core_backed_parent(workspace.path(), parent);
    let result = crate::core::runtime::CoreContext::scope(core, async {
        let detached =
            crate::agent::subagent_host::detached_subagent_parent(&live, "async-subagent")
                .expect("live parent can be detached");
        SpawnSubagentTool::new()
            .execute_with_live_parent_context(
                json!({
                    "agent_id": "totally_made_up",
                    "prompt": "x",
                }),
                None,
                live.data.child(),
                detached,
            )
            .await
            .unwrap()
    })
    .await;
    assert!(result.is_error);
    let out = result.output();
    assert!(
        out.contains("unknown agent_id 'totally_made_up'"),
        "the request must reach scoped registry lookup: {out}"
    );
    // Should list at least one valid built-in.
    assert!(out.contains("task_manager_agent"));
}

#[test]
fn frozen_legacy_mode_arguments_are_accepted_but_ignored() {
    let args = json!({
        "agent_id": "task_manager_agent",
        "prompt": "inspect the frozen-session compatibility route",
        "blocking": true,
        "dedicated_thread": true,
    });
    let decoded = decode_async_spawn_args(&args);
    let plain = decode_async_spawn_args(&json!({
        "agent_id": "task_manager_agent",
        "prompt": "inspect the frozen-session compatibility route",
    }));
    assert_eq!(decoded.agent_id, "task_manager_agent");
    assert_eq!(
        decoded.prompt,
        "inspect the frozen-session compatibility route"
    );
    assert_eq!(decoded.task_key, plain.task_key);
    assert!(!decoded.force_fresh);
}
