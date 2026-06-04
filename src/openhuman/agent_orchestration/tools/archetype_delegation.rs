use async_trait::async_trait;
use serde_json::json;
use serde_json::Value;

use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolCategory, ToolResult};

pub struct ArchetypeDelegationTool {
    pub tool_name: String,
    pub agent_id: String,
    pub tool_description: String,
}

#[async_trait]
impl Tool for ArchetypeDelegationTool {
    fn name(&self) -> &str {
        &self.tool_name
    }

    fn description(&self) -> &str {
        &self.tool_description
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["prompt"],
            "properties": {
                "prompt": {
                    "type": "string",
                    "description": "Brief task instruction. Prefer structured fields below for context; the sub-agent has no memory of your conversation."
                },
                "objective": {
                    "type": "string",
                    "description": "One sentence outcome the child must produce."
                },
                "evidence": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Only facts, file paths, URLs, ids, or tool outputs the parent has actually observed."
                },
                "constraints": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Hard requirements or limits the child must follow."
                },
                "must_not_assume": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Claims or facts the child must not infer without evidence."
                },
                "expected_output": {
                    "type": "string",
                    "description": "Requested output shape, e.g. findings list, patch summary, cited answer."
                },
                "citation_requirement": {
                    "type": "string",
                    "enum": ["none", "file_paths", "urls", "retrieval_hits", "tool_outputs"],
                    "description": "Citation/evidence style the child must preserve in its result."
                },
                "model": {
                    "type": "string",
                    "description": "Optional exact model id for this delegation only. Keeps the parent provider/routing, but pins the child agent to this model instead of the agent definition's default."
                }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::System
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let raw_prompt = args
            .get("prompt")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();

        if raw_prompt.is_empty() {
            return Ok(ToolResult::error(format!(
                "{}: `prompt` is required",
                self.tool_name
            )));
        }
        let prompt = render_structured_handoff(&raw_prompt, &args);

        let model_override = args
            .get("model")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());

        super::dispatch_subagent(
            &self.agent_id,
            &self.tool_name,
            &prompt,
            None,
            model_override,
        )
        .await
    }
}

fn render_structured_handoff(prompt: &str, args: &Value) -> String {
    let mut out = String::new();
    out.push_str("Task:\n");
    out.push_str(prompt.trim());

    push_optional_string(&mut out, "Objective", args.get("objective"));
    push_optional_array(&mut out, "Evidence", args.get("evidence"));
    push_optional_array(&mut out, "Constraints", args.get("constraints"));
    push_optional_array(&mut out, "Must not assume", args.get("must_not_assume"));
    push_optional_string(&mut out, "Expected output", args.get("expected_output"));
    push_optional_string(
        &mut out,
        "Citation requirement",
        args.get("citation_requirement"),
    );

    out
}

fn push_optional_string(out: &mut String, label: &str, value: Option<&Value>) {
    let Some(text) = value.and_then(Value::as_str).map(str::trim) else {
        return;
    };
    if text.is_empty() {
        return;
    }
    out.push_str("\n\n");
    out.push_str(label);
    out.push_str(":\n");
    out.push_str(text);
}

fn push_optional_array(out: &mut String, label: &str, value: Option<&Value>) {
    let Some(items) = value.and_then(Value::as_array) else {
        return;
    };
    let strings: Vec<&str> = items
        .iter()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if strings.is_empty() {
        return;
    }
    out.push_str("\n\n");
    out.push_str(label);
    out.push_str(":\n");
    for item in strings {
        out.push_str("- ");
        out.push_str(item);
        out.push('\n');
    }
    if out.ends_with('\n') {
        out.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::agent::harness::definition::AgentDefinitionRegistry;

    fn sample_tool() -> ArchetypeDelegationTool {
        ArchetypeDelegationTool {
            tool_name: "delegate_researcher".to_string(),
            agent_id: "researcher".to_string(),
            tool_description: "Use for web and docs research.".to_string(),
        }
    }

    #[test]
    fn metadata_methods_expose_name_description_and_system_category() {
        let tool = sample_tool();
        assert_eq!(tool.name(), "delegate_researcher");
        assert_eq!(tool.description(), "Use for web and docs research.");
        assert_eq!(tool.permission_level(), PermissionLevel::Execute);
        assert_eq!(tool.category(), ToolCategory::System);
    }

    #[test]
    fn parameters_schema_requires_prompt_only() {
        let tool = sample_tool();
        let schema = tool.parameters_schema();
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["required"], json!(["prompt"]));
        assert_eq!(schema["properties"]["prompt"]["type"], "string");
        assert_eq!(schema["properties"]["objective"]["type"], "string");
        assert_eq!(schema["properties"]["evidence"]["type"], "array");
        assert_eq!(
            schema["properties"]["citation_requirement"]["enum"],
            json!([
                "none",
                "file_paths",
                "urls",
                "retrieval_hits",
                "tool_outputs"
            ])
        );
    }

    #[test]
    fn structured_handoff_renders_compact_child_prompt() {
        let rendered = render_structured_handoff(
            "Check this",
            &json!({
                "prompt": "Check this",
                "objective": "Answer with supported claims only.",
                "evidence": ["file:src/lib.rs", "tool output: count=3", ""],
                "constraints": ["Do not edit files"],
                "must_not_assume": ["Current service state"],
                "expected_output": "Findings list",
                "citation_requirement": "file_paths",
            }),
        );

        assert!(rendered.contains("Task:\nCheck this"));
        assert!(rendered.contains("Objective:\nAnswer with supported claims only."));
        assert!(rendered.contains("Evidence:\n- file:src/lib.rs\n- tool output: count=3"));
        assert!(rendered.contains("Must not assume:\n- Current service state"));
        assert!(rendered.contains("Citation requirement:\nfile_paths"));
        assert!(!rendered.contains("\"model\""));
    }

    #[tokio::test]
    async fn execute_rejects_missing_or_blank_prompt() {
        let tool = sample_tool();

        let missing = tool.execute(json!({})).await.unwrap();
        assert!(missing.is_error);
        assert!(missing.output().contains("`prompt` is required"));

        let blank = tool.execute(json!({ "prompt": "   " })).await.unwrap();
        assert!(blank.is_error);
        assert!(blank.output().contains("`prompt` is required"));
    }

    #[tokio::test]
    async fn execute_accepts_non_empty_prompt_and_reaches_dispatch_path() {
        let _ = AgentDefinitionRegistry::init_global_builtins();
        let tool = sample_tool();
        let result = tool
            .execute(json!({ "prompt": "find the answer" }))
            .await
            .unwrap();

        let out = result.output();
        assert!(
            !out.contains("`prompt` is required"),
            "non-empty prompt should bypass local validation, got: {out}"
        );
    }
}
