use std::str::FromStr;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CapabilityCategory {
    #[serde(rename = "conversation")]
    Conversation,
    #[serde(rename = "intelligence")]
    Intelligence,
    #[serde(rename = "skills")]
    Skills,
    #[serde(rename = "local_ai")]
    LocalAI,
    #[serde(rename = "team")]
    Team,
    #[serde(rename = "settings")]
    Settings,
    #[serde(rename = "auth")]
    Auth,
    #[serde(rename = "screen_intelligence")]
    ScreenIntelligence,
    #[serde(rename = "channels")]
    Channels,
    #[serde(rename = "automation")]
    Automation,
}

impl CapabilityCategory {
    pub const ALL: [Self; 10] = [
        Self::Conversation,
        Self::Intelligence,
        Self::Skills,
        Self::LocalAI,
        Self::Team,
        Self::Settings,
        Self::Auth,
        Self::ScreenIntelligence,
        Self::Channels,
        Self::Automation,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Conversation => "conversation",
            Self::Intelligence => "intelligence",
            Self::Skills => "skills",
            Self::LocalAI => "local_ai",
            Self::Team => "team",
            Self::Settings => "settings",
            Self::Auth => "auth",
            Self::ScreenIntelligence => "screen_intelligence",
            Self::Channels => "channels",
            Self::Automation => "automation",
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
            "skills" => Ok(Self::Skills),
            "local_ai" | "local-ai" | "local ai" | "localai" => Ok(Self::LocalAI),
            "team" => Ok(Self::Team),
            "settings" => Ok(Self::Settings),
            "auth" => Ok(Self::Auth),
            "screen_intelligence" | "screen-intelligence" | "screen intelligence" => {
                Ok(Self::ScreenIntelligence)
            }
            "channels" => Ok(Self::Channels),
            "automation" => Ok(Self::Automation),
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn category_serializes_expected_wire_names() {
        assert_eq!(
            serde_json::to_string(&CapabilityCategory::LocalAI).expect("serialize LocalAI"),
            "\"local_ai\""
        );
        assert_eq!(
            serde_json::to_string(&CapabilityCategory::ScreenIntelligence)
                .expect("serialize ScreenIntelligence"),
            "\"screen_intelligence\""
        );
    }

    #[test]
    fn status_serializes_expected_wire_names() {
        assert_eq!(
            serde_json::to_string(&CapabilityStatus::ComingSoon).expect("serialize ComingSoon"),
            "\"coming_soon\""
        );
    }
}
