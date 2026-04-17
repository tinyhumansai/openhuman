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

    #[test]
    fn category_all_has_10_variants() {
        assert_eq!(CapabilityCategory::ALL.len(), 10);
    }

    #[test]
    fn category_as_str_roundtrips_through_from_str() {
        for cat in CapabilityCategory::ALL {
            let s = cat.as_str();
            let parsed: CapabilityCategory = s.parse().unwrap();
            assert_eq!(parsed, cat);
        }
    }

    #[test]
    fn category_from_str_accepts_aliases() {
        assert_eq!(
            "local-ai".parse::<CapabilityCategory>().unwrap(),
            CapabilityCategory::LocalAI
        );
        assert_eq!(
            "local ai".parse::<CapabilityCategory>().unwrap(),
            CapabilityCategory::LocalAI
        );
        assert_eq!(
            "localai".parse::<CapabilityCategory>().unwrap(),
            CapabilityCategory::LocalAI
        );
        assert_eq!(
            "screen-intelligence".parse::<CapabilityCategory>().unwrap(),
            CapabilityCategory::ScreenIntelligence
        );
        assert_eq!(
            "screen intelligence".parse::<CapabilityCategory>().unwrap(),
            CapabilityCategory::ScreenIntelligence
        );
    }

    #[test]
    fn category_from_str_is_case_insensitive() {
        assert_eq!(
            "CONVERSATION".parse::<CapabilityCategory>().unwrap(),
            CapabilityCategory::Conversation
        );
        assert_eq!(
            "  Team  ".parse::<CapabilityCategory>().unwrap(),
            CapabilityCategory::Team
        );
    }

    #[test]
    fn category_from_str_rejects_unknown() {
        let err = "bogus".parse::<CapabilityCategory>().unwrap_err();
        assert!(err.contains("unknown capability category"));
        assert!(err.contains("bogus"));
    }

    #[test]
    fn status_as_str_covers_all_variants() {
        assert_eq!(CapabilityStatus::Stable.as_str(), "stable");
        assert_eq!(CapabilityStatus::Beta.as_str(), "beta");
        assert_eq!(CapabilityStatus::ComingSoon.as_str(), "coming_soon");
        assert_eq!(CapabilityStatus::Deprecated.as_str(), "deprecated");
    }

    #[test]
    fn status_serde_roundtrip() {
        for status in [
            CapabilityStatus::Stable,
            CapabilityStatus::Beta,
            CapabilityStatus::ComingSoon,
            CapabilityStatus::Deprecated,
        ] {
            let json = serde_json::to_string(&status).unwrap();
            let back: CapabilityStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(back, status);
        }
    }

    #[test]
    fn category_serde_roundtrip_all() {
        for cat in CapabilityCategory::ALL {
            let json = serde_json::to_string(&cat).unwrap();
            let back: CapabilityCategory = serde_json::from_str(&json).unwrap();
            assert_eq!(back, cat);
        }
    }
}
