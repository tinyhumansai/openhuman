use std::collections::BTreeMap;

use goose_provider_types::conversation::Conversation;
use serde::{Deserialize, Serialize};

/// Durable per-turn usage. `latest_primary_input_tokens` is context occupancy
/// for the latest primary call; the other fields are cumulative traffic.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GooseUsage {
    pub primary_calls: u32,
    pub latest_primary_input_tokens: u64,
    pub cumulative_input_tokens: u64,
    pub cumulative_output_tokens: u64,
    pub cumulative_cached_input_tokens: u64,
    pub cumulative_cache_creation_tokens: u64,
    pub cumulative_reasoning_tokens: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AcceptedToolAction {
    pub call_id: String,
    pub tool_name: String,
    pub arguments: serde_json::Value,
    pub observation: Option<ToolObservation>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolObservation {
    pub call_id: String,
    pub success: bool,
    pub output: String,
}

/// The complete state reloaded by Goose before every pass.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GooseCheckpoint {
    pub revision: u64,
    pub conversation: Conversation,
    #[serde(default)]
    pub actions: BTreeMap<String, AcceptedToolAction>,
    #[serde(default)]
    pub usage: GooseUsage,
    #[serde(default)]
    pub protocol_correction_count: u32,
    #[serde(default)]
    pub terminal_protocol_failure: bool,
    #[serde(default)]
    pub last_call_signature: Option<String>,
    #[serde(default)]
    pub last_failure_type: Option<String>,
    #[serde(default)]
    pub repeated_failure_count: u32,
    #[serde(default)]
    pub no_progress_count: u32,
    #[serde(default)]
    pub unavailable_routes: Vec<String>,
    #[serde(default)]
    pub completion_state: Option<crate::agent::primary_orchestration::CompletionStatus>,
    #[serde(default)]
    pub terminal_reason: Option<String>,
}

impl GooseCheckpoint {
    pub fn new(conversation: Conversation) -> Self {
        Self {
            revision: 0,
            conversation,
            actions: BTreeMap::new(),
            usage: GooseUsage::default(),
            protocol_correction_count: 0,
            terminal_protocol_failure: false,
            last_call_signature: None,
            last_failure_type: None,
            repeated_failure_count: 0,
            no_progress_count: 0,
            unavailable_routes: Vec::new(),
            completion_state: None,
            terminal_reason: None,
        }
    }

    /// Whether the persisted machine is waiting for another pass rather than
    /// holding a terminal assistant answer.
    pub fn is_resumable(&self) -> bool {
        self.conversation.last().is_some_and(|message| {
            matches!(
                goose_provider_types::conversation::effective_role(message),
                goose_provider_types::conversation::EffectiveRole::User
                    | goose_provider_types::conversation::EffectiveRole::Tool
            )
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GooseStopReason {
    FinalAnswer,
    Cancelled,
    CallCeiling,
    Yielded,
    DuplicateSignature,
    RepeatedFailure,
    UnavailableTool,
    NoProgress,
    Completed,
}

impl GooseStopReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FinalAnswer => "final_answer",
            Self::Cancelled => "cancelled",
            Self::CallCeiling => "call_ceiling",
            Self::Yielded => "yielded",
            Self::DuplicateSignature => "duplicate_signature",
            Self::RepeatedFailure => "repeated_failure",
            Self::UnavailableTool => "unavailable_tool",
            Self::NoProgress => "no_progress",
            Self::Completed => "completed",
        }
    }
}

#[derive(Clone, Debug)]
pub struct GooseTurnOutcome {
    pub checkpoint: GooseCheckpoint,
    pub stop_reason: GooseStopReason,
    pub openhuman_messages: Vec<crate::agent::messages::ConversationMessage>,
}

pub(super) enum OpenHumanEffect {
    Conversation(goose_agent::operation::ConversationEffect),
    Usage {
        model: String,
        input_tokens: u64,
        output_tokens: u64,
        cached_input_tokens: u64,
        cache_creation_tokens: u64,
        reasoning_tokens: u64,
    },
    IncrementProtocolCorrection,
    SetTerminalProtocolFailure,
    SetLastCallSignature(Option<String>),
    RecordFailure(String),
    ResetFailure,
    IncrementNoProgress,
    ResetNoProgress,
    MarkRouteUnavailable(String),
    SetCompletionState(Option<crate::agent::primary_orchestration::CompletionStatus>),
    SetTerminalReason(Option<String>),
}

impl From<goose_provider_types::conversation::message::Message> for OpenHumanEffect {
    fn from(message: goose_provider_types::conversation::message::Message) -> Self {
        Self::Conversation(goose_agent::operation::ConversationEffect::AppendMessage(
            message,
        ))
    }
}

impl goose_agent::operation::MachineEffect for OpenHumanEffect {
    fn ensure_message_ids(&mut self) {
        if let Self::Conversation(effect) = self {
            effect.ensure_message_ids();
        }
    }
}

pub(super) struct GooseSession {
    pub id: String,
    pub checkpoint: GooseCheckpoint,
}

impl goose_agent::machine::MachineSession for GooseSession {
    fn id(&self) -> &str {
        &self.id
    }

    fn conversation(&self) -> Option<&Conversation> {
        Some(&self.checkpoint.conversation)
    }
}
