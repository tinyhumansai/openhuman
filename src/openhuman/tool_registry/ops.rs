use std::collections::BTreeMap;

use serde_json::{json, Map, Value};

use crate::core::all;
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::mcp_server::McpToolSpec;
use crate::rpc::RpcOutcome;

use super::types::{
    ToolRegistryEntry, ToolRegistryHealth, ToolRegistryList, ToolRegistryTransport,
};

const REGISTRY_ENTRY_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Return the current read-only tool registry snapshot.
pub fn list_tools() -> RpcOutcome<ToolRegistryList> {
    let tools = registry_entries();
    log::debug!(
        "[tool_registry] list_tools completed entries={}",
        tools.len()
    );
    RpcOutcome::new(ToolRegistryList { tools }, vec![])
}

/// Look up one registry entry by stable `tool_id`.
pub fn get_tool(tool_id: &str) -> Result<RpcOutcome<ToolRegistryEntry>, String> {
    let normalized = tool_id.trim();
    if normalized.is_empty() {
        return Err("tool_id must be a non-empty string".to_string());
    }

    let tool = registry_entries()
        .into_iter()
        .find(|entry| entry.tool_id == normalized)
        .ok_or_else(|| format!("tool not found in registry: {normalized}"))?;

    log::debug!(
        "[tool_registry] get_tool completed tool_id={} transport={:?}",
        tool.tool_id,
        tool.transport
    );
    Ok(RpcOutcome::new(tool, vec![]))
}

/// Build sorted registry entries from the current MCP and controller metadata.
pub fn registry_entries() -> Vec<ToolRegistryEntry> {
    let mut entries = BTreeMap::new();

    for spec in crate::openhuman::mcp_server::tool_specs() {
        let entry = mcp_tool_entry(spec);
        insert_registry_entry(&mut entries, entry, "mcp_stdio");
    }

    for schema in crate::openhuman::tools::all_tools_controller_schemas() {
        let entry = controller_tool_entry(&schema);
        insert_registry_entry(&mut entries, entry, "controller");
    }

    entries.into_values().collect()
}

fn insert_registry_entry(
    entries: &mut BTreeMap<String, ToolRegistryEntry>,
    entry: ToolRegistryEntry,
    source: &str,
) {
    let key = entry.tool_id.clone();
    assert!(
        entries.insert(key.clone(), entry).is_none(),
        "duplicate tool_id in registry: {key} from {source}"
    );
}

fn mcp_tool_entry(spec: McpToolSpec) -> ToolRegistryEntry {
    let tool_id = spec.name.to_string();
    ToolRegistryEntry {
        tool_id: tool_id.clone(),
        name: spec.name.to_string(),
        title: spec.title.to_string(),
        description: spec.description.to_string(),
        version: REGISTRY_ENTRY_VERSION.to_string(),
        transport: ToolRegistryTransport::McpStdio,
        route: json!({
            "protocol": "mcp",
            "method": "tools/call",
            "tool": spec.name,
            "rpc_method": spec.rpc_method,
        }),
        input_schema: spec.input_schema,
        output_schema: mcp_output_schema(),
        allowed_agents: vec!["*".to_string()],
        tags: tags_for_tool_id(&tool_id, "mcp"),
        enabled: true,
        health: ToolRegistryHealth::Available,
    }
}

fn controller_tool_entry(schema: &ControllerSchema) -> ToolRegistryEntry {
    let tool_id = schema.method_name();
    ToolRegistryEntry {
        tool_id: tool_id.clone(),
        name: tool_id.clone(),
        title: title_from_function(schema.function),
        description: schema.description.to_string(),
        version: REGISTRY_ENTRY_VERSION.to_string(),
        transport: ToolRegistryTransport::JsonRpc,
        route: json!({
            "protocol": "json_rpc",
            "method": all::rpc_method_name(schema),
            "controller": schema.method_name(),
        }),
        input_schema: schema_fields_to_json_schema(&schema.inputs),
        output_schema: schema_fields_to_json_schema(&schema.outputs),
        allowed_agents: vec!["*".to_string()],
        tags: tags_for_tool_id(&tool_id, "controller"),
        enabled: true,
        health: ToolRegistryHealth::Available,
    }
}

fn schema_fields_to_json_schema(fields: &[FieldSchema]) -> Value {
    let mut properties = Map::new();
    let mut required = Vec::new();

    for field in fields {
        properties.insert(field.name.to_string(), field_schema_to_json(field));
        if field.required {
            required.push(Value::String(field.name.to_string()));
        }
    }

    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false,
    })
}

fn field_schema_to_json(field: &FieldSchema) -> Value {
    let mut schema = type_schema_to_json(&field.ty);
    match schema.as_object_mut() {
        Some(object) => {
            object.insert(
                "description".to_string(),
                Value::String(field.comment.to_string()),
            );
        }
        None => {
            schema = json!({
                "description": field.comment,
                "anyOf": [schema],
            });
        }
    }
    schema
}

