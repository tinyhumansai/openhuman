//! Tiered model presets and recommendation logic for local AI.
//!
//! Text generation is always the primary summarizer. Vision is a secondary
//! scene-description sidecar whose output can be merged with OCR by the text
//! model when a tier supports it.

use serde::{Deserialize, Serialize};

use crate::openhuman::config::schema::LocalAiConfig;

use super::device::DeviceProfile;

/// Performance tier for local AI model selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModelTier {
    #[serde(rename = "ram_1gb")]
    Ram1Gb,
    #[serde(rename = "ram_2_4gb")]
    Ram2To4Gb,
    #[serde(rename = "ram_4_8gb")]
    Ram4To8Gb,
    #[serde(rename = "ram_8_16gb")]
    Ram8To16Gb,
    #[serde(rename = "ram_16_plus_gb")]
    Ram16PlusGb,
    #[serde(rename = "custom")]
    Custom,
}

/// Single local model tier exposed in the current build. Larger local models
/// stay blocked to keep summarization lightweight and battery-friendly.
pub const MVP_MAX_TIER: ModelTier = ModelTier::Ram2To4Gb;

/// Minimum host RAM (in whole GB) below which the **default** is to skip
/// local inference and use the cloud summarizer instead.  The user can still
/// override this and opt into local AI via settings.
pub const MIN_RAM_GB_FOR_LOCAL_AI: u64 = 8;

/// Returns `true` when the device has enough RAM that local AI should be
/// enabled by default. Below the floor we recommend cloud fallback instead.
pub fn device_supports_local_ai(device: &DeviceProfile) -> bool {
    device.total_ram_gb() >= MIN_RAM_GB_FOR_LOCAL_AI
}

/// Returns `true` when the device is below the RAM floor and local AI should
/// default to disabled (cloud fallback). This is a **recommendation**, not a
/// hard gate — the user can still opt in.
pub fn should_default_to_cloud_fallback(device: &DeviceProfile) -> bool {
    !device_supports_local_ai(device)
}

