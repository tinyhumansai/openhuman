use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkbenchTraceEntryKind {
    AlertCreated,
    HealthPlanState,
    TaskState,
    CheckinReceived,
    ActionRequestProposed,
    ActionRequestApproved,
    ActionRequestRejected,
    ActionRequestExecution,
    AuditAction,
    OutboxEvent,
    OutboxDelivery,
    DeliveryFailed,
    DeliverySucceeded,
    DeliveryRecovered,
    DeliveryDeadLettered,
    Unknown(String),
}

impl WorkbenchTraceEntryKind {
    fn as_str(&self) -> &str {
        match self {
            Self::AlertCreated => "alert_created",
            Self::HealthPlanState => "health_plan_state",
            Self::TaskState => "task_state",
            Self::CheckinReceived => "checkin_received",
            Self::ActionRequestProposed => "action_request_proposed",
            Self::ActionRequestApproved => "action_request_approved",
            Self::ActionRequestRejected => "action_request_rejected",
            Self::ActionRequestExecution => "action_request_execution",
            Self::AuditAction => "audit_action",
            Self::OutboxEvent => "outbox_event",
            Self::OutboxDelivery => "outbox_delivery",
            Self::DeliveryFailed => "delivery_failed",
            Self::DeliverySucceeded => "delivery_succeeded",
            Self::DeliveryRecovered => "delivery_recovered",
            Self::DeliveryDeadLettered => "delivery_dead_lettered",
            Self::Unknown(value) => value.as_str(),
        }
    }
}

