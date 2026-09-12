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
