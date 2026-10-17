use super::*;

#[test]
fn custom_delegate_is_treated_as_spawn_tool() {
    assert!(is_subagent_spawn_tool("spawn_subagent"));
    assert!(is_subagent_spawn_tool("delegate_researcher"));
    assert!(!is_subagent_spawn_tool("directory_resolve"));
}

#[test]
fn unprefixed_delegate_name_overrides_are_treated_as_spawn_tools() {
    // Most synthesised delegation tools use an unprefixed
    // `delegate_name` override (`plan`, `manage_tasks`, `create_image`, …).
    // They must be stripped from every sub-agent surface, exactly like
    // the `delegate_*`-prefixed defaults.
    let tmp = tempfile::TempDir::new().unwrap();
    crate::agent::harness::host_definition::AgentDefinitionRegistry::init_global(tmp.path())
        .unwrap();
    for delegate in [
        "plan",
        "review_code",
        "manage_tasks",
        "create_image",
        // `make_presentation` is `presentation_agent`'s `delegate_name`; the agent —
        // and therefore this delegate tool — is compiled out with the
        // `documents` feature.
        #[cfg(feature = "documents")]
        "make_presentation",
    ] {
        assert!(
            is_subagent_spawn_tool(delegate),
            "`{delegate}` is a synthesised delegation tool and must be \
             stripped from sub-agent tool surfaces"
        );
    }
    // Ordinary worker tools stay visible.
    for plain in ["shell", "file_read", "web_fetch", "todo"] {
        assert!(
            !is_subagent_spawn_tool(plain),
            "`{plain}` must not be classified as a spawn tool"
        );
    }
}

#[test]
fn child_keeps_the_parents_protocol() {
    use tinytools_agent::dialect::ToolCallFormat;

    for parent in [
        ToolCallFormat::Native,
        ToolCallFormat::PFormat,
        ToolCallFormat::Json,
    ] {
        let (format, _) = subagent_prompt_protocol(parent, &[]);
        assert_eq!(format, parent);
    }
}

struct NamedTool(&'static str);

#[async_trait::async_trait]
impl Tool for NamedTool {
    fn name(&self) -> &str {
        self.0
    }
    fn description(&self) -> &str {
        "named"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object" })
    }
    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<tinytools::ToolResult> {
        Ok(tinytools::ToolResult::success("ok"))
    }
}

#[test]
fn rule_withheld_tools_leave_the_child_prompt_but_stay_allowed() {
    let tools: Vec<Box<dyn Tool>> = vec![
        Box::new(NamedTool("file_read")),
        Box::new(NamedTool("shell")),
        Box::new(NamedTool("web_fetch")),
    ];
    let rules: tinytools::ToolRules = serde_json::from_value(serde_json::json!({ "rules": [
        { "effect": "hide", "match": { "name": "shell" } },
        { "effect": "deny", "match": { "name": "web_*" } },
    ] }))
    .unwrap();
    let policy = tinyagents_harness::tool::ToolRulePolicy::new(rules);
    let allowed = vec![0, 1, 2];

    let listed = rule_listed_indices(&allowed, &tools, Some(&policy));
    assert_eq!(listed, vec![0], "hidden and denied tools leave the listing");
    let prompt = prompt_tools_for(&listed, &tools);
    assert_eq!(prompt.len(), 1);
    assert_eq!(prompt[0].name, "file_read");
    assert_eq!(rule_listed_indices(&allowed, &tools, None), allowed);
}
