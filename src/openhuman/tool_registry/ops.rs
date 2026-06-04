use std::collections::{BTreeMap, BTreeSet};

use serde_json::{json, Map, Value};

use crate::core::all;
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::Config;
use crate::openhuman::mcp_server::McpToolSpec;
use crate::openhuman::memory_store::chunks::store as chunk_store;
use crate::rpc::RpcOutcome;

use super::providers::capability_provider_diagnostics;
use super::types::{
    McpAllowlistDiagnostics, McpServerAllowlistSummary, McpWriteAuditHealth, ToolPolicyDiagnostics,
    ToolPolicyPosture, ToolRegistryEntry, ToolRegistryHealth, ToolRegistryList,
    ToolRegistryTransport,
};

const REGISTRY_ENTRY_VERSION: &str = env!("CARGO_PKG_VERSION");
const POLICY_SURFACES: &[&str] = &[
    "security.policy_info",
    "approval.list_pending",
    "approval.list_recent_decisions",
    "approval.decide",
    "tool_registry.list",
    "tool_registry.get",
    "tool_registry.diagnostics",
];

/// Return the current read-only tool registry snapshot.
pub fn list_tools() -> RpcOutcome<ToolRegistryList> {
    let tools = registry_entries();
    log::debug!(
        "[tool_registry] list_tools completed entries={}",
        tools.len()
    );
    RpcOutcome::new(ToolRegistryList { tools }, vec![])
}

/// Return redacted diagnostics for policy/tool visibility reviews.
pub async fn diagnostics() -> Result<RpcOutcome<ToolPolicyDiagnostics>, String> {
    log::debug!("[tool_registry] diagnostics loading_config");
    let config = Config::load_or_init().await.map_err(|err| {
        log::warn!("[tool_registry] diagnostics config_load_failed error={err}");
        format!("failed to load config for tool registry diagnostics: {err}")
    })?;
    Ok(diagnostics_for_config(&config))
}

/// Return redacted diagnostics using a specific config snapshot.
pub fn diagnostics_for_config(config: &Config) -> RpcOutcome<ToolPolicyDiagnostics> {
    log::debug!("[tool_registry] diagnostics_for_config start");

    let tools = registry_entries();
    let total_tools = tools.len();
    let enabled_tools = tools.iter().filter(|entry| entry.enabled).count();
    let mcp_stdio_tools = tools
        .iter()
        .filter(|entry| entry.transport == ToolRegistryTransport::McpStdio)
        .count();
    let json_rpc_tools = tools
        .iter()
        .filter(|entry| entry.transport == ToolRegistryTransport::JsonRpc)
        .count();
    let possible_write_surfaces = tools
        .iter()
        .filter(|entry| looks_write_capable(&entry.tool_id))
        .map(|entry| entry.tool_id.clone())
        .collect::<Vec<_>>();
    let policy_surfaces = policy_surface_ids();
    let posture = posture_from_config(config);
    let mcp_allowlists = mcp_allowlists_from_config(config);
    let mcp_write_audit = mcp_write_audit_health(config);
    let recent_denials = super::denials::list(25);
    let capability_providers = capability_provider_diagnostics(config);

    log::trace!(
        "[tool_registry] diagnostics_for_config counted total_tools={} enabled_tools={} mcp_stdio_tools={} json_rpc_tools={} possible_write_surfaces={} policy_surfaces={}",
        total_tools,
        enabled_tools,
        mcp_stdio_tools,
        json_rpc_tools,
        possible_write_surfaces.len(),
        policy_surfaces.len()
    );

    let diagnostics = ToolPolicyDiagnostics {
        total_tools,
        enabled_tools,
        mcp_stdio_tools,
        json_rpc_tools,
        possible_write_surfaces,
        policy_surfaces,
        posture,
        mcp_allowlists,
        mcp_write_audit,
        recent_denials,
        capability_providers,
    };
    log::debug!(
        "[tool_registry] diagnostics_for_config completed total_tools={} enabled_tools={} mcp_stdio_tools={} json_rpc_tools={} possible_write_surfaces={} policy_surfaces={} providers_total={} providers_enabled={} providers_trusted={} providers_trusted_enabled={} provider_errors={}",
        diagnostics.total_tools,
        diagnostics.enabled_tools,
        diagnostics.mcp_stdio_tools,
        diagnostics.json_rpc_tools,
        diagnostics.possible_write_surfaces.len(),
        diagnostics.policy_surfaces.len(),
        diagnostics.capability_providers.total_providers,
        diagnostics.capability_providers.enabled_providers,
        diagnostics.capability_providers.trusted_providers,
        diagnostics.capability_providers.trusted_enabled_providers,
        diagnostics.capability_providers.registry_errors.len()
    );
    RpcOutcome::new(diagnostics, vec![])
}

fn posture_from_config(config: &Config) -> ToolPolicyPosture {
    ToolPolicyPosture {
        autonomy_level: format!("{:?}", config.autonomy.level).to_lowercase(),
        workspace_only: config.autonomy.workspace_only,
        max_actions_per_hour: config.autonomy.max_actions_per_hour,
        require_approval_for_medium_risk: config.autonomy.require_approval_for_medium_risk,
        block_high_risk_commands: config.autonomy.block_high_risk_commands,
    }
}

