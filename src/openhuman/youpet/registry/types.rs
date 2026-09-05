use base64::Engine as _;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::openhuman::youpet::invalid_request_error;

const DEFAULT_REGISTRY_LIMIT: i64 = 50;
const MAX_REGISTRY_LIMIT: i64 = 200;
const MAX_REGISTRY_KEY_LEN: usize = 128;

fn deserialize_json_object<'de, D>(
    deserializer: D,
    field_name: &'static str,
) -> Result<Value, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    if value.is_object() {
        Ok(value)
    } else {
        Err(serde::de::Error::custom(format!(
            "{field_name} must be a JSON object"
        )))
    }
}

fn deserialize_active_agent_lifecycle<'de, D>(
    deserializer: D,
) -> Result<AgentRegistryLifecycleState, D::Error>
where
    D: Deserializer<'de>,
{
    let value = AgentRegistryLifecycleState::deserialize(deserializer)?;
    if value == AgentRegistryLifecycleState::Active {
        Ok(value)
    } else {
        Err(serde::de::Error::custom(
            "lifecycle_state must be active in agent registry summaries",
        ))
    }
}

fn deserialize_active_tool_definition_lifecycle<'de, D>(
    deserializer: D,
) -> Result<ToolDefinitionLifecycleState, D::Error>
where
    D: Deserializer<'de>,
{
    let value = ToolDefinitionLifecycleState::deserialize(deserializer)?;
    if value == ToolDefinitionLifecycleState::Active {
        Ok(value)
    } else {
        Err(serde::de::Error::custom(
            "lifecycle_state must be active in tool definition summaries",
        ))
    }
}

fn deserialize_input_schema<'de, D>(deserializer: D) -> Result<Value, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_json_object(deserializer, "input_schema")
}

fn deserialize_output_schema<'de, D>(deserializer: D) -> Result<Value, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_json_object(deserializer, "output_schema")
}

fn deserialize_timeout_defaults<'de, D>(deserializer: D) -> Result<Value, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_json_object(deserializer, "timeout_defaults")
}

fn deserialize_retry_contract<'de, D>(deserializer: D) -> Result<Value, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_json_object(deserializer, "retry_contract")
}

fn deserialize_audit_contract<'de, D>(deserializer: D) -> Result<Value, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_json_object(deserializer, "audit_contract")
}

fn deserialize_delivery_behavior<'de, D>(deserializer: D) -> Result<Value, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_json_object(deserializer, "delivery_behavior")
}

fn deserialize_owner_actor_type<'de, D>(deserializer: D) -> Result<RegistryOwnerActorType, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    match value.as_str() {
        "service" => Ok(RegistryOwnerActorType::Service),
        "user" => Ok(RegistryOwnerActorType::User),
        _ => Err(serde::de::Error::custom(
            "actor_type must be one of: service, user",
        )),
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RegistryCursorListResponse<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
}

