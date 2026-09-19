use super::*;

// #6276: bare calls to withheld packed tools, driven through a whole `OpenHumanSessionHost::turn`.
// Split out of `agent_turn_loop_tests.rs` to keep that file under the layout gate.

/// Stands in for a packed tool and records the arguments it ran with.
struct RecordingPackedTool(Arc<Mutex<Option<serde_json::Value>>>);

#[async_trait]
impl Tool for RecordingPackedTool {
    fn name(&self) -> &str {
        "skill_registry_search"
    }

    fn description(&self) -> &str {
        "Search available skills by keyword."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": { "query": { "type": "string" } },
            "required": ["query"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> Result<ToolResult> {
        *self.0.lock().unwrap() = Some(args);
        Ok(ToolResult::success("found: code-reviewer"))
    }
}

// #6276: the model reads a packed tool's bare name (pack listing, sibling
// descriptions) and calls it as a top-level tool. That call must reach the tool
// through its pack, not loop on "unknown tool".
#[tokio::test]
async fn turn_routes_a_bare_packed_tool_call_through_use_skill() {
    let provider = Arc::new(ScriptedProvider::new(vec![
        tool_response(vec![ToolCall {
            id: "tc1".into(),
            name: "skill_registry_search".into(),
            arguments: r#"{"query": "code review"}"#.into(),
            extra_content: None,
        }]),
        text_response("found one"),
    ]));
    let ran = Arc::new(Mutex::new(None));
    let mut tools: Vec<Box<dyn Tool>> = vec![Box::new(RecordingPackedTool(ran.clone()))];
    crate::tools::toolpacks::append_pack_tools(&mut tools);

    let (mut agent, _tmp) = build_agent_with(provider, tools, Box::new(NativeDialect));
    assert!(
        !agent
            .visible_tool_names_for_test()
            .contains("skill_registry_search"),
        "precondition: the tool is withheld behind its pack"
    );

    agent.turn("find a code review skill").await.unwrap();

    assert_eq!(
        ran.lock().unwrap().clone(),
        Some(serde_json::json!({ "query": "code review" })),
        "a bare call to a withheld packed tool must run that tool with its own arguments"
    );
    let results: Vec<String> = agent
        .history()
        .iter()
        .filter_map(|msg| match msg {
            ConversationMessage::ToolResults(results) => Some(
                results
                    .iter()
                    .map(|r| r.content.clone())
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        })
        .flatten()
        .collect();
    assert!(
        results.iter().any(|c| c.contains("found: code-reviewer"))
            && !results.iter().any(|c| c.contains("unknown tool")),
        "the model must see the tool's result, not an unknown-tool error: {results:?}"
    );
}

/// A packed tool that needs `Write` and records whether it ran.
struct WritePackedTool(Arc<Mutex<bool>>);

#[async_trait]
impl Tool for WritePackedTool {
    fn name(&self) -> &str {
        "skill_registry_install"
    }

    fn description(&self) -> &str {
        "Install a skill from the catalog."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object" })
    }

    async fn execute(&self, _args: serde_json::Value) -> Result<ToolResult> {
        *self.0.lock().unwrap() = true;
        Ok(ToolResult::success("installed"))
    }

    fn permission_level(&self) -> tinytools::PermissionLevel {
        tinytools::PermissionLevel::Write
    }
}

// #6276 review: routing must never be a way around the session's gate. A packed
// tool this channel may not run is not rewritten at all: it keeps the crate's
// unknown-tool answer and never executes.
#[tokio::test]
async fn turn_does_not_route_a_bare_call_the_session_would_refuse() {
    let provider = Arc::new(ScriptedProvider::new(vec![
        tool_response(vec![ToolCall {
            id: "tc1".into(),
            name: "skill_registry_install".into(),
            arguments: r#"{"entry_id": "code-reviewer"}"#.into(),
            extra_content: None,
        }]),
        text_response("could not install"),
    ]));
    let ran = Arc::new(Mutex::new(false));
    let mut tools: Vec<Box<dyn Tool>> = vec![Box::new(WritePackedTool(ran.clone()))];
    crate::tools::toolpacks::append_pack_tools(&mut tools);
    // The builder's default event channel is `internal`; cap it at read-only.
    let config = AgentConfig {
        channel_permissions: std::collections::HashMap::from([(
            "internal".to_string(),
            "read".to_string(),
        )]),
        ..AgentConfig::default()
    };

    let (mut agent, _tmp) = build_agent_with_config(provider, tools, config);
    agent.turn("install a code review skill").await.unwrap();

    assert!(
        !*ran.lock().unwrap(),
        "a packed tool the session blocks must not run through a bare call"
    );
    let unrouted = agent.history().iter().any(|msg| match msg {
        ConversationMessage::ToolResults(results) => results
            .iter()
            .any(|r| r.content.contains("unknown tool `skill_registry_install`")),
        _ => false,
    });
    assert!(
        unrouted,
        "a bare call the session would refuse must not be rewritten into use_skill: {:?}",
        agent.history()
    );
}
