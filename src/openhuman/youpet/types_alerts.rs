use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CoreAlertSeverity {
    Low,
    Medium,
    High,
    Critical,
}

impl CoreAlertSeverity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Critical => "critical",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CoreAlertStatus {
    Open,
    Acknowledged,
    Resolved,
    Dismissed,
}

impl CoreAlertStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Acknowledged => "acknowledged",
            Self::Resolved => "resolved",
            Self::Dismissed => "dismissed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CoreAlertStatusFilter {
    #[default]
    Omitted,
    All,
    Status(CoreAlertStatus),
}

impl CoreAlertStatusFilter {
    pub fn as_query_param(self) -> Option<&'static str> {
        match self {
            Self::Omitted => None,
            Self::All => Some(""),
            Self::Status(status) => Some(status.as_str()),
        }
    }
}

impl<'de> Deserialize<'de> for CoreAlertStatusFilter {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        match value {
            Value::Null => Ok(Self::All),
            Value::String(raw) => {
                if raw.trim().is_empty() {
                    Ok(Self::All)
                } else {
                    serde_json::from_value::<CoreAlertStatus>(Value::String(raw))
                        .map(Self::Status)
                        .map_err(serde::de::Error::custom)
                }
            }
            other => Err(serde::de::Error::custom(format!(
                "status must be null or a string, got {other}"
            ))),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CoreWorkbenchAlertContext {
    pub pet: CoreWorkbenchPetContext,
    pub owner: CoreWorkbenchOwnerContext,
    pub health_plan: CoreWorkbenchHealthPlanContext,
    pub task: CoreWorkbenchTaskContext,
    #[serde(default)]
    pub latest_checkin: Option<CoreWorkbenchLatestCheckinContext>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CoreWorkbenchPetContext {
    pub id: String,
    pub name: String,
    pub species: String,
    #[serde(default)]
    pub breed: Option<String>,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CoreWorkbenchOwnerContext {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub phone: Option<String>,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CoreWorkbenchHealthPlanContext {
    pub id: String,
    pub title: String,
    pub plan_type: String,
    pub status: String,
    #[serde(default)]
    pub openclaw_flow_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CoreWorkbenchTaskContext {
    pub id: String,
    pub status: String,
    pub due_at: String,
    pub missed_count: i64,
    #[serde(default)]
    pub openclaw_flow_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CoreWorkbenchLatestCheckinContext {
    pub id: String,
    pub submitted_at: String,
    #[serde(default)]
    pub submitted_by: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub status_tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CoreWorkbenchAlert {
    pub id: String,
    pub alert_type: String,
    pub severity: CoreAlertSeverity,
    pub related_type: String,
    pub related_id: String,
    pub status: CoreAlertStatus,
    #[serde(default)]
    pub assigned_to: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    pub created_at: String,
    #[serde(default)]
    pub acknowledged_at: Option<String>,
    #[serde(default)]
    pub resolved_at: Option<String>,
    #[serde(default)]
    pub context: Option<CoreWorkbenchAlertContext>,
}

#[derive(Debug, Clone)]
pub struct CoreWorkbenchAlertsResponse {
    pub items: Vec<CoreWorkbenchAlert>,
}

impl<'de> Deserialize<'de> for CoreWorkbenchAlertsResponse {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct RawResponse {
            items: Vec<Value>,
        }

        let raw = RawResponse::deserialize(deserializer)?;
        let mut items = Vec::with_capacity(raw.items.len());
        for item in raw.items {
            let has_context = item
                .as_object()
                .is_some_and(|object| object.contains_key("context"));
            if !has_context {
                return Err(serde::de::Error::custom(
                    "listed Core workbench alerts must include context (nullable)",
                ));
            }
            items.push(serde_json::from_value(item).map_err(serde::de::Error::custom)?);
        }
        Ok(Self { items })
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ListAlertsRpcParams {
    #[serde(default)]
    pub status: CoreAlertStatusFilter,
    #[serde(default)]
    pub severity: Option<CoreAlertSeverity>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlertActionRpcParams {
    pub alert_id: String,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub resolution: Option<String>,
    #[serde(default)]
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TraceAlertRpcParams {
    pub alert_id: String,
}