impl Serialize for WorkbenchTraceEntryKind {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for WorkbenchTraceEntryKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Ok(match value.as_str() {
            "alert_created" => Self::AlertCreated,
            "health_plan_state" => Self::HealthPlanState,
            "task_state" => Self::TaskState,
            "checkin_received" => Self::CheckinReceived,
            "action_request_proposed" => Self::ActionRequestProposed,
            "action_request_approved" => Self::ActionRequestApproved,
            "action_request_rejected" => Self::ActionRequestRejected,
            "action_request_execution" => Self::ActionRequestExecution,
            "audit_action" => Self::AuditAction,
            "outbox_event" => Self::OutboxEvent,
            "outbox_delivery" => Self::OutboxDelivery,
            "delivery_failed" => Self::DeliveryFailed,
            "delivery_succeeded" => Self::DeliverySucceeded,
            "delivery_recovered" => Self::DeliveryRecovered,
            "delivery_dead_lettered" => Self::DeliveryDeadLettered,
            _ => Self::Unknown(value),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkbenchTraceSource {
    Alerts,
    HealthPlans,
    TaskInstances,
    Checkins,
    ActionRequests,
    AuditLogs,
    EventOutbox,
    OutboxDeliveries,
    Unknown(String),
}

impl WorkbenchTraceSource {
    fn as_str(&self) -> &str {
        match self {
            Self::Alerts => "alerts",
            Self::HealthPlans => "health_plans",
            Self::TaskInstances => "task_instances",
            Self::Checkins => "checkins",
            Self::ActionRequests => "action_requests",
            Self::AuditLogs => "audit_logs",
            Self::EventOutbox => "event_outbox",
            Self::OutboxDeliveries => "outbox_deliveries",
            Self::Unknown(value) => value.as_str(),
        }
    }
}

impl Serialize for WorkbenchTraceSource {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for WorkbenchTraceSource {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Ok(match value.as_str() {
            "alerts" => Self::Alerts,
            "health_plans" => Self::HealthPlans,
            "task_instances" => Self::TaskInstances,
            "checkins" => Self::Checkins,
            "action_requests" => Self::ActionRequests,
            "audit_logs" => Self::AuditLogs,
            "event_outbox" => Self::EventOutbox,
            "outbox_deliveries" => Self::OutboxDeliveries,
            _ => Self::Unknown(value),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkbenchTraceWarningCode {
    TraceTruncated,
    UnsupportedRelatedType,
    MissingRelatedTask,
    MissingRelatedPlan,
    MissingRelatedEvent,
    MissingRelatedActionRequest,
    ActionRequestProjectionTruncated,
    InvalidActionRequestProjection,
    ActionRequestAuditsTruncated,
    ActionRequestEventsTruncated,
    ActionRequestDeliveriesTruncated,
    ActionRequestLinksTruncated,
    TraceReservedBudgetExceeded,
    Unknown(String),
}

impl WorkbenchTraceWarningCode {
    fn as_str(&self) -> &str {
        match self {
            Self::TraceTruncated => "trace_truncated",
            Self::UnsupportedRelatedType => "unsupported_related_type",
            Self::MissingRelatedTask => "missing_related_task",
            Self::MissingRelatedPlan => "missing_related_plan",
            Self::MissingRelatedEvent => "missing_related_event",
            Self::MissingRelatedActionRequest => "missing_related_action_request",
            Self::ActionRequestProjectionTruncated => "action_request_projection_truncated",
            Self::InvalidActionRequestProjection => "invalid_action_request_projection",
            Self::ActionRequestAuditsTruncated => "action_request_audits_truncated",
            Self::ActionRequestEventsTruncated => "action_request_events_truncated",
            Self::ActionRequestDeliveriesTruncated => "action_request_deliveries_truncated",
            Self::ActionRequestLinksTruncated => "action_request_links_truncated",
            Self::TraceReservedBudgetExceeded => "trace_reserved_budget_exceeded",
            Self::Unknown(value) => value.as_str(),
        }
    }
}

impl Serialize for WorkbenchTraceWarningCode {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for WorkbenchTraceWarningCode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Ok(match value.as_str() {
            "trace_truncated" => Self::TraceTruncated,
            "unsupported_related_type" => Self::UnsupportedRelatedType,
            "missing_related_task" => Self::MissingRelatedTask,
            "missing_related_plan" => Self::MissingRelatedPlan,
            "missing_related_event" => Self::MissingRelatedEvent,
            "missing_related_action_request" => Self::MissingRelatedActionRequest,
            "action_request_projection_truncated" => Self::ActionRequestProjectionTruncated,
            "invalid_action_request_projection" => Self::InvalidActionRequestProjection,
            "action_request_audits_truncated" => Self::ActionRequestAuditsTruncated,
            "action_request_events_truncated" => Self::ActionRequestEventsTruncated,
            "action_request_deliveries_truncated" => Self::ActionRequestDeliveriesTruncated,
            "action_request_links_truncated" => Self::ActionRequestLinksTruncated,
            "trace_reserved_budget_exceeded" => Self::TraceReservedBudgetExceeded,
            _ => Self::Unknown(value),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkbenchTraceSeverity {
    Low,
    Medium,
    High,
    Critical,
    Unknown(String),
}

impl WorkbenchTraceSeverity {
    fn as_str(&self) -> &str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Critical => "critical",
            Self::Unknown(value) => value.as_str(),
        }
    }
}

impl Serialize for WorkbenchTraceSeverity {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for WorkbenchTraceSeverity {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Ok(match value.as_str() {
            "low" => Self::Low,
            "medium" => Self::Medium,
            "high" => Self::High,
            "critical" => Self::Critical,
            _ => Self::Unknown(value),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkbenchTraceActor {
    #[serde(rename = "type")]
    pub actor_type: String,
    #[serde(default)]
    pub id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkbenchTraceEntry {
    pub id: String,
    pub occurred_at: String,
    pub kind: WorkbenchTraceEntryKind,
    pub source: WorkbenchTraceSource,
    pub title: String,
    #[serde(default)]
    pub detail: Option<String>,
    #[serde(default)]
    pub actor: Option<WorkbenchTraceActor>,
    #[serde(default)]
    pub related_type: Option<String>,
    #[serde(default)]
    pub related_id: Option<String>,
    #[serde(default)]
    pub severity: Option<WorkbenchTraceSeverity>,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkbenchTraceWarning {
    pub code: WorkbenchTraceWarningCode,
    pub message: String,
    #[serde(default)]
    pub source: Option<WorkbenchTraceSource>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkbenchWorkflowIdentity {
    #[serde(rename = "type")]
    pub workflow_type: String,
    pub id: String,
    #[serde(default)]
    pub task_id: Option<String>,
    #[serde(default)]
    pub openclaw_flow_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkbenchAlertTrace {
    pub alert_id: String,
    #[serde(default)]
    pub workflow: Option<WorkbenchWorkflowIdentity>,
    pub partial: bool,
    #[serde(default)]
    pub warnings: Vec<WorkbenchTraceWarning>,
    #[serde(default)]
    pub entries: Vec<WorkbenchTraceEntry>,
}