impl ModelTier {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ram1Gb => "ram_1gb",
            Self::Ram2To4Gb => "ram_2_4gb",
            Self::Ram4To8Gb => "ram_4_8gb",
            Self::Ram8To16Gb => "ram_8_16gb",
            Self::Ram16PlusGb => "ram_16_plus_gb",
            Self::Custom => "custom",
        }
    }

    /// Whether this tier is allowed in the current MVP build.
    pub fn is_mvp_allowed(self) -> bool {
        matches!(self, Self::Ram2To4Gb)
    }

    pub fn from_str_opt(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "ram_1gb" | "tier_1gb" | "1gb" => Some(Self::Ram1Gb),
            "ram_2_4gb" | "tier_2_4gb" | "2_4gb" | "low" => Some(Self::Ram2To4Gb),
            "ram_4_8gb" | "tier_4_8gb" | "4_8gb" => Some(Self::Ram4To8Gb),
            "ram_8_16gb" | "tier_8_16gb" | "8_16gb" | "medium" => Some(Self::Ram8To16Gb),
            "ram_16_plus_gb" | "tier_16_plus_gb" | "16_plus_gb" | "high" => Some(Self::Ram16PlusGb),
            "custom" => Some(Self::Custom),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VisionMode {
    Disabled,
    Ondemand,
    Bundled,
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
    pub vision_mode: VisionMode,
    pub supports_screen_summary: bool,
    pub target_ram_gb: u64,
    pub min_ram_gb: u64,
    pub approx_download_gb: f32,
}

/// Return all built-in presets.
pub fn all_presets() -> Vec<ModelPreset> {
    vec![
        ModelPreset {
            tier: ModelTier::Ram1Gb,
            label: "1 GB",
            description: "Fastest chat-only tier for ultra-low-memory devices. OCR + text only.",
            chat_model_id: "gemma3:270m-it-qat",
            vision_model_id: "",
            embedding_model_id: "all-minilm:latest",
            quantization: "qat",
            vision_mode: VisionMode::Disabled,
            supports_screen_summary: false,
            target_ram_gb: 1,
            min_ram_gb: 1,
            approx_download_gb: 0.3,
        },
        ModelPreset {
            tier: ModelTier::Ram2To4Gb,
            label: "Gemma 3 1B",
            description:
                "Battery-friendly local summarization preset. Uses the 1B Gemma model with vision disabled.",
            chat_model_id: "gemma3:1b-it-qat",
            vision_model_id: "",
            // bge-m3 — 1024 dims required by memory tree's on-disk format
            // and 8192-token context for long-chunk embeds.
            embedding_model_id: "bge-m3",
            quantization: "qat",
            vision_mode: VisionMode::Disabled,
            supports_screen_summary: false,
            target_ram_gb: 2,
            min_ram_gb: 2,
            approx_download_gb: 2.3,
        },
        ModelPreset {
            tier: ModelTier::Ram4To8Gb,
            label: "4-8 GB",
            description: "Light Gemma chat with on-demand vision summaries via Moondream.",
            chat_model_id: "gemma3:1b-it-qat",
            vision_model_id: "moondream:1.8b-v2-q4_K_S",
            embedding_model_id: "all-minilm:latest",
            quantization: "qat",
            vision_mode: VisionMode::Ondemand,
            supports_screen_summary: true,
            target_ram_gb: 4,
            min_ram_gb: 4,
            approx_download_gb: 2.8,
        },
        ModelPreset {
            tier: ModelTier::Ram8To16Gb,
            label: "8-16 GB",
            description: "Balanced Gemma multimodal preset with bundled vision support.",
            chat_model_id: "gemma3:4b-it-qat",
            // Gemma 3 is multimodal from 4B upward, so one model covers chat
            // and vision here. (The 270M and 1B builds are text-only.)
            vision_model_id: "gemma3:4b-it-qat",
            // bge-m3 (1024 dims). nomic-embed-text is 768 dims and fails the
            // memory tree's post-call dimension validator; the embedding
            // allowlist already rewrote it to bge-m3 at resolution time, so
            // naming bge-m3 here only makes the preset honest about what is
            // actually pulled.
            embedding_model_id: "bge-m3",
            quantization: "qat",
            vision_mode: VisionMode::Bundled,
            supports_screen_summary: true,
            target_ram_gb: 8,
            min_ram_gb: 8,
            // gemma3:4b-it-qat (4.0 GB) + bge-m3 (1.2 GB)
            approx_download_gb: 5.2,
        },
        ModelPreset {
            tier: ModelTier::Ram16PlusGb,
            label: "16 GB+",
            description: "Best local quality with Gemma 4 on higher-end devices.",
            // GH #5055 moved this tier off `gemma4:e4b` because no `gemma4`
            // namespace existed on the Ollama library at the time, and landed
            // on `gemma3n:e4b-it-q8_0`. Two things have changed (#5146 §1.3):
            //
            //  1. Gemma 4 has since been published, and `gemma4:e4b-it-q8_0`
            //     resolves (11.6 GB, 128K context).
            //  2. Gemma 3n is **text-only** on Ollama, so using it as the
            //     `vision_model_id` of a `Bundled` vision tier pointed every
            //     vision request at a model with no vision encoder. Ollama
            //     accepts the `images` array against such a model, discards
            //     it, and answers from the prompt text — a hallucinated
            //     description rather than an error.
            //
            // Gemma 4 is multimodal at every published size, so this tier is
            // back to one model serving both chat and vision, matching how the
            // 8-16 GB tier uses `gemma3:4b-it-qat`.
            chat_model_id: "gemma4:e4b-it-q8_0",
            vision_model_id: "gemma4:e4b-it-q8_0",
            // bge-m3 (1024 dims) — see the 8-16 GB tier note above.
            embedding_model_id: "bge-m3",
            // The other tiers ship QAT builds; this one is q8_0. The field is a
            // display label (`effective_quantization`) and does not take part in
            // resolving the model tag, but it should still match what is pulled.
            quantization: "q8_0",
            vision_mode: VisionMode::Bundled,
            supports_screen_summary: true,
            target_ram_gb: 16,
            min_ram_gb: 16,
            // gemma4:e4b-it-q8_0 (11.6 GB) + bge-m3 (1.2 GB)
            approx_download_gb: 12.8,
        },
    ]
}

/// Return only the presets allowed under the current MVP ceiling.
pub fn mvp_presets() -> Vec<ModelPreset> {
    all_presets()
        .into_iter()
        .filter(|preset| preset.tier.is_mvp_allowed())
        .collect()
}

/// Return the preset for a specific tier, or `None` for `Custom`.
pub fn preset_for_tier(tier: ModelTier) -> Option<ModelPreset> {
    all_presets().into_iter().find(|preset| preset.tier == tier)
}

/// Recommend a tier based on device capabilities.
pub fn recommend_tier(device: &DeviceProfile) -> ModelTier {
    // NOTE: the MVP intentionally caps every device at `MVP_MAX_TIER`
    // regardless of installed RAM (the `recommend_tier_scales_with_ram` test
    // pins this non-scaling contract). `ram_gb` is read only for the
    // diagnostic log below; RAM->tier scaling is deferred until the higher
    // tiers are productised.
    let ram_gb = device.total_ram_gb();
    let tier = MVP_MAX_TIER;
    tracing::debug!(ram_gb, ?tier, "[local_ai] recommended model tier");
    tier
}

pub fn vision_mode_for_tier(tier: ModelTier) -> VisionMode {
    match tier {
        ModelTier::Ram1Gb | ModelTier::Ram2To4Gb => VisionMode::Disabled,
        ModelTier::Ram4To8Gb => VisionMode::Ondemand,
        ModelTier::Ram8To16Gb | ModelTier::Ram16PlusGb => VisionMode::Bundled,
        ModelTier::Custom => VisionMode::Bundled,
    }
}

pub fn vision_mode_for_config(config: &LocalAiConfig) -> VisionMode {
    match current_tier_from_config(config) {
        ModelTier::Custom => {
            if config.vision_model_id.trim().is_empty() {
                VisionMode::Disabled
            } else if config.preload_vision_model {
                VisionMode::Bundled
            } else {
                VisionMode::Ondemand
            }
        }
        tier => vision_mode_for_tier(tier),
    }
}

pub fn supports_screen_summary(config: &LocalAiConfig) -> bool {
    !matches!(vision_mode_for_config(config), VisionMode::Disabled)
}

/// Apply a preset to a [`LocalAiConfig`], overwriting model IDs, quantization,
/// and the `selected_tier` marker.
pub fn apply_preset_to_config(config: &mut LocalAiConfig, tier: ModelTier) {
    if let Some(preset) = preset_for_tier(tier) {
        tracing::debug!(
            ?tier,
            chat = preset.chat_model_id,
            vision_mode = ?preset.vision_mode,
            "[local_ai] applying preset to config"
        );
        config.model_id = preset.chat_model_id.to_string();
        config.chat_model_id = preset.chat_model_id.to_string();
        config.vision_model_id = preset.vision_model_id.to_string();
        config.embedding_model_id = preset.embedding_model_id.to_string();
        config.quantization = preset.quantization.to_string();
        config.preload_vision_model = matches!(preset.vision_mode, VisionMode::Bundled);
        config.preload_embedding_model = true;
        config.selected_tier = Some(tier.as_str().to_string());
        // Applying a real preset enables the runtime — this is the authoritative
        // activation path. bootstrap's config_with_recommended_tier_if_unselected
        // also sets runtime_enabled = true when opt_in_confirmed is true, but
        // setting it here ensures in-process callers (tests, RPC handlers) see
        // the correct state without relying on bootstrap's post-processing.
        config.runtime_enabled = true;
    } else {
        tracing::debug!("[local_ai] apply_preset_to_config called for custom tier; no-op");
    }
}

/// Reverse-lookup the current tier from config. Returns `Custom` if none of the
/// built-in presets match the current model IDs.
pub fn current_tier_from_config(config: &LocalAiConfig) -> ModelTier {
    if let Some(ref stored) = config.selected_tier {
        if let Some(tier) = ModelTier::from_str_opt(stored) {
            if tier == ModelTier::Custom {
                return ModelTier::Custom;
            }
            if let Some(preset) = preset_for_tier(tier) {
                let vision_matches = if matches!(preset.vision_mode, VisionMode::Disabled) {
                    config.vision_model_id.trim().is_empty()
                } else {
                    config.vision_model_id == preset.vision_model_id
                };
                if config.chat_model_id == preset.chat_model_id
                    && vision_matches
                    && config.embedding_model_id == preset.embedding_model_id
                {
                    return tier;
                }
            }
        }
    }

    for preset in all_presets() {
        let vision_matches = if matches!(preset.vision_mode, VisionMode::Disabled) {
            config.vision_model_id.trim().is_empty()
        } else {
            config.vision_model_id == preset.vision_model_id
        };
        if config.chat_model_id == preset.chat_model_id
            && vision_matches
            && config.embedding_model_id == preset.embedding_model_id
        {
            return preset.tier;
        }
    }

    ModelTier::Custom
}

#[cfg(test)]
#[path = "presets_tests.rs"]
mod presets_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn test_device(total_ram_gb: u64) -> DeviceProfile {
        DeviceProfile {
            total_ram_bytes: total_ram_gb * 1024 * 1024 * 1024,
            cpu_count: 4,
            cpu_brand: String::new(),
            os_name: String::new(),
            os_version: String::new(),
            has_gpu: false,
            gpu_description: None,
        }
    }

    #[test]
    fn recommend_tier_scales_with_ram() {
        assert_eq!(recommend_tier(&test_device(1)), ModelTier::Ram2To4Gb);
        assert_eq!(recommend_tier(&test_device(3)), ModelTier::Ram2To4Gb);
        assert_eq!(recommend_tier(&test_device(4)), ModelTier::Ram2To4Gb);
        assert_eq!(recommend_tier(&test_device(8)), ModelTier::Ram2To4Gb);
        assert_eq!(recommend_tier(&test_device(32)), ModelTier::Ram2To4Gb);
    }

    #[test]
    fn mvp_allowed_tiers() {
        assert!(!ModelTier::Ram1Gb.is_mvp_allowed());
        assert!(ModelTier::Ram2To4Gb.is_mvp_allowed());
        assert!(!ModelTier::Ram4To8Gb.is_mvp_allowed());
        assert!(!ModelTier::Ram8To16Gb.is_mvp_allowed());
        assert!(!ModelTier::Ram16PlusGb.is_mvp_allowed());
        assert!(!ModelTier::Custom.is_mvp_allowed());
    }

    #[test]
    fn mvp_presets_only_returns_allowed_tiers() {
        let presets = mvp_presets();
        assert_eq!(presets.len(), 1);
        assert_eq!(presets[0].tier, ModelTier::Ram2To4Gb);
    }

    #[test]
    fn preset_application_and_round_trip() {
        let mut config = LocalAiConfig::default();
        apply_preset_to_config(&mut config, ModelTier::Ram2To4Gb);
        assert_eq!(config.chat_model_id, "gemma3:1b-it-qat");
        assert_eq!(config.selected_tier, Some("ram_2_4gb".to_string()));
        assert_eq!(current_tier_from_config(&config), ModelTier::Ram2To4Gb);
        assert!(!config.preload_vision_model);
        assert_eq!(vision_mode_for_config(&config), VisionMode::Disabled);
    }

    #[test]
    fn custom_detection_when_models_dont_match() {
        let mut config = LocalAiConfig::default();
        config.chat_model_id = "some-other-model:latest".to_string();
        config.selected_tier = None;
        assert_eq!(current_tier_from_config(&config), ModelTier::Custom);
    }

    #[test]
    fn all_presets_returns_five_tiers() {
        let presets = all_presets();
        assert_eq!(presets.len(), 5);
        assert_eq!(presets[0].tier, ModelTier::Ram1Gb);
        assert_eq!(presets[1].tier, ModelTier::Ram2To4Gb);
        assert_eq!(presets[2].tier, ModelTier::Ram4To8Gb);
        assert_eq!(presets[3].tier, ModelTier::Ram8To16Gb);
        assert_eq!(presets[4].tier, ModelTier::Ram16PlusGb);
    }

    #[test]
    fn default_config_maps_to_balanced_tier() {
        let config = LocalAiConfig::default();
        assert_eq!(current_tier_from_config(&config), ModelTier::Ram2To4Gb);
        assert_eq!(vision_mode_for_config(&config), VisionMode::Disabled);
    }

    #[test]
    fn device_supports_local_ai_honors_min_ram_floor() {
        assert!(!device_supports_local_ai(&test_device(1)));
        assert!(!device_supports_local_ai(&test_device(4)));
        assert!(!device_supports_local_ai(&test_device(7)));
        assert!(device_supports_local_ai(&test_device(8)));
        assert!(device_supports_local_ai(&test_device(16)));
        assert!(device_supports_local_ai(&test_device(64)));
    }

    #[test]
    fn should_default_to_cloud_fallback_below_floor() {
        assert!(should_default_to_cloud_fallback(&test_device(1)));
        assert!(should_default_to_cloud_fallback(&test_device(4)));
        assert!(should_default_to_cloud_fallback(&test_device(7)));
        assert!(!should_default_to_cloud_fallback(&test_device(8)));
        assert!(!should_default_to_cloud_fallback(&test_device(16)));
    }

    #[test]
    fn built_in_vision_modes_match_expectations() {
        let mut config = LocalAiConfig::default();
        apply_preset_to_config(&mut config, ModelTier::Ram2To4Gb);
        assert_eq!(vision_mode_for_config(&config), VisionMode::Disabled);
        assert!(!supports_screen_summary(&config));

        apply_preset_to_config(&mut config, ModelTier::Ram4To8Gb);
        assert_eq!(vision_mode_for_config(&config), VisionMode::Ondemand);
        assert!(supports_screen_summary(&config));

        apply_preset_to_config(&mut config, ModelTier::Ram16PlusGb);
        assert_eq!(vision_mode_for_config(&config), VisionMode::Bundled);
    }

    /// GH #5055 / #5146 §1.3: every preset must name a model that actually
    /// exists on the Ollama library and is fully qualified, so `ollama pull`
    /// can fetch it and the allowlist does not silently redirect the user.
    ///
    /// The original #5055 form of this test asserted the narrower fact "no id
    /// may start with `gemma4:`", because no `gemma4` namespace existed then.
    /// Gemma 4 has since been published (`gemma4:e4b-it-q8_0` resolves against
    /// `registry.ollama.ai`), so that assertion encoded a fact that expired.
    /// The durable invariant is the shape check below plus the
    /// `preset_chat_models_are_allowlisted_and_resolve_unchanged` cross-check
    /// in `model_ids`.
    #[test]
    fn preset_model_ids_are_fully_qualified() {
        for preset in all_presets() {
            for (field, id) in [
                ("chat", preset.chat_model_id),
                ("vision", preset.vision_model_id),
                ("embedding", preset.embedding_model_id),
            ] {
                if id.is_empty() {
                    // Only `vision` is legitimately empty (vision disabled).
                    assert_eq!(
                        field, "vision",
                        "preset {:?} {field} model must not be empty",
                        preset.tier
                    );
                    continue;
                }
                // `bge-m3` is deliberately exempt rather than retagged to
                // `bge-m3:latest` (greptile, #5253). The bare id is the
                // canonical spelling across the embedding stack: it is the
                // entry in `model_ids::MVP_ALLOWED_EMBEDDING_MODELS`, the
                // `embeddings::catalog` id, and what `normalize_embed_model_id`
                // collapses `bge-m3:latest` *to*. Retagging the preset alone
                // would be silently rewritten back by
                // `enforce_mvp_embedding_allowlist`; retagging all of them is a
                // cross-cutting rename that must also migrate configs already
                // persisting `bge-m3` - disproportionate to a cosmetic tag.
                assert!(
                    id.contains(':') || id == "bge-m3",
                    "preset {:?} {field} model `{id}` must be a fully-qualified \
                     `<model>:<tag>` id",
                    preset.tier
                );
            }
        }
    }

    /// #5146 §Part 1: a preset that declares a vision mode must name a model
    /// that can actually accept images.
    ///
    /// The 16 GB+ tier previously used `gemma3n:e4b-it-q8_0` as its
    /// `vision_model_id`. Gemma 3n is text-only on Ollama, and Ollama does not
    /// reject an `images` array sent to a text-only model — it drops the
    /// images and answers from the prompt alone, so the user got a fluent,
    /// fabricated description of an image the model never saw.
    #[test]
    fn preset_vision_models_are_vision_capable() {
        use crate::openhuman::inference::vision_models::is_vision_capable;

        for preset in all_presets() {
            match preset.vision_mode {
                VisionMode::Disabled => assert!(
                    preset.vision_model_id.is_empty(),
                    "preset {:?} disables vision but names `{}`",
                    preset.tier,
                    preset.vision_model_id
                ),
                VisionMode::Ondemand | VisionMode::Bundled => assert!(
                    is_vision_capable(preset.vision_model_id),
                    "preset {:?} routes vision at `{}`, which is not vision-capable",
                    preset.tier,
                    preset.vision_model_id
                ),
            }
        }
    }

    /// The 16 GB+ tier must name an allowlisted chat model, so selecting the
    /// preset is not immediately redirected back to the default by
    /// `enforce_mvp_chat_allowlist`, and that model must be multimodal so the
    /// tier's `Bundled` vision mode is real rather than nominal.
    #[test]
    fn high_tier_preset_uses_one_multimodal_build_for_chat_and_vision() {
        use crate::openhuman::inference::vision_models::is_vision_capable;

        let preset = preset_for_tier(ModelTier::Ram16PlusGb).expect("16 GB+ preset");
        assert_eq!(preset.chat_model_id, "gemma4:e4b-it-q8_0");
        assert_eq!(preset.vision_model_id, preset.chat_model_id);
        assert!(is_vision_capable(preset.vision_model_id));
    }
}
