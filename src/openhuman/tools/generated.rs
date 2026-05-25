//! Runtime-generated tool wrappers.
//!
//! This module gives trusted profile/runtime layers a narrow way to
//! expose generated capability tools without adding a bespoke Rust type
//! for each tool and without handing the model a broad raw bridge.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolCategory, ToolResult, ToolScope};

#[derive(Debug, Clone)]
pub struct GeneratedToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters_schema: Value,
    pub permission_level: PermissionLevel,
    pub category: ToolCategory,
    pub scope: ToolScope,
    pub adapter_id: String,
}

impl GeneratedToolDefinition {
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        parameters_schema: Value,
        adapter_id: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            parameters_schema,
            permission_level: PermissionLevel::ReadOnly,
            category: ToolCategory::Skill,
            scope: ToolScope::All,
            adapter_id: adapter_id.into(),
        }
    }
}

#[async_trait]
pub trait GeneratedToolAdapter: Send + Sync {
    fn id(&self) -> &str;

    async fn execute(
        &self,
        definition: &GeneratedToolDefinition,
        args: Value,
    ) -> anyhow::Result<ToolResult>;
}

pub struct GeneratedTool {
    definition: GeneratedToolDefinition,
    adapter: Arc<dyn GeneratedToolAdapter>,
}

impl GeneratedTool {
    pub fn new(
        mut definition: GeneratedToolDefinition,
        adapter: Arc<dyn GeneratedToolAdapter>,
    ) -> anyhow::Result<Self> {
        normalize_definition(&mut definition);
        if let Err(err) = validate_definition(&definition) {
            log::debug!(
                "[generated_tools] definition validation failed tool_name={} error={err}",
                definition.name
            );
            return Err(err);
        }
        if adapter.id() != definition.adapter_id {
            log::debug!(
                "[generated_tools] adapter mismatch tool_name={} required_adapter={} actual_adapter={}",
                definition.name,
                definition.adapter_id,
                adapter.id()
            );
            anyhow::bail!(
                "generated tool `{}` requires adapter `{}` but got `{}`",
                definition.name,
                definition.adapter_id,
                adapter.id()
            );
        }
        Ok(Self {
            definition,
            adapter,
        })
    }

    pub fn definition(&self) -> &GeneratedToolDefinition {
        &self.definition
    }
}

#[async_trait]
impl Tool for GeneratedTool {
    fn name(&self) -> &str {
        &self.definition.name
    }

    fn description(&self) -> &str {
        &self.definition.description
    }

    fn parameters_schema(&self) -> Value {
        self.definition.parameters_schema.clone()
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        self.adapter.execute(&self.definition, args).await
    }

    fn permission_level(&self) -> PermissionLevel {
        self.definition.permission_level
    }

    fn scope(&self) -> ToolScope {
        self.definition.scope
    }

    fn category(&self) -> ToolCategory {
        self.definition.category
    }
}

pub fn generated_tools_from_definitions(
    definitions: Vec<GeneratedToolDefinition>,
    adapter: Arc<dyn GeneratedToolAdapter>,
) -> anyhow::Result<Vec<Box<dyn Tool>>> {
    definitions
        .into_iter()
        .map(|definition| {
            GeneratedTool::new(definition, Arc::clone(&adapter))
                .map(|tool| Box::new(tool) as Box<dyn Tool>)
        })
        .collect()
}

fn normalize_definition(definition: &mut GeneratedToolDefinition) {
    definition.name = definition.name.trim().to_string();
    definition.description = definition.description.trim().to_string();
    definition.adapter_id = definition.adapter_id.trim().to_string();
}

fn validate_definition(definition: &GeneratedToolDefinition) -> anyhow::Result<()> {
    let name = definition.name.trim();
    if name.is_empty() {
        anyhow::bail!("generated tool name must be non-empty");
    }
    if definition.description.trim().is_empty() {
        anyhow::bail!("generated tool `{name}` description must be non-empty");
    }
    if definition.adapter_id.trim().is_empty() {
        anyhow::bail!("generated tool `{name}` adapter_id must be non-empty");
    }
    crate::openhuman::tools::schema::SchemaCleanr::validate(&definition.parameters_schema)
        .map_err(|err| anyhow::anyhow!("generated tool `{name}` has invalid schema: {err}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct EchoAdapter;

    #[async_trait]
    impl GeneratedToolAdapter for EchoAdapter {
        fn id(&self) -> &str {
            "echo-adapter"
        }

        async fn execute(
            &self,
            definition: &GeneratedToolDefinition,
            args: Value,
        ) -> anyhow::Result<ToolResult> {
            Ok(ToolResult::success(
                json!({
                    "tool": definition.name,
                    "adapter": definition.adapter_id,
                    "args": args,
                })
                .to_string(),
            ))
        }
    }

    fn sample_definition() -> GeneratedToolDefinition {
        let mut definition = GeneratedToolDefinition::new(
            "send_update",
            "Send a scoped update through a trusted adapter.",
            json!({
                "type": "object",
                "properties": {
                    "message": { "type": "string" }
                },
                "required": ["message"]
            }),
            "echo-adapter",
        );
        definition.permission_level = PermissionLevel::Write;
        definition
    }

    #[tokio::test]
    async fn generated_tool_executes_through_adapter() {
        let tool = GeneratedTool::new(sample_definition(), Arc::new(EchoAdapter)).unwrap();

        let result = tool
            .execute(json!({ "message": "hello" }))
            .await
            .expect("execute");

        assert_eq!(tool.name(), "send_update");
        assert_eq!(tool.permission_level(), PermissionLevel::Write);
        assert_eq!(tool.category(), ToolCategory::Skill);
        assert!(result.output().contains("send_update"));
        assert!(result.output().contains("hello"));
    }

    #[test]
    fn generated_tools_from_definitions_returns_tool_objects() {
        let tools =
            generated_tools_from_definitions(vec![sample_definition()], Arc::new(EchoAdapter))
                .unwrap();

        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name(), "send_update");
        assert_eq!(tools[0].parameters_schema()["type"], json!("object"));
    }

    #[test]
    fn generated_tool_rejects_adapter_mismatch() {
        let mut definition = sample_definition();
        definition.adapter_id = "missing-adapter".into();

        match GeneratedTool::new(definition, Arc::new(EchoAdapter)) {
            Ok(_) => panic!("adapter mismatch should fail"),
            Err(err) => assert!(err.to_string().contains("requires adapter")),
        }
    }

    #[test]
    fn generated_tool_rejects_blank_adapter_id() {
        let mut definition = sample_definition();
        definition.adapter_id = "  ".into();

        match GeneratedTool::new(definition, Arc::new(EchoAdapter)) {
            Ok(_) => panic!("blank adapter_id should fail"),
            Err(err) => assert!(err.to_string().contains("adapter_id must be non-empty")),
        }
    }

    #[test]
    fn generated_tool_normalizes_definition_fields() {
        let mut definition = sample_definition();
        definition.name = " send_update ".into();
        definition.description = " Send a scoped update. ".into();
        definition.adapter_id = " echo-adapter ".into();

        let tool = GeneratedTool::new(definition, Arc::new(EchoAdapter)).unwrap();

        assert_eq!(tool.name(), "send_update");
        assert_eq!(tool.description(), "Send a scoped update.");
        assert_eq!(tool.definition().adapter_id, "echo-adapter");
    }
}
