//! Tiered model presets and recommendation logic for local AI.
//!
//! # Tier → Model ID Mapping
//!
//! | Tier   | Chat / Vision Model       | Embedding Model            | Min RAM | ~Download |
//! |--------|---------------------------|----------------------------|---------|-----------|
//! | Low    | `gemma3:1b-it-q4_0`       | `nomic-embed-text:latest`  | 4 GB    | ~1 GB     |
//! | Medium | `gemma3:4b-it-qat`        | `nomic-embed-text:latest`  | 8 GB    | ~3 GB     |
//! | High   | `gemma3:12b-it-q4_K_M`    | `nomic-embed-text:latest`  | 16 GB   | ~8 GB     |
//!
//! # Changing defaults for a release
//!
//! Edit [`all_presets()`] below. Each `ModelPreset` defines:
//! - `chat_model_id` / `vision_model_id` — Ollama tag for the chat and vision models.
//! - `embedding_model_id` — Ollama tag for the embedding model.
//! - `quantization` — quantization label shown in the UI.
//! - `min_ram_gb` / `approx_download_gb` — user-facing guidance.
//!
//! After changing model tags, verify they exist in the Ollama library and update
//! the `OPENHUMAN_LOCAL_AI_TIER` env var docs in `.env.example` if tier names change.

use serde::{Deserialize, Serialize};

use crate::openhuman::config::schema::LocalAiConfig;

use super::device::DeviceProfile;

/// Performance tier for local AI model selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelTier {
    Low,
    Medium,
    High,
    Custom,
}

impl ModelTier {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Custom => "custom",
        }
    }

    pub fn from_str_opt(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            "custom" => Some(Self::Custom),
            _ => None,
        }
    }
}

/// A concrete model preset tied to a performance tier.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelPreset {
    pub tier: ModelTier,
    pub label: &'static str,
    pub description: &'static str,
    pub chat_model_id: &'static str,
    pub vision_model_id: &'static str,
    pub embedding_model_id: &'static str,
    pub quantization: &'static str,
    pub min_ram_gb: u64,
    pub approx_download_gb: f32,
}

/// Return all built-in presets (Low, Medium, High).
pub fn all_presets() -> Vec<ModelPreset> {
    vec![
        ModelPreset {
            tier: ModelTier::Low,
            label: "Lightweight",
            description: "Smallest footprint. Works on machines with 4 GB+ RAM.",
            chat_model_id: "gemma3:1b-it-q4_0",
            vision_model_id: "gemma3:1b-it-q4_0",
            embedding_model_id: "nomic-embed-text:latest",
            quantization: "q4_0",
            min_ram_gb: 4,
            approx_download_gb: 1.0,
        },
        ModelPreset {
            tier: ModelTier::Medium,
            label: "Balanced",
            description: "Good quality with moderate resource use. Requires 8 GB+ RAM.",
            chat_model_id: "gemma3:4b-it-qat",
            vision_model_id: "gemma3:4b-it-qat",
            embedding_model_id: "nomic-embed-text:latest",
            quantization: "q4",
            min_ram_gb: 8,
            approx_download_gb: 3.0,
        },
        ModelPreset {
            tier: ModelTier::High,
            label: "Performance",
            description: "Best quality. Requires 16 GB+ RAM and a capable GPU.",
            chat_model_id: "gemma3:12b-it-q4_K_M",
            vision_model_id: "gemma3:12b-it-q4_K_M",
            embedding_model_id: "nomic-embed-text:latest",
            quantization: "q4_K_M",
            min_ram_gb: 16,
            approx_download_gb: 8.0,
        },
    ]
}

/// Return the preset for a specific tier, or `None` for `Custom`.
pub fn preset_for_tier(tier: ModelTier) -> Option<ModelPreset> {
    all_presets().into_iter().find(|p| p.tier == tier)
}

/// Recommend a tier based on device capabilities.
///
/// * < 8 GB RAM  -> Low
/// * 8 - 15 GB   -> Medium
/// * >= 16 GB    -> High
pub fn recommend_tier(device: &DeviceProfile) -> ModelTier {
    let ram_gb = device.total_ram_gb();
    let tier = if ram_gb >= 16 {
        ModelTier::High
    } else if ram_gb >= 8 {
        ModelTier::Medium
    } else {
        ModelTier::Low
    };
    tracing::debug!(ram_gb, ?tier, "recommended model tier");
    tier
}

