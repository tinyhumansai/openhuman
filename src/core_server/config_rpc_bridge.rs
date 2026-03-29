//! Maps `core_server::types` JSON-RPC DTOs to `openhuman::config::rpc` patches in one place
//! so dispatch and CLI stay thin.

use crate::core_server::types::{
    BrowserSettingsUpdate, MemorySettingsUpdate, ModelSettingsUpdate, RuntimeSettingsUpdate,
    ScreenIntelligenceSettingsUpdate,
};
use crate::openhuman::config::rpc::{
    BrowserSettingsPatch, MemorySettingsPatch, ModelSettingsPatch, RuntimeSettingsPatch,
    ScreenIntelligenceSettingsPatch,
};

impl From<ModelSettingsUpdate> for ModelSettingsPatch {
    fn from(u: ModelSettingsUpdate) -> Self {
        Self {
            api_key: u.api_key,
            api_url: u.api_url,
            default_provider: u.default_provider,
            default_model: u.default_model,
            default_temperature: u.default_temperature,
        }
    }
}

impl From<MemorySettingsUpdate> for MemorySettingsPatch {
    fn from(u: MemorySettingsUpdate) -> Self {
        Self {
            backend: u.backend,
            auto_save: u.auto_save,
            embedding_provider: u.embedding_provider,
            embedding_model: u.embedding_model,
            embedding_dimensions: u.embedding_dimensions,
        }
    }
}

impl From<RuntimeSettingsUpdate> for RuntimeSettingsPatch {
    fn from(u: RuntimeSettingsUpdate) -> Self {
        Self {
            kind: u.kind,
            reasoning_enabled: u.reasoning_enabled,
        }
    }
}

impl From<BrowserSettingsUpdate> for BrowserSettingsPatch {
    fn from(u: BrowserSettingsUpdate) -> Self {
        Self { enabled: u.enabled }
    }
}

impl From<ScreenIntelligenceSettingsUpdate> for ScreenIntelligenceSettingsPatch {
    fn from(u: ScreenIntelligenceSettingsUpdate) -> Self {
        Self {
            enabled: u.enabled,
            capture_policy: u.capture_policy,
            policy_mode: u.policy_mode,
            baseline_fps: u.baseline_fps,
            vision_enabled: u.vision_enabled,
            autocomplete_enabled: u.autocomplete_enabled,
            allowlist: u.allowlist,
            denylist: u.denylist,
        }
    }
}
