use super::*;

#[test]
fn autosummary_debounce_blocks_repeated_calls_inside_window() {
    let mut config = Config::default();
    config.local_ai.autosummary_debounce_ms = 60_000;
    let service = LocalAiService::new(&config);

    assert!(service.should_run_memory_autosummary(&config));
    assert!(!service.should_run_memory_autosummary(&config));
}

fn test_device(ram_gb: u64) -> DeviceProfile {
    DeviceProfile {
        total_ram_bytes: ram_gb * 1024 * 1024 * 1024,
        cpu_count: 4,
        cpu_brand: String::new(),
        os_name: String::new(),
        os_version: String::new(),
        has_gpu: false,
        gpu_description: None,
    }
}

#[test]
fn bootstrap_defaults_to_disabled_on_low_ram_device() {
    let config = Config::default();
    let device = test_device(4);

    let effective = config_with_recommended_tier_if_unselected(&config, &device);

    assert!(
        !effective.local_ai.runtime_enabled,
        "local_ai.runtime_enabled must default to false on <8 GB device"
    );
}

#[test]
fn bootstrap_defaults_to_disabled_on_sufficient_ram_device() {
    // Local AI is opt-in. Even with >= 8 GB RAM, an unselected tier must
    // leave local AI disabled — the user has to explicitly turn it on.
    let config = Config::default();
    let device = test_device(16);

    let effective = config_with_recommended_tier_if_unselected(&config, &device);

    assert!(
        !effective.local_ai.runtime_enabled,
        "local_ai.runtime_enabled must default to false when no tier selected, regardless of RAM"
    );
}

#[test]
fn bootstrap_honors_opt_in_on_low_ram_device() {
    let mut config = Config::default();
    config.local_ai.selected_tier = Some("ram_2_4gb".to_string());
    config.local_ai.opt_in_confirmed = true;
    crate::openhuman::inference::presets::apply_preset_to_config(
        &mut config.local_ai,
        crate::openhuman::inference::presets::ModelTier::Ram2To4Gb,
    );
    let device = test_device(4);

    let effective = config_with_recommended_tier_if_unselected(&config, &device);

    assert!(
        effective.local_ai.runtime_enabled,
        "explicit opt-in must be honored even on low-RAM device"
    );
}

#[test]
fn bootstrap_honors_opt_in_on_sufficient_ram_device() {
    let mut config = Config::default();
    config.local_ai.selected_tier = Some("ram_2_4gb".to_string());
    config.local_ai.opt_in_confirmed = true;
    crate::openhuman::inference::presets::apply_preset_to_config(
        &mut config.local_ai,
        crate::openhuman::inference::presets::ModelTier::Ram2To4Gb,
    );
    let device = test_device(16);

    let effective = config_with_recommended_tier_if_unselected(&config, &device);

    assert!(
        effective.local_ai.runtime_enabled,
        "explicit opt-in on sufficient-RAM device must stay enabled"
    );
    assert_eq!(
        effective.local_ai.chat_model_id, config.local_ai.chat_model_id,
        "opt-in config must not be mutated"
    );
}

#[test]
fn bootstrap_overrides_stale_selected_tier_without_opt_in() {
    // Existing install (pre-MVP) had `selected_tier = "ram_2_4gb"` auto-populated
    // by old RAM-based bootstrap logic, but never went through an explicit MVP
    // opt-in. `opt_in_confirmed = false` must hard-override to disabled.
    let mut config = Config::default();
    config.local_ai.runtime_enabled = true;
    config.local_ai.selected_tier = Some("ram_2_4gb".to_string());
    config.local_ai.opt_in_confirmed = false;
    let device = test_device(16);

    let effective = config_with_recommended_tier_if_unselected(&config, &device);

    assert!(
        !effective.local_ai.runtime_enabled,
        "stale selected_tier without opt_in_confirmed must be hard-overridden to disabled"
    );
    assert_eq!(
        effective.local_ai.selected_tier.as_deref(),
        Some("ram_2_4gb"),
        "bootstrap must leave the persisted selected_tier untouched — only the effective `enabled` flips"
    );
}
