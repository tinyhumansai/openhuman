use serde::Serialize;
use serde_json::Value;

/// Serialized discovery metadata for one OpenHuman tool surface.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ToolRegistryEntry {
    /// Stable unique registry id, such as `memory.search` or `tools.web_search`.
    pub tool_id: String,
    /// Machine-readable tool name exposed by the source surface.
    pub name: String,
    /// Human-readable display title.
    pub title: String,
    /// Short description suitable for agents and dashboards.
    pub description: String,
    /// Registry entry schema/version marker, currently the core crate version.
    pub version: String,
    /// Transport used to call the tool.
    pub transport: ToolRegistryTransport,
    /// Transport-specific route metadata.
    pub route: Value,
    /// JSON Schema for accepted input parameters.
    pub input_schema: Value,
    /// JSON Schema for the successful output shape.
    pub output_schema: Value,
    /// Agent ids allowed to discover/use the tool; `*` means unrestricted in this MVP.
    pub allowed_agents: Vec<String>,
    /// Search/filter tags derived from the source namespace and tool purpose.
    pub tags: Vec<String>,
    /// Whether the tool is currently enabled in the static registry.
    pub enabled: bool,
    /// Current health state for discovery consumers.
    pub health: ToolRegistryHealth,
}

/// Transport family used to route a registry entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolRegistryTransport {
    /// Existing HTTP JSON-RPC controller method.
    JsonRpc,
    /// Existing stdio Model Context Protocol `tools/call` surface.
    McpStdio,
}

/// Health state exposed by the registry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolRegistryHealth {
    /// The tool is statically registered and available for discovery.
    Available,
    /// Health cannot currently be determined.
    Unknown,
}

/// Response payload for `openhuman.tool_registry_list`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ToolRegistryList {
    /// Sorted registry entries.
    pub tools: Vec<ToolRegistryEntry>,
}

/// Redacted diagnostics for policy/tool visibility reviews.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ToolPolicyDiagnostics {
    pub total_tools: usize,
    pub enabled_tools: usize,
    pub mcp_stdio_tools: usize,
    pub json_rpc_tools: usize,
    pub possible_write_surfaces: Vec<String>,
    pub policy_surfaces: Vec<String>,
}