fn mcp_allowlists_from_config(config: &Config) -> McpAllowlistDiagnostics {
    let enabled = config.mcp_client.enabled;
    let server_count = config.mcp_client.servers.len();
    let mut enabled_server_count = 0;
    let mut servers = Vec::new();
    for server in &config.mcp_client.servers {
        if server.enabled {
            enabled_server_count += 1;
        }
        servers.push(McpServerAllowlistSummary {
            name: server.name.clone(),
            enabled: server.enabled,
            allowed_tools_count: server.allowed_tools.len(),
            disallowed_tools_count: server.disallowed_tools.len(),
            has_allowlist: !server.allowed_tools.is_empty(),
            has_denylist: !server.disallowed_tools.is_empty(),
        });
    }
    McpAllowlistDiagnostics {
        enabled,
        server_count,
        enabled_server_count,
        servers,
    }
}

fn mcp_write_audit_health(config: &Config) -> McpWriteAuditHealth {
    let result = chunk_store::with_connection(config, |conn| {
        let since_ms = chrono::Utc::now()
            .timestamp_millis()
            .saturating_sub(24 * 60 * 60 * 1000);
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM mcp_writes WHERE timestamp_ms >= ?1",
            rusqlite::params![since_ms],
            |row| row.get::<_, i64>(0),
        )?;
        Ok(u64::try_from(count).unwrap_or(0))
    });

    match result {
        Ok(count) => McpWriteAuditHealth {
            enabled: true,
            recent_rows: Some(count),
            last_error: None,
        },
        Err(err) => McpWriteAuditHealth {
            enabled: true,
            recent_rows: None,
            last_error: Some(err.to_string()),
        },
    }
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
///
/// This includes:
/// 1. MCP stdio server tools (existing `mcp_server` surface)
/// 2. Controller-backed tools (existing `tools` namespace)
/// 3. Connected MCP client server tools (new `mcp_clients` domain)
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

    // Enumerate tools from all currently-connected MCP client servers.
    // `block_in_place` requires the multi-threaded tokio runtime; fall back
    // silently to an empty list in single-threaded contexts (e.g. unit tests).
    let client_tools = {
        use crate::openhuman::mcp_registry::connections;
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                // Only use block_in_place when we are on the multi-threaded
                // runtime (kind = MultiThread). The current-thread runtime
                // (kind = CurrentThread) panics on block_in_place.
                if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread {
                    tokio::task::block_in_place(|| {
                        handle.block_on(connections::all_connected_tools())
                    })
                } else {
                    Vec::new()
                }
            }
            Err(_) => Vec::new(),
        }
    };

    for (server_id, _qualified_name_placeholder, tool) in client_tools {
        let tool_id = format!("mcp-client::{server_id}::{}", tool.name);
        let entry = ToolRegistryEntry {
            tool_id: tool_id.clone(),
            name: tool.name.clone(),
            title: title_from_function(&tool.name),
            description: tool.description.unwrap_or_default(),
            version: REGISTRY_ENTRY_VERSION.to_string(),
            transport: ToolRegistryTransport::McpStdio,
            route: json!({
                "protocol": "mcp-client",
                "rpc_method": "openhuman.mcp_clients_tool_call",
                "server_id": server_id,
                "tool_name": tool.name,
            }),
            input_schema: tool.input_schema,
            output_schema: mcp_output_schema(),
            allowed_agents: vec!["*".to_string()],
            tags: tags_for_tool_id(&tool_id, "mcp_client"),
            enabled: true,
            health: ToolRegistryHealth::Available,
        };
        insert_registry_entry(&mut entries, entry, "mcp_client");
    }

    entries.into_values().collect()
}

fn insert_registry_entry(
    entries: &mut BTreeMap<String, ToolRegistryEntry>,
    entry: ToolRegistryEntry,
    source: &str,
) {
    let key = entry.tool_id.clone();
    if entries.contains_key(&key) {
        // Duplicate tool IDs can arrive from external MCP servers that reuse
        // well-known names.  First-write-wins: log and skip the duplicate
        // rather than panicking or silently overwriting in production.
        log::warn!(
            "[tool_registry] duplicate tool_id={} from source={}; skipping",
            key,
            source
        );
        return;
    }
    entries.insert(key, entry);
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

fn looks_write_capable(tool_id: &str) -> bool {
    const MARKERS: &[&str] = &[
        "add", "apply", "create", "decide", "delete", "email", "execute", "forget", "ingest",
        "post", "put", "remove", "run", "send", "store", "update", "write",
    ];
    let lower = tool_id.to_ascii_lowercase();
    MARKERS.iter().any(|marker| {
        lower == *marker
            || lower.contains(&format!(".{marker}"))
            || lower.contains(&format!("_{marker}"))
            || lower.contains(&format!("{marker}."))
            || lower.contains(&format!("{marker}_"))
    })
}

fn policy_surface_ids() -> Vec<String> {
    let mut ids = POLICY_SURFACES
        .iter()
        .copied()
        .map(String::from)
        .collect::<BTreeSet<_>>();

    ids.extend(
        all::all_controller_schemas()
            .into_iter()
            .map(|schema| schema.method_name())
            .filter(|tool_id| is_policy_surface(tool_id)),
    );

    ids.into_iter().collect()
}

fn is_policy_surface(tool_id: &str) -> bool {
    POLICY_SURFACES.contains(&tool_id)
        || tool_id.starts_with("security.")
        || tool_id.starts_with("approval.")
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
#[path = "ops_tests.rs"]
mod tests;
