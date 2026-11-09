//! Composio trigger settings exposed through the config controller.

use crate::config::Config;
use crate::core::runtime::DispatchCtx;
use crate::core::Outcome;

use super::loader::snapshot_config_json;

#[derive(Debug, Clone, Default)]
pub struct ComposioTriggerSettingsPatch {
    /// When `Some(true)`, disables triage for all toolkits.
    pub triage_disabled: Option<bool>,
    /// When `Some(v)`, replaces the per-toolkit opt-out list entirely.
    pub triage_disabled_toolkits: Option<Vec<String>>,
}

/// Updates the Composio trigger-triage settings in the configuration.
pub async fn apply_composio_trigger_settings(
    config: &mut Config,
    update: ComposioTriggerSettingsPatch,
) -> Result<Outcome<serde_json::Value>, String> {
    if let Some(v) = update.triage_disabled {
        config.composio.triage_disabled = v;
        tracing::debug!(
            triage_disabled = v,
            "[config][composio] triage_disabled updated"
        );
    }
    if let Some(toolkits) = update.triage_disabled_toolkits {
        tracing::debug!(
            count = toolkits.len(),
            "[config][composio] triage_disabled_toolkits updated"
        );
        config.composio.triage_disabled_toolkits = toolkits;
    }
    config.save().await.map_err(super::config_save_error)?;
    let snapshot = snapshot_config_json(config).await?;
    Ok(Outcome::new(
        snapshot,
        vec![format!(
            "composio trigger settings saved to {}",
            config.config_path.display()
        )],
    ))
}

/// Loads the configuration, applies composio trigger settings, and saves it.
pub async fn load_and_apply_composio_trigger_settings(
    ctx: &DispatchCtx,
    update: ComposioTriggerSettingsPatch,
) -> Result<Outcome<serde_json::Value>, String> {
    let mut config = ctx.load_config().await?;
    apply_composio_trigger_settings(&mut config, update).await
}

/// Reads the current composio trigger-triage settings.
pub async fn get_composio_trigger_settings(
    ctx: &DispatchCtx,
) -> Result<Outcome<serde_json::Value>, String> {
    let config = ctx.load_config().await?;
    let result = serde_json::json!({
        "triage_disabled": config.composio.triage_disabled,
        "triage_disabled_toolkits": config.composio.triage_disabled_toolkits,
    });
    Ok(Outcome::new(
        result,
        vec!["composio trigger settings read".to_string()],
    ))
}

#[cfg(test)]
#[path = "composio_tests.rs"]
mod tests;
