//! Additional unit tests for `local_ai::presets` — colocated test module.
//!
//! These tests complement the inline `#[cfg(test)] mod tests` block in `presets.rs`
//! and focus on coverage gaps: `from_str_opt`, `as_str`, `preset_for_tier`,
//! `vision_mode_for_tier` for every tier, and `apply_preset_to_config` runtime-enable
//! semantics.

use super::*;
use crate::openhuman::config::schema::LocalAiConfig;

// ── ModelTier helpers ─────────────────────────────────────────────────────

#[test]
fn from_str_opt_recognizes_all_canonical_names() {
    assert_eq!(ModelTier::from_str_opt("ram_1gb"), Some(ModelTier::Ram1Gb));
    assert_eq!(
        ModelTier::from_str_opt("ram_2_4gb"),
        Some(ModelTier::Ram2To4Gb)
    );
    assert_eq!(
        ModelTier::from_str_opt("ram_4_8gb"),
        Some(ModelTier::Ram4To8Gb)
    );
    assert_eq!(
        ModelTier::from_str_opt("ram_8_16gb"),
        Some(ModelTier::Ram8To16Gb)
    );
    assert_eq!(
        ModelTier::from_str_opt("ram_16_plus_gb"),
        Some(ModelTier::Ram16PlusGb)
    );
    assert_eq!(ModelTier::from_str_opt("custom"), Some(ModelTier::Custom));
}

#[test]
fn from_str_opt_recognizes_aliases() {
    assert_eq!(ModelTier::from_str_opt("1gb"), Some(ModelTier::Ram1Gb));
    assert_eq!(ModelTier::from_str_opt("low"), Some(ModelTier::Ram2To4Gb));
    assert_eq!(
        ModelTier::from_str_opt("medium"),
        Some(ModelTier::Ram8To16Gb)
    );
    assert_eq!(
        ModelTier::from_str_opt("high"),
        Some(ModelTier::Ram16PlusGb)
    );
}

#[test]
fn from_str_opt_is_case_insensitive() {
    assert_eq!(
        ModelTier::from_str_opt("RAM_2_4GB"),
        Some(ModelTier::Ram2To4Gb)
    );
    assert_eq!(ModelTier::from_str_opt("CUSTOM"), Some(ModelTier::Custom));
}

#[test]
fn from_str_opt_returns_none_for_unknown() {
    assert!(ModelTier::from_str_opt("").is_none());
    assert!(ModelTier::from_str_opt("bogus").is_none());
    assert!(ModelTier::from_str_opt("ram_999gb").is_none());
}

#[test]
fn as_str_round_trips_through_from_str_opt() {
    let tiers = [
        ModelTier::Ram1Gb,
        ModelTier::Ram2To4Gb,
        ModelTier::Ram4To8Gb,
        ModelTier::Ram8To16Gb,
        ModelTier::Ram16PlusGb,
        ModelTier::Custom,
    ];
    for tier in tiers {
        let s = tier.as_str();
        assert_eq!(
            ModelTier::from_str_opt(s),
            Some(tier),
            "as_str/from_str_opt round-trip failed for {s}"
        );
    }
}

// ── preset_for_tier ───────────────────────────────────────────────────────

#[test]
fn preset_for_tier_returns_some_for_all_non_custom_tiers() {
    let expected_tiers = [
        ModelTier::Ram1Gb,
        ModelTier::Ram2To4Gb,
        ModelTier::Ram4To8Gb,
        ModelTier::Ram8To16Gb,
        ModelTier::Ram16PlusGb,
    ];
    for tier in expected_tiers {
        let preset = preset_for_tier(tier);
        assert!(
            preset.is_some(),
            "preset_for_tier({tier:?}) should return Some"
        );
        assert_eq!(preset.unwrap().tier, tier);
    }
}

#[test]
fn preset_for_custom_returns_none() {
    assert!(preset_for_tier(ModelTier::Custom).is_none());
}

// ── vision_mode_for_tier — all 5 tiers ───────────────────────────────────

#[test]
fn vision_mode_for_tier_ram1gb_is_disabled() {
    assert_eq!(
        vision_mode_for_tier(ModelTier::Ram1Gb),
        VisionMode::Disabled
    );
}

#[test]
fn vision_mode_for_tier_ram2_4gb_is_disabled() {
    assert_eq!(
        vision_mode_for_tier(ModelTier::Ram2To4Gb),
        VisionMode::Disabled
    );
}

#[test]
fn vision_mode_for_tier_ram4_8gb_is_ondemand() {
    assert_eq!(
        vision_mode_for_tier(ModelTier::Ram4To8Gb),
        VisionMode::Ondemand
    );
}

#[test]
fn vision_mode_for_tier_ram8_16gb_is_bundled() {
    assert_eq!(
        vision_mode_for_tier(ModelTier::Ram8To16Gb),
        VisionMode::Bundled
    );
}

#[test]
fn vision_mode_for_tier_ram16plus_is_bundled() {
    assert_eq!(
        vision_mode_for_tier(ModelTier::Ram16PlusGb),
        VisionMode::Bundled
    );
}