fn type_schema_to_json(ty: &TypeSchema) -> Value {
    match ty {
        TypeSchema::Bool => json!({ "type": "boolean" }),
        TypeSchema::I64 | TypeSchema::U64 => json!({ "type": "integer" }),
        TypeSchema::F64 => json!({ "type": "number" }),
        TypeSchema::String => json!({ "type": "string" }),
        TypeSchema::Json => json!({}),
        TypeSchema::Bytes => json!({ "type": "string", "contentEncoding": "base64" }),
        TypeSchema::Array(inner) => json!({
            "type": "array",
            "items": type_schema_to_json(inner),
        }),
        TypeSchema::Map(inner) => json!({
            "type": "object",
            "additionalProperties": type_schema_to_json(inner),
        }),
        TypeSchema::Option(inner) => json!({
            "anyOf": [
                type_schema_to_json(inner),
                { "type": "null" }
            ],
        }),
        TypeSchema::Enum { variants } => json!({
            "type": "string",
            "enum": variants,
        }),
        TypeSchema::Object { fields } => schema_fields_to_json_schema(fields),
        TypeSchema::Ref(name) => json!({
            "$ref": format!("#/$defs/{name}"),
        }),
    }
}

fn mcp_output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "content": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": true
                }
            },
            "isError": { "type": "boolean" }
        },
        "additionalProperties": true,
    })
}

fn tags_for_tool_id(tool_id: &str, source: &str) -> Vec<String> {
    let mut tags = vec![source.to_string()];
    if let Some(namespace) = tool_id.split('.').next() {
        push_unique(&mut tags, namespace);
    }
    if tool_id.contains("search") || tool_id.contains("recall") {
        push_unique(&mut tags, "retrieval");
    }
    if tool_id.contains("memory") || tool_id.contains("tree") {
        push_unique(&mut tags, "memory");
    }
    tags
}

fn push_unique(tags: &mut Vec<String>, tag: &str) {
    if !tag.is_empty() && !tags.iter().any(|existing| existing == tag) {
        tags.push(tag.to_string());
    }
}

fn title_from_function(function: &str) -> String {
    function
        .split('_')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{FieldSchema, TypeSchema};

    #[test]
    fn registry_entries_include_mcp_and_controller_tools() {
        let entries = registry_entries();

        let memory_search = entries
            .iter()
            .find(|entry| entry.tool_id == "memory.search")
            .expect("memory.search mcp tool");
        assert_eq!(memory_search.transport, ToolRegistryTransport::McpStdio);
        assert_eq!(memory_search.route["method"], json!("tools/call"));
        assert_eq!(memory_search.health, ToolRegistryHealth::Available);

        let web_search = entries
            .iter()
            .find(|entry| entry.tool_id == "tools.web_search")
            .expect("tools.web_search controller tool");
        assert_eq!(web_search.transport, ToolRegistryTransport::JsonRpc);
        assert_eq!(
            web_search.route["method"],
            json!("openhuman.tools_web_search")
        );
        assert_eq!(web_search.input_schema["type"], json!("object"));
    }

    #[test]
    fn registry_entries_are_unique_and_sorted_by_tool_id() {
        let entries = registry_entries();
        let ids = entries
            .iter()
            .map(|entry| entry.tool_id.as_str())
            .collect::<Vec<_>>();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        sorted.dedup();

        assert_eq!(ids, sorted);
    }

    #[test]
    #[should_panic(expected = "duplicate tool_id in registry")]
    fn insert_registry_entry_panics_on_duplicate_tool_id() {
        let mut entries = BTreeMap::new();
        let entry = ToolRegistryEntry {
            tool_id: "duplicate.tool".to_string(),
            name: "duplicate.tool".to_string(),
            title: "Duplicate Tool".to_string(),
            description: "Test duplicate entry.".to_string(),
            version: REGISTRY_ENTRY_VERSION.to_string(),
            transport: ToolRegistryTransport::JsonRpc,
            route: json!({}),
            input_schema: json!({}),
            output_schema: json!({}),
            allowed_agents: vec!["*".to_string()],
            tags: vec!["test".to_string()],
            enabled: true,
            health: ToolRegistryHealth::Available,
        };

        insert_registry_entry(&mut entries, entry.clone(), "first");
        insert_registry_entry(&mut entries, entry, "second");
    }

    #[test]
    fn get_tool_trims_and_returns_exact_entry() {
        let outcome = get_tool("  memory.search  ").expect("registry lookup");
        assert_eq!(outcome.value.tool_id, "memory.search");
    }

    #[test]
    fn get_tool_rejects_blank_id() {
        let err = get_tool("  ").expect_err("blank id should fail");
        assert!(err.contains("non-empty"));
    }

    #[test]
    fn get_tool_reports_unknown_id() {
        let err = get_tool("missing.tool").expect_err("unknown id should fail");
        assert!(err.contains("missing.tool"));
    }

    #[test]
    fn controller_json_schema_marks_required_and_optional_fields() {
        let schema = schema_fields_to_json_schema(&[
            FieldSchema {
                name: "query",
                ty: TypeSchema::String,
                comment: "Query text.",
                required: true,
            },
            FieldSchema {
                name: "max_results",
                ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                comment: "Optional cap.",
                required: false,
            },
        ]);

        assert_eq!(schema["required"], json!(["query"]));
        assert_eq!(schema["properties"]["query"]["type"], json!("string"));
        assert_eq!(
            schema["properties"]["max_results"]["anyOf"][0]["type"],
            json!("integer")
        );
        assert_eq!(
            schema["properties"]["max_results"]["description"],
            json!("Optional cap.")
        );
    }
}
