//! Hosted orchestration configuration.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

fn default_enabled() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct OrchestrationConfig {
    /// Master switch for hosted orchestration ingest, rendering, and direct
    /// Medulla runs. Default: `true`.
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    /// Options forwarded to the paid `/orchestration/v1/run` surface.
    #[serde(default)]
    pub medulla: MedullaClientConfig,
}

impl Default for OrchestrationConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            medulla: MedullaClientConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct MedullaClientConfig {
    /// Optional prompt replacements. Empty fields retain backend defaults.
    #[serde(default)]
    pub prompt_overrides: MedullaPromptOverrides,

    /// Optional graph tuning. The backend clamps every supplied value.
    #[serde(default)]
    pub config: MedullaCycleConfig,

    /// Optional per-cycle resource limits. The backend clamps every value.
    #[serde(default)]
    pub limits: MedullaCycleLimits,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct MedullaPromptOverrides {
    pub orchestrate_system: Option<String>,
    pub reasoning_execute_system: Option<String>,
    pub orchestrate_rlm_system: Option<String>,
    pub compress_system: Option<String>,
    pub frontend_gate_system: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct MedullaCycleConfig {
    pub max_passes: Option<u32>,
    pub max_steps: Option<u32>,
    pub max_depth: Option<u32>,
    pub context_window_tokens: Option<u64>,
    pub verification: Option<MedullaVerification>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MedullaVerification {
    Remind,
    Off,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct MedullaCycleLimits {
    pub max_concurrency: Option<u32>,
    pub max_tokens: Option<u64>,
    pub deadline_ms: Option<u64>,
    pub max_tasks_per_delegate: Option<u32>,
    pub max_depth: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_keep_orchestration_enabled_without_overrides() {
        let config = OrchestrationConfig::default();
        assert!(config.enabled);
        assert!(config.medulla.config.max_passes.is_none());
        assert!(config.medulla.limits.deadline_ms.is_none());
        assert!(config.medulla.prompt_overrides.orchestrate_system.is_none());
    }

    #[test]
    fn medulla_nested_toml_deserializes() {
        let config: OrchestrationConfig = toml::from_str(
            r#"
enabled = true

[medulla.prompt_overrides]
orchestrate_system = "Plan carefully"

[medulla.config]
max_passes = 3
verification = "remind"

[medulla.limits]
max_concurrency = 8
deadline_ms = 90000
"#,
        )
        .unwrap();

        assert_eq!(config.medulla.config.max_passes, Some(3));
        assert!(matches!(
            config.medulla.config.verification,
            Some(MedullaVerification::Remind)
        ));
        assert_eq!(config.medulla.limits.max_concurrency, Some(8));
        assert_eq!(
            config
                .medulla
                .prompt_overrides
                .orchestrate_system
                .as_deref(),
            Some("Plan carefully")
        );
    }
}
