//! Runtime-generated tool wrappers.
//!
//! This module gives trusted profile/runtime layers a narrow way to
//! expose generated capability tools without adding a bespoke Rust type
//! for each tool and without handing the model a broad raw bridge.

use std::collections::BTreeSet;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolCategory, ToolResult, ToolScope};

#[derive(Debug, Clone)]
pub struct GeneratedToolDefinition {
    /// Stable tool name exposed to the model.
    pub name: String,
    /// Human-readable tool description.
    pub description: String,
    /// JSON schema for tool arguments.
    pub parameters_schema: Value,
    /// Permission required to execute this tool.
    pub permission_level: PermissionLevel,
    /// Tool category used for agent/tool scoping.
    pub category: ToolCategory,
    /// Execution surface where the tool is available.
    pub scope: ToolScope,
    /// Adapter responsible for executing the generated tool.
    pub adapter_id: String,
    /// Provider that produced this generated tool.
    pub provider_id: Option<String>,
    /// Provider-scoped capability id for policy and revocation.
    pub capability_id: Option<String>,
    /// Digest of the source capability definition.
    pub source_digest: Option<String>,
    /// Declared runtime risk for policy and approval.
    pub risk: Option<GeneratedToolRisk>,
    /// Optional policy namespace/surface label.
    pub policy_surface: Option<String>,
}

