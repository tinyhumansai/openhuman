//! TinyComputer model settings: the decision model and the planner and
//! rescue models the module runs a task with.

use crate::config::{ComputerConfig, Config, DecisionModel};
use crate::core::runtime::DispatchCtx;
use crate::core::Outcome;

use super::loader::snapshot_config_json;

/// A partial update to `[computer]`. An empty model string restores the
/// module's default.
#[derive(Debug, Clone, Default)]
pub struct ComputerSettingsPatch {
    pub decision_model: Option<String>,
    pub sage_fast: Option<bool>,
    pub planner_model: Option<String>,
    pub rescue_model: Option<String>,
    pub max_rescues: Option<u32>,
}

/// The most rescues TinyComputer allows one task.
pub const MAX_RESCUES: u32 = 5;

fn parse_decision_model(raw: &str) -> Result<DecisionModel, String> {
    match raw.trim().to_ascii_lowercase().replace('-', "_").as_str() {
        "jev" => Ok(DecisionModel::Jev),
        "open_jev" | "openjev" => Ok(DecisionModel::OpenJev),
        "sage" => Ok(DecisionModel::Sage),
        other => Err(format!(
            "Unsupported decision model '{other}'. Use jev, open_jev, or sage"
        )),
    }
}

fn model_id(raw: String) -> Result<Option<String>, String> {
    let value = raw.trim();
    if value.is_empty() {
        return Ok(None);
    }
    if value.len() > 200 || value.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("model ids are at most 200 characters, without spaces".into());
    }
    Ok(Some(value.to_owned()))
}

/// Apply `update` to a copy of `[computer]`, validating every field first.
pub fn patched(
    current: &ComputerConfig,
    update: ComputerSettingsPatch,
) -> Result<ComputerConfig, String> {
    let mut computer = current.clone();
    if let Some(raw) = update.decision_model {
        computer.decision_model = parse_decision_model(&raw)?;
    }
    if let Some(fast) = update.sage_fast {
        computer.sage_fast = fast;
    }
    if let Some(raw) = update.planner_model {
        computer.planner_model = model_id(raw)?;
    }
    if let Some(raw) = update.rescue_model {
        computer.rescue_model = model_id(raw)?;
    }
    if let Some(rescues) = update.max_rescues {
        if rescues > MAX_RESCUES {
            return Err(format!("max_rescues must be 0..={MAX_RESCUES}"));
        }
        computer.max_rescues = Some(rescues);
    }
    Ok(computer)
}

/// Update `[computer]` and save.
pub async fn apply_computer_settings(
    config: &mut Config,
    update: ComputerSettingsPatch,
) -> Result<Outcome<serde_json::Value>, String> {
    config.computer = patched(&config.computer, update)?;
    tracing::debug!(
        decision_model = config.computer.decision_model.as_str(),
        rescue_model = ?config.computer.rescue_model,
        max_rescues = ?config.computer.max_rescues,
        "[config] computer settings updated"
    );
    config.save().await.map_err(|e| e.to_string())?;
    let snapshot = snapshot_config_json(config)?;
    Ok(Outcome::new(
        snapshot,
        vec![format!(
            "computer settings saved to {}",
            config.config_path.display()
        )],
    ))
}

/// Load the configuration, update `[computer]`, and save it.
pub async fn load_and_apply_computer_settings(
    ctx: &DispatchCtx,
    update: ComputerSettingsPatch,
) -> Result<Outcome<serde_json::Value>, String> {
    let mut config = ctx.load_config().await?;
    apply_computer_settings(&mut config, update).await
}

#[cfg(test)]
#[path = "computer_tests.rs"]
mod tests;
