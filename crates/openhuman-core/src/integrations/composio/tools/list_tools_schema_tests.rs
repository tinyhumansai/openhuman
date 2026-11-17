use super::*;
use serde_json::json;

fn expected_composio_list_tools() -> serde_json::Value {
    json!({
        "type": "object",
        "properties": {
            "toolkits": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Optional list of toolkit slugs to filter by."
            },
            "tags": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Optional Composio action tags to filter by \
                                (OR semantics — multiple tags broaden the result, \
                                e.g. [\"readOnlyHint\"] or [\"repos\", \"stars\"]). \
                                Case-insensitive."
            },
            "include_unconnected": {
                "type": "boolean",
                "description": "When true, include actions from toolkits the user \
                                has not connected yet. Defaults to false (only \
                                connected toolkits)."
            }
        },
        "additionalProperties": false
    })
}

#[test]
fn composio_list_tools_static_schema_matches_json_literal() {
    let tool = ComposioListToolsTool::new(std::sync::Arc::new(crate::config::Config::default()));
    assert_eq!(
        tinytools::Tool::parameters_schema(&tool),
        expected_composio_list_tools()
    );
}
