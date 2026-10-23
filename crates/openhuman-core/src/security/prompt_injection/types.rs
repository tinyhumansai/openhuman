//! The shapes a prompt screen answers with. The scoring, the rules and the
//! thresholds are the security module's (`ScanPrompt2`); these are what the host
//! maps its tier to.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PromptInjectionVerdict {
    Allow,
    Block,
    Review,
}

impl PromptInjectionVerdict {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Block => "block",
            Self::Review => "review",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptInjectionReason {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptEnforcementAction {
    Allow,
    Blocked,
    ReviewBlocked,
}

impl PromptEnforcementAction {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Blocked => "block",
            Self::ReviewBlocked => "review_blocked",
        }
    }
}

#[derive(Debug, Clone)]
pub struct PromptEnforcementDecision {
    pub verdict: PromptInjectionVerdict,
    pub score: f32,
    pub reasons: Vec<PromptInjectionReason>,
    pub action: PromptEnforcementAction,
    pub prompt_hash: String,
    pub prompt_chars: usize,
}

#[derive(Debug, Clone, Copy)]
pub struct PromptEnforcementContext<'a> {
    pub source: &'a str,
    pub request_id: Option<&'a str>,
    pub user_id: Option<&'a str>,
    pub session_id: Option<&'a str>,
}