impl<'de, T> Deserialize<'de> for RegistryCursorListResponse<T>
where
    T: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct RawCursorListResponse<T> {
            items: Vec<T>,
            next_cursor: Value,
        }

        let raw = RawCursorListResponse::deserialize(deserializer)?;
        let next_cursor = match raw.next_cursor {
            Value::Null => None,
            Value::String(cursor) => Some(cursor),
            _ => {
                return Err(serde::de::Error::custom(
                    "next_cursor must be a string or null",
                ));
            }
        };
        Ok(Self {
            items: raw.items,
            next_cursor,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RegistryUnpagedListResponse<T> {
    pub items: Vec<T>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RegistryOwnerActorType {
    Service,
    User,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentRegistryLifecycleState {
    Draft,
    Active,
    Retired,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolDefinitionLifecycleState {
    Draft,
    Active,
    Retired,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolEffectClass {
    ReadOnly,
    Effectful,
    Destructive,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolEnablementLifecycleState {
    Enabled,
    Disabled,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolEnablementAuditMode {
    MetadataOnly,
    RedactedIo,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConnectorTypeLifecycleState {
    Draft,
    Active,
    Retired,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConnectorBindingLifecycleState {
    Draft,
    Active,
    Retired,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentOwnerRef {
    #[serde(deserialize_with = "deserialize_owner_actor_type")]
    pub actor_type: RegistryOwnerActorType,
    pub actor_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolRefV1 {
    pub tool_key: String,
    pub version: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KnowledgeScopeRefV1 {
    pub source_key: String,
    pub trust_version: String,
    pub access_scope: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PolicyRefV1 {
    pub policy_id: String,
    pub policy_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentConfigurationV1 {
    pub schema_version: i64,
    pub domain_key: String,
    pub owner: AgentOwnerRef,
    pub allowed_tool_refs: Vec<ToolRefV1>,
    pub knowledge_scope_refs: Vec<KnowledgeScopeRefV1>,
    pub risk_policy_ref: Option<PolicyRefV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentRegistryAgentSummary {
    pub id: String,
    pub agent_key: String,
    pub version: i64,
    #[serde(deserialize_with = "deserialize_active_agent_lifecycle")]
    pub lifecycle_state: AgentRegistryLifecycleState,
    pub configuration_fingerprint: String,
    pub owner_actor_type: RegistryOwnerActorType,
    pub owner_actor_id: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentRegistryAgent {
    pub id: String,
    pub agent_key: String,
    pub version: i64,
    pub lifecycle_state: AgentRegistryLifecycleState,
    pub configuration: AgentConfigurationV1,
    pub configuration_fingerprint: String,
    pub owner_actor_type: RegistryOwnerActorType,
    pub owner_actor_id: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolRegistryToolDefinitionSummary {
    pub tool_key: String,
    pub version: i64,
    #[serde(deserialize_with = "deserialize_active_tool_definition_lifecycle")]
    pub lifecycle_state: ToolDefinitionLifecycleState,
    pub definition_fingerprint: String,
    pub schema_version: i64,
    pub display_name: String,
    pub description: String,
    pub tool_effect_class: ToolEffectClass,
    pub abstract_auth_scopes: Vec<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolRegistryToolDefinition {
    pub tool_key: String,
    pub version: i64,
    pub lifecycle_state: ToolDefinitionLifecycleState,
    pub definition_fingerprint: String,
    pub schema_version: i64,
    pub display_name: String,
    pub description: String,
    pub tool_effect_class: ToolEffectClass,
    pub abstract_auth_scopes: Vec<String>,
    #[serde(deserialize_with = "deserialize_input_schema")]
    pub input_schema: Value,
    #[serde(deserialize_with = "deserialize_output_schema")]
    pub output_schema: Value,
    #[serde(deserialize_with = "deserialize_timeout_defaults")]
    pub timeout_defaults: Value,
    #[serde(deserialize_with = "deserialize_retry_contract")]
    pub retry_contract: Value,
    #[serde(deserialize_with = "deserialize_audit_contract")]
    pub audit_contract: Value,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolRegistryToolEnablement {
    pub tool_key: String,
    pub version: i64,
    pub lifecycle_state: ToolEnablementLifecycleState,
    pub generation: i64,
    pub timeout_cap_ms: Option<i64>,
    pub approval_required: bool,
    pub allow_ttl_seconds: Option<i64>,
    pub audit_mode: Option<ToolEnablementAuditMode>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConnectorRegistryTypeSummary {
    pub connector_key: String,
    pub version: i64,
    pub lifecycle_state: ConnectorTypeLifecycleState,
    pub source_type: String,
    pub connector_type_fingerprint: String,
    pub capabilities: Vec<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConnectorNormalizationContract {
    pub evidence_family: String,
    pub kernel_event_type: String,
    pub kernel_event_schema_version: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConnectorRegistryType {
    pub connector_key: String,
    pub version: i64,
    pub lifecycle_state: ConnectorTypeLifecycleState,
    pub source_type: String,
    pub connector_type_fingerprint: String,
    pub capabilities: Vec<String>,
    pub normalization_contracts: Vec<ConnectorNormalizationContract>,
    #[serde(deserialize_with = "deserialize_delivery_behavior")]
    pub delivery_behavior: Value,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConnectorRegistryProviderAccount {
    pub namespace: String,
    pub external_account_ref: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConnectorRegistryBindingSummary {
    pub binding_key: String,
    pub version: i64,
    pub lifecycle_state: ConnectorBindingLifecycleState,
    pub connector_type_key: String,
    pub connector_type_version: i64,
    pub connector_type_fingerprint: String,
    pub enabled_capabilities: Vec<String>,
    pub binding_fingerprint: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConnectorRegistryBinding {
    pub binding_key: String,
    pub version: i64,
    pub lifecycle_state: ConnectorBindingLifecycleState,
    pub connector_type_key: String,
    pub connector_type_version: i64,
    pub connector_type_fingerprint: String,
    pub provider_account: ConnectorRegistryProviderAccount,
    pub config_ref: String,
    pub credential_ref: String,
    pub enabled_capabilities: Vec<String>,
    pub binding_fingerprint: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RegistryListAgentsRpcParams {
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub cursor: Option<String>,
}

impl RegistryListAgentsRpcParams {
    pub fn validate(&self) -> Result<(), String> {
        validate_cursor_list_params(self.limit, self.cursor.as_deref(), CursorKind::Agent)
    }

    pub fn limit_or_default(&self) -> i64 {
        self.limit.unwrap_or(DEFAULT_REGISTRY_LIMIT)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RegistryGetAgentVersionRpcParams {
    pub agent_key: String,
    pub version: i64,
}

impl RegistryGetAgentVersionRpcParams {
    pub fn validate(&self) -> Result<(), String> {
        validate_exact_params("agentKey", &self.agent_key, self.version)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RegistryListToolDefinitionsRpcParams {
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub cursor: Option<String>,
}

impl RegistryListToolDefinitionsRpcParams {
    pub fn validate(&self) -> Result<(), String> {
        validate_cursor_list_params(
            self.limit,
            self.cursor.as_deref(),
            CursorKind::ToolDefinition,
        )
    }

    pub fn limit_or_default(&self) -> i64 {
        self.limit.unwrap_or(DEFAULT_REGISTRY_LIMIT)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RegistryGetToolDefinitionVersionRpcParams {
    pub tool_key: String,
    pub version: i64,
}

impl RegistryGetToolDefinitionVersionRpcParams {
    pub fn validate(&self) -> Result<(), String> {
        validate_exact_params("toolKey", &self.tool_key, self.version)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegistryListToolEnablementsRpcParams {}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RegistryGetToolEnablementVersionRpcParams {
    pub tool_key: String,
    pub version: i64,
}

impl RegistryGetToolEnablementVersionRpcParams {
    pub fn validate(&self) -> Result<(), String> {
        validate_exact_params("toolKey", &self.tool_key, self.version)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RegistryListConnectorTypesRpcParams {
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub cursor: Option<String>,
}

impl RegistryListConnectorTypesRpcParams {
    pub fn validate(&self) -> Result<(), String> {
        validate_cursor_list_params(
            self.limit,
            self.cursor.as_deref(),
            CursorKind::ConnectorType,
        )
    }

    pub fn limit_or_default(&self) -> i64 {
        self.limit.unwrap_or(DEFAULT_REGISTRY_LIMIT)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RegistryGetConnectorTypeVersionRpcParams {
    pub connector_key: String,
    pub version: i64,
}

impl RegistryGetConnectorTypeVersionRpcParams {
    pub fn validate(&self) -> Result<(), String> {
        validate_exact_params("connectorKey", &self.connector_key, self.version)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RegistryListConnectorBindingsRpcParams {
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub cursor: Option<String>,
}

impl RegistryListConnectorBindingsRpcParams {
    pub fn validate(&self) -> Result<(), String> {
        validate_cursor_list_params(
            self.limit,
            self.cursor.as_deref(),
            CursorKind::ConnectorBinding,
        )
    }

    pub fn limit_or_default(&self) -> i64 {
        self.limit.unwrap_or(DEFAULT_REGISTRY_LIMIT)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RegistryGetConnectorBindingVersionRpcParams {
    pub binding_key: String,
    pub version: i64,
}

impl RegistryGetConnectorBindingVersionRpcParams {
    pub fn validate(&self) -> Result<(), String> {
        validate_exact_params("bindingKey", &self.binding_key, self.version)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CursorKind {
    Agent,
    ToolDefinition,
    ConnectorType,
    ConnectorBinding,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentCursorEnvelope {
    agent_id: String,
    agent_key: String,
    tenant_id: String,
    v: i64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ToolDefinitionCursorEnvelope {
    definition_id: String,
    tool_key: String,
    v: i64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConnectorCursorEnvelope {
    key: String,
    kind: String,
    schema_version: i64,
    version: i64,
}

fn validate_cursor_list_params(
    limit: Option<i64>,
    cursor: Option<&str>,
    expected_kind: CursorKind,
) -> Result<(), String> {
    validate_limit(limit)?;
    validate_cursor(cursor, expected_kind)
}

fn validate_limit(limit: Option<i64>) -> Result<(), String> {
    if let Some(value) = limit {
        if !(1..=MAX_REGISTRY_LIMIT).contains(&value) {
            return Err(invalid_request_error(
                "limit",
                "limit must be between 1 and 200",
            ));
        }
    }
    Ok(())
}

fn validate_cursor(cursor: Option<&str>, expected_kind: CursorKind) -> Result<(), String> {
    let Some(raw) = cursor.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(());
    };

    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(raw)
        .map_err(|_| invalid_request_error("cursor", "cursor must be base64url JSON"))?;
    match expected_kind {
        CursorKind::Agent => {
            let payload: AgentCursorEnvelope = serde_json::from_slice(&decoded).map_err(|_| {
                invalid_request_error("cursor", "cursor does not match the Agent Registry")
            })?;
            if payload.v != 1
                || payload.agent_key.trim().is_empty()
                || payload.agent_key.len() > MAX_REGISTRY_KEY_LEN
                || !is_non_nil_uuid(&payload.agent_id)
                || !is_non_nil_uuid(&payload.tenant_id)
            {
                return Err(invalid_request_error("cursor", "Agent cursor is invalid"));
            }
        }
        CursorKind::ToolDefinition => {
            let payload: ToolDefinitionCursorEnvelope =
                serde_json::from_slice(&decoded).map_err(|_| {
                    invalid_request_error(
                        "cursor",
                        "cursor does not match the Tool Definition Registry",
                    )
                })?;
            if payload.v != 1
                || payload.tool_key.trim().is_empty()
                || payload.tool_key.len() > MAX_REGISTRY_KEY_LEN
                || !is_non_nil_uuid(&payload.definition_id)
            {
                return Err(invalid_request_error(
                    "cursor",
                    "Tool Definition cursor is invalid",
                ));
            }
        }
        CursorKind::ConnectorType | CursorKind::ConnectorBinding => {
            let payload: ConnectorCursorEnvelope =
                serde_json::from_slice(&decoded).map_err(|_| {
                    invalid_request_error("cursor", "cursor does not match the Connector Registry")
                })?;
            let expected = match expected_kind {
                CursorKind::ConnectorType => "connector_types",
                CursorKind::ConnectorBinding => "connector_bindings",
                _ => unreachable!(),
            };
            if payload.schema_version != 1
                || payload.kind != expected
                || payload.key.trim().is_empty()
                || payload.key.len() > MAX_REGISTRY_KEY_LEN
                || payload.version < 1
            {
                return Err(invalid_request_error(
                    "cursor",
                    "Connector cursor kind or fields are invalid",
                ));
            }
        }
    }
    Ok(())
}

fn is_non_nil_uuid(value: &str) -> bool {
    Uuid::parse_str(value)
        .map(|parsed| !parsed.is_nil())
        .unwrap_or(false)
}

fn validate_exact_params(field: &str, key: &str, version: i64) -> Result<(), String> {
    let trimmed = key.trim();
    if trimmed.is_empty() {
        return Err(invalid_request_error(field, "logical key is required"));
    }
    if trimmed.len() > MAX_REGISTRY_KEY_LEN {
        return Err(invalid_request_error(
            field,
            "logical key must be at most 128 characters",
        ));
    }
    if version < 1 {
        return Err(invalid_request_error("version", "version must be >= 1"));
    }
    Ok(())
}
