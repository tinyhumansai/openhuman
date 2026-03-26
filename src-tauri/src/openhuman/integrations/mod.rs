//! Integration registry for UI display.

pub mod registry;

use crate::openhuman::config::Config;
use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Integration status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IntegrationStatus {
    /// Fully implemented and ready to use
    Available,
    /// Configured and active
    Active,
    /// Planned but not yet implemented
    ComingSoon,
}

/// Integration category
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IntegrationCategory {
    Chat,
    AiModel,
    Productivity,
    MusicAudio,
    SmartHome,
    ToolsAutomation,
    MediaCreative,
    Social,
    Platform,
}

impl IntegrationCategory {
    pub fn label(self) -> &'static str {
        match self {
            Self::Chat => "Chat Providers",
            Self::AiModel => "AI Models",
            Self::Productivity => "Productivity",
            Self::MusicAudio => "Music & Audio",
            Self::SmartHome => "Smart Home",
            Self::ToolsAutomation => "Tools & Automation",
            Self::MediaCreative => "Media & Creative",
            Self::Social => "Social",
            Self::Platform => "Platforms",
        }
    }

    pub fn all() -> &'static [Self] {
        &[
            Self::Chat,
            Self::AiModel,
            Self::Productivity,
            Self::MusicAudio,
            Self::SmartHome,
            Self::ToolsAutomation,
            Self::MediaCreative,
            Self::Social,
            Self::Platform,
        ]
    }
}

/// A registered integration
pub struct IntegrationEntry {
    pub name: &'static str,
    pub description: &'static str,
    pub category: IntegrationCategory,
    pub status_fn: fn(&Config) -> IntegrationStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntegrationInfo {
    pub name: String,
    pub description: String,
    pub category: IntegrationCategory,
    pub status: IntegrationStatus,
    pub setup_hints: Vec<String>,
}

pub fn list_integrations(config: &Config) -> Vec<IntegrationInfo> {
    registry::all_integrations()
        .into_iter()
        .map(|entry| IntegrationInfo {
            name: entry.name.to_string(),
            description: entry.description.to_string(),
            category: entry.category,
            status: (entry.status_fn)(config),
            setup_hints: integration_setup_hints(entry.name, (entry.status_fn)(config)),
        })
        .collect()
}

pub fn get_integration_info(config: &Config, name: &str) -> Result<IntegrationInfo> {
    let entries = registry::all_integrations();
    let name_lower = name.to_lowercase();

    let Some(entry) = entries.iter().find(|e| e.name.to_lowercase() == name_lower) else {
        anyhow::bail!(
            "Unknown integration: {name}. Check the UI integrations catalog for supported integrations."
        );
    };

    let status = (entry.status_fn)(config);
    Ok(IntegrationInfo {
        name: entry.name.to_string(),
        description: entry.description.to_string(),
        category: entry.category,
        status,
        setup_hints: integration_setup_hints(entry.name, status),
    })
}

fn integration_setup_hints(name: &str, status: IntegrationStatus) -> Vec<String> {
    let mut hints = Vec::new();

    match name {
        "Telegram" => {
            hints.push("Message @BotFather on Telegram".to_string());
            hints.push("Create a bot and copy the token".to_string());
            hints.push("Add the token in Settings > Channels".to_string());
        }
        "Discord" => {
            hints.push("Create an app at https://discord.com/developers/applications".to_string());
            hints.push("Enable MESSAGE CONTENT intent".to_string());
            hints.push("Paste the bot token in Settings > Channels".to_string());
        }
        "Slack" => {
            hints.push("Create an app at https://api.slack.com/apps".to_string());
            hints.push("Install the app and copy the bot token".to_string());
            hints.push("Paste the token in Settings > Channels".to_string());
        }
        "OpenRouter" => {
            hints.push("Get an API key at https://openrouter.ai/keys".to_string());
            hints.push("Paste the key in Settings > Providers".to_string());
        }
        "Ollama" => {
            hints.push("Install Ollama locally".to_string());
            hints.push("Pull a model: ollama pull llama3".to_string());
            hints.push("Set provider to 'ollama' in Settings > Providers".to_string());
        }
        "iMessage" => {
            hints.push("macOS only: ensure Full Disk Access is enabled".to_string());
        }
        "GitHub" => {
            hints.push("Create a personal access token in GitHub settings".to_string());
            hints.push("Add it under Settings > Integrations".to_string());
        }
        "Browser" => {
            hints.push("Browser automation is built-in".to_string());
        }
        "Cron" => {
            hints.push("Create schedules in the UI".to_string());
        }
        "Webhooks" => {
            hints.push("Enable the gateway to receive webhooks".to_string());
        }
        _ => {
            if status == IntegrationStatus::ComingSoon {
                hints.push("This integration is planned.".to_string());
            }
        }
    }

    hints
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integration_category_all_includes_every_variant_once() {
        let all = IntegrationCategory::all();
        assert_eq!(all.len(), 9);

        let labels: Vec<&str> = all.iter().map(|cat| cat.label()).collect();
        assert!(labels.contains(&"Chat Providers"));
        assert!(labels.contains(&"AI Models"));
        assert!(labels.contains(&"Productivity"));
        assert!(labels.contains(&"Music & Audio"));
        assert!(labels.contains(&"Smart Home"));
        assert!(labels.contains(&"Tools & Automation"));
        assert!(labels.contains(&"Media & Creative"));
        assert!(labels.contains(&"Social"));
        assert!(labels.contains(&"Platforms"));
    }

    #[test]
    fn get_integration_info_is_case_insensitive_for_known_integrations() {
        let config = Config::default();
        let first_name = registry::all_integrations()
            .first()
            .expect("integration registry is not empty")
            .name
            .to_string();

        let info = get_integration_info(&config, &first_name.to_lowercase()).unwrap();
        assert_eq!(info.name, first_name);
    }
}
