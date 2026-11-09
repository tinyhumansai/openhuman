use super::*;
use serde_json::json;

fn expected_memory() -> serde_json::Value {
    json!({
        "type": "object",
        "properties": {
            "action": {
                "type": "string",
                "enum": ["recall", "fetch", "learn", "forget"],
                "description": "What to do."
            },
            "question": {"type": "string", "description": "recall: the question to answer from memory."},
            "query": {"type": "string", "description": "fetch: what to search for."},
            "mode": {"type": "string", "enum": ["hybrid", "keyword", "vector"], "description": "fetch: retrieval mode; only the engine's declared modes work (hybrid is always safe)."},
            "filter": {"type": "object", "description": "recall/fetch: metadata filter, e.g. {\"kinds\": [\"learning\"], \"workspace\": \"/path\", \"repo\": \"owner/name\"}."},
            "cursor": {"type": "string", "description": "fetch: next-page cursor."},
            "refers_to": {
                "type": "object",
                "additionalProperties": false,
                "description": "recall/fetch: the local calendar dates the question is about, resolved against the Current Date & Time line. Set it whenever the question mentions or implies a time, in any language: \"yesterday\", \"last Saturday\", \"in March\", \"kal\", \"pichle hafte\", \"el viernes\". A single day has from == to. If a word can mean the past or the future (Hindi \"kal\"), cover both days. Prefer this over filter.observed_after/observed_before: those drop every memory recorded on another day, this only ranks the matching days first.",
                "properties": {
                    "from": {"type": "string", "format": "date", "description": "First local date, YYYY-MM-DD."},
                    "to": {"type": "string", "format": "date", "description": "Last local date, YYYY-MM-DD, inclusive."}
                },
                "required": ["from", "to"]
            },
            "text": {"type": "string", "description": "learn: the learning, one self-contained sentence."},
            "kind": {"type": "string", "enum": ["preference", "fact", "procedure", "correction", "other"], "description": "learn: what kind of learning (default fact)."},
            "confidence": {"type": "number", "minimum": 0, "maximum": 1, "description": "learn: confidence (default 0.8)."},
            "ids": {"type": "array", "items": {"type": "string"}, "description": "forget: item ids to remove."}
        },
        "required": ["action"]
    })
}

#[test]
fn memory_static_schema_matches_json_literal() {
    let tool = MemoryTool::new(std::sync::Arc::new(crate::config::Config::default()));
    assert_eq!(MEMORY_TOOL_NAME, "memory");
    assert_eq!(tinytools::Tool::name(&tool), "memory");
    assert_eq!(tinytools::Tool::parameters_schema(&tool), expected_memory());
}
