use std::str::FromStr;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CapabilityCategory {
    #[serde(rename = "conversation")]
    Conversation,
    #[serde(rename = "intelligence")]
    Intelligence,
    #[serde(rename = "workflows", alias = "skills")]
    Workflows,
    #[serde(rename = "local_ai")]
    LocalAI,
    #[serde(rename = "team")]
    Team,
    #[serde(rename = "settings")]
    Settings,
    #[serde(rename = "auth")]
    Auth,
    #[serde(rename = "channels")]
    Channels,
    #[serde(rename = "automation")]
    Automation,
    #[serde(rename = "mobile")]
    Mobile,
}

impl CapabilityCategory {
    pub const ALL: [Self; 10] = [
        Self::Conversation,
        Self::Intelligence,
        Self::Workflows,
        Self::LocalAI,
        Self::Team,
        Self::Settings,
        Self::Auth,
        Self::Channels,
        Self::Automation,
        Self::Mobile,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Conversation => "conversation",
            Self::Intelligence => "intelligence",
            Self::Workflows => "workflows",
            Self::LocalAI => "local_ai",
            Self::Team => "team",
            Self::Settings => "settings",
            Self::Auth => "auth",
            Self::Channels => "channels",
            Self::Automation => "automation",
            Self::Mobile => "mobile",
        }
    }
}

impl FromStr for CapabilityCategory {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let normalized = value.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "conversation" => Ok(Self::Conversation),
            "intelligence" => Ok(Self::Intelligence),
            "workflows" | "skills" => Ok(Self::Workflows),
            "local_ai" | "local-ai" | "local ai" | "localai" => Ok(Self::LocalAI),
            "team" => Ok(Self::Team),
            "settings" => Ok(Self::Settings),
            "auth" => Ok(Self::Auth),
            "channels" => Ok(Self::Channels),
            "automation" => Ok(Self::Automation),
            "mobile" => Ok(Self::Mobile),
            _ => Err(format!(
                "unknown capability category '{value}'; expected one of: {}",
                Self::ALL
                    .iter()
                    .map(|category| category.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CapabilityStatus {
    #[serde(rename = "stable")]
    Stable,
    #[serde(rename = "beta")]
    Beta,
    #[serde(rename = "coming_soon")]
    ComingSoon,
    #[serde(rename = "deprecated")]
    Deprecated,
}

impl CapabilityStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Beta => "beta",
            Self::ComingSoon => "coming_soon",
            Self::Deprecated => "deprecated",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Capability {
    pub id: &'static str,
    pub name: &'static str,
    pub domain: &'static str,
    pub category: CapabilityCategory,
    pub description: &'static str,
    pub how_to: &'static str,
    pub status: CapabilityStatus,
    /// Optional privacy disclosure metadata. `None` means the capability has not
    /// been annotated yet — UI should treat absence as "unknown", not "safe".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub privacy: Option<CapabilityPrivacy>,
}

/// Per-capability privacy disclosure consumed by the in-app Privacy surface.
///
/// Source of truth for "what leaves my computer" — kept narrow on purpose so
/// every field is something the UI can render directly without further mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct CapabilityPrivacy {
    /// True when invoking this capability sends *some* data off the device.
    pub leaves_device: bool,
    /// Classifies what kind of data leaves (or stays).
    pub data_kind: PrivacyDataKind,
    /// Stable, human-readable destinations data may flow to (empty when local-only).
    pub destinations: &'static [&'static str],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrivacyDataKind {
    /// Raw user content (messages, screen frames, audio) — kept local.
    Raw,
    /// Derived signals (embeddings, summaries, prompts) — may be sent to backends.
    Derived,
    /// OAuth tokens, API keys, wallet connections — stored locally, never logged.
    Credentials,
    /// Anonymous analytics, crash reports, version pings.
    Diagnostics,
    /// Non-sensitive metadata (capability ids, feature flags, settings shape).
    Metadata,
}

#[cfg(test)]
#[path = "types_tests.rs"]
mod tests;
