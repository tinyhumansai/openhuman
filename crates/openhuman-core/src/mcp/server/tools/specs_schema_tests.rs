use super::*;
use serde_json::json;

#[test]
fn web_answer_static_schema_matches_json_literal() {
    let expected = json!({
        "type": "object",
        "properties": {
            "query": {"type": "string", "minLength": 1, "description": "The question to answer."},
            "depth": {"type": "string", "enum": ["quick", "deep"], "description": "quick (default) or deep."}
        },
        "required": ["query"],
        "additionalProperties": false
    });
    assert_eq!(web_answer_tool_spec().input_schema, expected);
}

#[test]
fn run_subagent_static_schema_matches_json_literal() {
    let expected = json!({
        "type": "object",
        "properties": {
            "agent_id": {
                "type": "string",
                "description": "Registered sub-agent id (for example `planner`, `code_executor`, `critic`)."
            },
            "prompt": {
                "type": "string",
                "description": "Task prompt for the sub-agent. Include the context it needs because this is a fresh session."
            }
        },
        "required": ["agent_id", "prompt"],
        "additionalProperties": false
    });
    let spec = base_tool_specs()
        .into_iter()
        .find(|spec| spec.name == "agent.run_subagent")
        .expect("agent.run_subagent spec");
    assert_eq!(spec.input_schema, expected);
}