// ── preset vision_mode metadata ───────────────────────────────────────────

#[test]
fn preset_vision_mode_matches_vision_mode_for_tier() {
    for preset in all_presets() {
        let expected = vision_mode_for_tier(preset.tier);
        assert_eq!(
            preset.vision_mode, expected,
            "preset {:?} vision_mode mismatch",
            preset.tier
        );
    }
}

#[test]
fn presets_below_4gb_have_no_vision_model_id() {
    let sub_4gb = all_presets()
        .into_iter()
        .filter(|p| matches!(p.tier, ModelTier::Ram1Gb | ModelTier::Ram2To4Gb));
    for preset in sub_4gb {
        assert!(
            preset.vision_model_id.is_empty(),
            "tier {:?} should have empty vision_model_id",
            preset.tier
        );
    }
}

#[test]
fn presets_4gb_and_above_have_vision_model_id() {
    let high_tiers = all_presets().into_iter().filter(|p| {
        matches!(
            p.tier,
            ModelTier::Ram4To8Gb | ModelTier::Ram8To16Gb | ModelTier::Ram16PlusGb
        )
    });
    for preset in high_tiers {
        assert!(
            !preset.vision_model_id.is_empty(),
            "tier {:?} should have a non-empty vision_model_id",
            preset.tier
        );
    }
}

// ── apply_preset_to_config: runtime_enabled semantics ────────────────────

#[test]
fn apply_preset_enables_runtime() {
    let mut config = LocalAiConfig {
        runtime_enabled: false,
        ..Default::default()
    };
    apply_preset_to_config(&mut config, ModelTier::Ram2To4Gb);
    assert!(
        config.runtime_enabled,
        "apply_preset_to_config should set runtime_enabled = true"
    );
}

#[test]
fn apply_preset_custom_is_noop() {
    let mut config = LocalAiConfig {
        chat_model_id: "original-model".to_string(),
        runtime_enabled: false,
        ..Default::default()
    };
    apply_preset_to_config(&mut config, ModelTier::Custom);
    // Custom tier → no-op, so none of the fields change
    assert_eq!(
        config.chat_model_id, "original-model",
        "custom tier should not change chat_model_id"
    );
    assert!(
        !config.runtime_enabled,
        "custom tier should not change runtime_enabled"
    );
}

#[test]
fn apply_preset_sets_selected_tier_marker() {
    let mut config = LocalAiConfig::default();
    apply_preset_to_config(&mut config, ModelTier::Ram4To8Gb);
    assert_eq!(
        config.selected_tier.as_deref(),
        Some("ram_4_8gb"),
        "selected_tier should be set to the tier's canonical string"
    );
}

#[test]
fn apply_preset_ram8_16gb_sets_preload_vision_model() {
    let mut config = LocalAiConfig::default();
    apply_preset_to_config(&mut config, ModelTier::Ram8To16Gb);
    assert!(
        config.preload_vision_model,
        "bundled vision tiers should set preload_vision_model"
    );
}

#[test]
fn apply_preset_ram2_4gb_does_not_set_preload_vision_model() {
    let mut config = LocalAiConfig::default();
    apply_preset_to_config(&mut config, ModelTier::Ram2To4Gb);
    assert!(
        !config.preload_vision_model,
        "disabled vision tier should not set preload_vision_model"
    );
}

// ── current_tier_from_config — selected_tier marker vs model IDs ──────────

#[test]
fn current_tier_prefers_selected_tier_when_models_match() {
    let mut config = LocalAiConfig::default();
    apply_preset_to_config(&mut config, ModelTier::Ram4To8Gb);
    // After apply the selected_tier is "ram_4_8gb" and models match the preset
    assert_eq!(current_tier_from_config(&config), ModelTier::Ram4To8Gb);
}

#[test]
fn current_tier_falls_back_to_model_scan_when_no_selected_tier() {
    let mut config = LocalAiConfig::default();
    apply_preset_to_config(&mut config, ModelTier::Ram8To16Gb);
    config.selected_tier = None; // clear the marker
                                 // The model IDs still match Ram8To16Gb so the scan should find it
    assert_eq!(current_tier_from_config(&config), ModelTier::Ram8To16Gb);
}

// ── supports_screen_summary ────────────────────────────────────────────────

#[test]
fn supports_screen_summary_false_below_4gb() {
    let mut config = LocalAiConfig::default();
    apply_preset_to_config(&mut config, ModelTier::Ram1Gb);
    assert!(!supports_screen_summary(&config));
    apply_preset_to_config(&mut config, ModelTier::Ram2To4Gb);
    assert!(!supports_screen_summary(&config));
}

#[test]
fn supports_screen_summary_true_at_4gb_and_above() {
    let mut config = LocalAiConfig::default();
    for tier in [
        ModelTier::Ram4To8Gb,
        ModelTier::Ram8To16Gb,
        ModelTier::Ram16PlusGb,
    ] {
        apply_preset_to_config(&mut config, tier);
        assert!(
            supports_screen_summary(&config),
            "tier {tier:?} should support screen summary"
        );
    }
}