/// Apply a preset to a [`LocalAiConfig`], overwriting model IDs, quantization,
/// and the `selected_tier` marker.
pub fn apply_preset_to_config(config: &mut LocalAiConfig, tier: ModelTier) {
    if let Some(preset) = preset_for_tier(tier) {
        tracing::debug!(
            ?tier,
            chat = preset.chat_model_id,
            "applying preset to config"
        );
        config.model_id = preset.chat_model_id.to_string();
        config.chat_model_id = preset.chat_model_id.to_string();
        config.vision_model_id = preset.vision_model_id.to_string();
        config.embedding_model_id = preset.embedding_model_id.to_string();
        config.quantization = preset.quantization.to_string();
        config.selected_tier = Some(tier.as_str().to_string());
    } else {
        tracing::debug!("apply_preset_to_config called for Custom tier; no-op");
    }
}

/// Reverse-lookup the current tier from config. Returns `Custom` if none of the
/// built-in presets match the current model IDs.
pub fn current_tier_from_config(config: &LocalAiConfig) -> ModelTier {
    // If a tier is explicitly stored, try to match it first.
    if let Some(ref stored) = config.selected_tier {
        if let Some(tier) = ModelTier::from_str_opt(stored) {
            if tier == ModelTier::Custom {
                return ModelTier::Custom;
            }
            // Verify the stored tier still matches the actual model IDs.
            if let Some(preset) = preset_for_tier(tier) {
                if config.chat_model_id == preset.chat_model_id
                    && config.vision_model_id == preset.vision_model_id
                    && config.embedding_model_id == preset.embedding_model_id
                {
                    return tier;
                }
            }
        }
    }

    // Fallback: match by model IDs.
    for preset in all_presets() {
        if config.chat_model_id == preset.chat_model_id
            && config.vision_model_id == preset.vision_model_id
            && config.embedding_model_id == preset.embedding_model_id
        {
            return preset.tier;
        }
    }

    ModelTier::Custom
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recommend_tier_by_ram() {
        let low_device = DeviceProfile {
            total_ram_bytes: 4 * 1024 * 1024 * 1024, // 4 GB
            cpu_count: 4,
            cpu_brand: String::new(),
            os_name: String::new(),
            os_version: String::new(),
            has_gpu: false,
            gpu_description: None,
        };
        assert_eq!(recommend_tier(&low_device), ModelTier::Low);

        let medium_device = DeviceProfile {
            total_ram_bytes: 8 * 1024 * 1024 * 1024, // 8 GB
            ..low_device.clone()
        };
        assert_eq!(recommend_tier(&medium_device), ModelTier::Medium);

        let high_device = DeviceProfile {
            total_ram_bytes: 32 * 1024 * 1024 * 1024_u64, // 32 GB
            ..low_device.clone()
        };
        assert_eq!(recommend_tier(&high_device), ModelTier::High);
    }

    #[test]
    fn preset_application_and_round_trip() {
        let mut config = LocalAiConfig::default();
        apply_preset_to_config(&mut config, ModelTier::Low);
        assert_eq!(config.chat_model_id, "gemma3:1b-it-q4_0");
        assert_eq!(config.selected_tier, Some("low".to_string()));
        assert_eq!(current_tier_from_config(&config), ModelTier::Low);
    }

    #[test]
    fn custom_detection_when_models_dont_match() {
        let mut config = LocalAiConfig::default();
        config.chat_model_id = "some-other-model:latest".to_string();
        config.selected_tier = None;
        assert_eq!(current_tier_from_config(&config), ModelTier::Custom);
    }

    #[test]
    fn all_presets_returns_three_tiers() {
        let presets = all_presets();
        assert_eq!(presets.len(), 3);
        assert_eq!(presets[0].tier, ModelTier::Low);
        assert_eq!(presets[1].tier, ModelTier::Medium);
        assert_eq!(presets[2].tier, ModelTier::High);
    }

    #[test]
    fn default_config_maps_to_medium() {
        let config = LocalAiConfig::default();
        // Default config uses gemma3:4b-it-qat which is the Medium preset.
        assert_eq!(current_tier_from_config(&config), ModelTier::Medium);
    }
}
