//! Onboarding history-reward upload, scoring, and claim types.
//!
//! Split from the parent types module. Field names use serde renames to match
//! the backend camelCase wire format exactly, and unknown fields are tolerated
//! so the client keeps working against newer server versions.

use serde::Deserialize;

/// Running claim metrics returned after uploading one transcript
/// (`POST /agent-integrations/history-rewards/uploads`).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryUploadResult {
    /// Transcripts accumulated on the claim so far.
    pub session_count: u64,
    /// Running token total across the claim.
    pub cumulative_tokens: u64,
    /// Running distinct-active-day count across the claim.
    pub active_days: u64,
    /// Agents represented on the claim so far.
    #[serde(default)]
    pub agents: Vec<String>,
}

/// Per-metric USD contributions, so the reveal can show how a total was earned.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryRewardBreakdown {
    pub tokens_usd: f64,
    pub active_days_usd: f64,
    pub sessions_usd: f64,
    pub multi_agent_usd: f64,
}

/// The caller's history-reward status
/// (`GET /agent-integrations/history-rewards/status`).
///
/// This is the authority for "has this user already earned the reward?" — the
/// local config flag only decides whether to re-render the welcome screen.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryRewardStatus {
    /// True once the reward has been granted.
    pub claimed: bool,
    /// True when at least one transcript was uploaded but not yet claimed.
    #[serde(default)]
    pub has_uploads: bool,
    /// USD granted, populated once scored.
    #[serde(default)]
    pub awarded_usd: f64,
    /// Human-facing power-level label.
    #[serde(default)]
    pub tier: Option<String>,
    #[serde(default)]
    pub session_count: u64,
    #[serde(default)]
    pub cumulative_tokens: u64,
    #[serde(default)]
    pub active_days: u64,
    #[serde(default)]
    pub agents: Vec<String>,
    /// Advertised ceiling, so the client renders "x of $25" without hardcoding.
    #[serde(default)]
    pub max_reward_usd: f64,
}

/// Result of claiming the reward
/// (`POST /agent-integrations/history-rewards/claim`).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryRewardClaim {
    /// The settled status after claiming.
    #[serde(flatten)]
    pub status: HistoryRewardStatus,
    /// How the award was composed.
    #[serde(default)]
    pub breakdown: HistoryRewardBreakdown,
    /// True when this call granted nothing new (a repeat claim).
    #[serde(default)]
    pub already_claimed: bool,
}