impl GeneratedToolDefinition {
    /// Build a generated tool definition with legacy-safe defaults.
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
            provider_id: None,
            capability_id: None,
            source_digest: None,
            risk: None,
            policy_surface: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GeneratedToolRisk {
    /// Read-only capability.
    Read,
    /// Local or internal write capability.
    Write,
    /// Externally observable write capability.
    ExternalWrite,
    /// Code or command execution capability.
    Execute,
    /// High-risk or destructive capability.
    Dangerous,
}

impl GeneratedToolRisk {
    fn is_external_effect(self) -> bool {
        matches!(self, Self::ExternalWrite | Self::Execute | Self::Dangerous)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GeneratedToolAdmissionConfig {
    /// Whether provenance fields are required for admission.
    pub enforce_provenance: bool,
    /// Provider ids allowed to register generated tools.
    ///
    /// Values are normalized with the same provider-id rules used for
    /// generated tool definitions before admission checks run.
    pub trusted_providers: BTreeSet<String>,
    /// Provider ids blocked from registration.
    ///
    /// Values are normalized with the same provider-id rules used for
    /// generated tool definitions before admission checks run.
    pub disabled_providers: BTreeSet<String>,
    /// Capability ids blocked from registration.
    pub disabled_capabilities: BTreeSet<String>,
    /// Existing tool names reserved before this admission pass.
    pub existing_tool_names: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedToolAdmissionRejection {
    /// Tool name rejected during admission.
    pub tool_name: String,
    /// Human-readable rejection reason.
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct GeneratedToolAdmissionReport {
    /// Definitions accepted for registration.
    pub admitted: Vec<GeneratedToolDefinition>,
    /// Definitions rejected before registration.
    pub rejected: Vec<GeneratedToolAdmissionRejection>,
}

#[async_trait]
pub trait GeneratedToolAdapter: Send + Sync {
    /// Stable adapter id matched against generated definitions.
    fn id(&self) -> &str;

    /// Execute a generated tool definition with validated arguments.
    async fn execute(
        &self,
        definition: &GeneratedToolDefinition,
        args: Value,
    ) -> anyhow::Result<ToolResult>;
}

/// Executable wrapper around a generated tool definition and adapter.
pub struct GeneratedTool {
    definition: GeneratedToolDefinition,
    adapter: Arc<dyn GeneratedToolAdapter>,
}

impl GeneratedTool {
    /// Create a generated tool wrapper after validation.
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

    /// Borrow the normalized generated tool definition.
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

    fn external_effect(&self) -> bool {
        self.definition
            .risk
            .map(GeneratedToolRisk::is_external_effect)
            .unwrap_or(false)
    }
}

/// Convert generated definitions into boxed tool trait objects.
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

/// Admit generated tool definitions according to provenance policy.
pub fn admit_generated_tool_definitions(
    definitions: Vec<GeneratedToolDefinition>,
    config: &GeneratedToolAdmissionConfig,
) -> GeneratedToolAdmissionReport {
    let mut seen = config.existing_tool_names.clone();
    let mut admitted = Vec::new();
    let mut rejected = Vec::new();

    // Pre-normalize provider allow/deny sets once before the admission loop
    // so we do not redo the O(N) normalization work per tool.
    let normalized_disabled_providers =
        normalize_provider_set(&config.disabled_providers, "disabled_providers");
    let normalized_trusted_providers =
        normalize_provider_set(&config.trusted_providers, "trusted_providers");

    for mut definition in definitions {
        normalize_definition(&mut definition);
        let tool_name = definition.name.clone();
        match validate_admission(
            &definition,
            config,
            &normalized_disabled_providers,
            &normalized_trusted_providers,
            &mut seen,
        ) {
            Ok(()) => {
                log::debug!(
                    "[generated_tools] admission accepted tool_name={} provider_id={:?} capability_id={:?}",
                    definition.name,
                    definition.provider_id,
                    definition.capability_id
                );
                admitted.push(definition);
            }
            Err(reason) => {
                log::debug!(
                    "[generated_tools] admission rejected tool_name={} provider_id={:?} capability_id={:?} reason={}",
                    tool_name,
                    definition.provider_id,
                    definition.capability_id,
                    reason
                );
                rejected.push(GeneratedToolAdmissionRejection { tool_name, reason });
            }
        }
    }

    GeneratedToolAdmissionReport { admitted, rejected }
}

fn normalize_definition(definition: &mut GeneratedToolDefinition) {
    definition.name = definition.name.trim().to_string();
    definition.description = definition.description.trim().to_string();
    definition.adapter_id = definition.adapter_id.trim().to_string();
    definition.provider_id = normalize_optional_provider_id(definition.provider_id.take());
    definition.capability_id = trim_option(definition.capability_id.take());
    definition.source_digest = trim_option(definition.source_digest.take());
    definition.policy_surface = trim_option(definition.policy_surface.take());
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

fn validate_admission(
    definition: &GeneratedToolDefinition,
    config: &GeneratedToolAdmissionConfig,
    normalized_disabled_providers: &BTreeSet<String>,
    normalized_trusted_providers: &BTreeSet<String>,
    seen: &mut BTreeSet<String>,
) -> Result<(), String> {
    validate_definition(definition).map_err(|err| err.to_string())?;
    if !is_safe_generated_tool_name(&definition.name) {
        return Err(format!(
            "generated tool `{}` name contains unsupported characters",
            definition.name
        ));
    }
    if !seen.insert(definition.name.clone()) {
        return Err(format!("duplicate generated tool `{}`", definition.name));
    }
    if !config.enforce_provenance {
        return Ok(());
    }

    let provider_id = definition
        .provider_id
        .as_deref()
        .ok_or_else(|| format!("generated tool `{}` missing provider_id", definition.name))?;
    if normalize_provider_id(provider_id).is_none() {
        return Err(format!(
            "generated tool `{}` has invalid provider_id `{provider_id}`",
            definition.name
        ));
    }
    if normalized_disabled_providers.contains(provider_id) {
        return Err(format!(
            "generated tool `{}` provider `{provider_id}` is disabled",
            definition.name
        ));
    }
    if !normalized_trusted_providers.contains(provider_id) {
        return Err(format!(
            "generated tool `{}` provider `{provider_id}` is not trusted",
            definition.name
        ));
    }

    let capability_id = definition
        .capability_id
        .as_deref()
        .ok_or_else(|| format!("generated tool `{}` missing capability_id", definition.name))?;
    if config.disabled_capabilities.contains(capability_id) {
        return Err(format!(
            "generated tool `{}` capability `{capability_id}` is disabled",
            definition.name
        ));
    }

    if definition.risk.is_none() {
        return Err(format!(
            "generated tool `{}` missing risk metadata",
            definition.name
        ));
    }
    if definition.source_digest.is_none() {
        return Err(format!(
            "generated tool `{}` missing source_digest",
            definition.name
        ));
    }

    Ok(())
}

fn trim_option(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn normalize_optional_provider_id(value: Option<String>) -> Option<String> {
    trim_option(value).map(|value| normalize_provider_id(&value).unwrap_or(value))
}

fn normalize_provider_set(values: &BTreeSet<String>, field: &str) -> BTreeSet<String> {
    values
        .iter()
        .filter_map(|value| {
            let normalized = normalize_provider_id(value);
            if normalized.is_none() {
                log::debug!(
                    "[generated_tools] dropped invalid provider_id from config field={field} value={value}"
                );
            }
            normalized
        })
        .collect()
}

fn normalize_provider_id(value: &str) -> Option<String> {
    let normalized = value.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return None;
    }
    let valid = normalized
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '-' | '_' | '.'));
    if !valid {
        return None;
    }
    let starts_or_ends_with_sep = normalized
        .chars()
        .next()
        .zip(normalized.chars().last())
        .map(|(first, last)| is_provider_separator(first) || is_provider_separator(last))
        .unwrap_or(true);
    if starts_or_ends_with_sep {
        return None;
    }
    Some(normalized)
}

fn is_provider_separator(ch: char) -> bool {
    matches!(ch, '-' | '_' | '.')
}

fn is_safe_generated_tool_name(name: &str) -> bool {
    let trimmed = name.trim();
    !trimmed.is_empty()
        && !trimmed.starts_with(['.', '-', '_'])
        && !trimmed.ends_with(['.', '-', '_'])
        && trimmed.chars().all(|ch| {
            ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '.' | '-' | '_')
        })
}

#[cfg(test)]
#[path = "generated_tests.rs"]
mod tests;
