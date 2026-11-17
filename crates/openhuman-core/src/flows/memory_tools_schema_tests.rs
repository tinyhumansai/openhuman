use super::*;
use serde_json::json;

fn expected_flow_memory_remember() -> serde_json::Value {
    json!({
        "type": "object",
        "properties": {
            "flow_id": {
                "type": "string",
                "description": "Informational only: inside a running flow the active flow's own id \
                 (from the run's trusted origin) is authoritative and this value is ignored. This \
                 tool ONLY works inside a workflow run — calling it from chat or any other context \
                 without a trusted run origin is refused, regardless of what is passed here."
            },
            "key": {
                "type": "string",
                "description": "Unique key for this memory within the flow's own memory"
            },
            "content": {
                "type": "string",
                "description": "The information to remember"
            },
            "category": {
                "type": "string",
                "description": "What kind of statement this is: 'fact' (default), 'preference', 'procedure', 'correction', or anything else (stored as 'other')."
            }
        },
        "required": ["flow_id", "key", "content"]
    })
}

#[test]
fn flow_memory_remember_static_schema_matches_json_literal() {
    let tool = FlowMemoryRememberTool::new(std::sync::Arc::new(
        crate::security::SecurityPolicy::default(),
    ));
    assert_eq!(
        tinytools::Tool::parameters_schema(&tool),
        expected_flow_memory_remember()
    );
}
