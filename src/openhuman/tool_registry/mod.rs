//! Unified read-only tool registry for discovery across OpenHuman tool surfaces.

pub mod denials;
pub mod ops;
mod providers;
mod schemas;
mod types;

pub use ops::{get_tool, list_tools, registry_entries};
pub use providers::{
    capability_provider_by_id, capability_provider_diagnostics, capability_provider_registry,
    is_capability_provider_trusted_enabled, list_capability_providers,
    normalize_capability_provider_id, CapabilityProviderMetadata, CapabilityProviderRegistry,
    CapabilityProviderRegistryError,
};
pub use schemas::{
    all_controller_schemas as all_tool_registry_controller_schemas,
    all_registered_controllers as all_tool_registry_registered_controllers,
};
pub use types::{
    CapabilityProviderDiagnostics, McpAllowlistDiagnostics, McpServerAllowlistSummary,
    McpWriteAuditHealth, RecentPolicyDenial, ToolPolicyDiagnostics, ToolPolicyPosture,
    ToolRegistryEntry, ToolRegistryHealth, ToolRegistryList, ToolRegistryTransport,
};
