//! Model evaluation for `prompt` hooks.
//!
//! A prompt hook is a policy written in English rather than in shell. It costs
//! a model call per event, so it belongs on rare, high-stakes moments — a
//! destructive shell command, a subagent about to be spawned — not on every
//! tool call.
//!
//! The model override in a hook definition is applied by cloning the loaded
//! config and replacing `default_model`, because the one-shot prompt entry
//! point takes its model from the config rather than from an argument. Cloning
//! keeps the override scoped to this evaluation: nothing persists, and a
//! concurrent turn on the real config is unaffected.

use crate::openhuman::config::schema::Config;

/// Ask the model to judge a hook condition, returning its raw answer.
///
/// Errors are strings rather than `anyhow` because every caller folds them into
/// the fail-open / fail-closed decision, where the only thing that matters is
/// the message.
pub async fn evaluate(instruction: &str, model: Option<&str>) -> Result<String, String> {
    let mut config = crate::openhuman::config::rpc::load_config_with_timeout()
        .await
        .map_err(|error| format!("loading config for prompt hook: {error}"))?;
    if let Some(model) = model {
        apply_model_override(&mut config, model);
    }
    let outcome = crate::openhuman::inference::ops::inference_prompt(
        &config,
        instruction,
        Some(MAX_VERDICT_TOKENS),
        Some(true),
    )
    .await
    .map_err(|error| format!("prompt hook evaluation failed: {error}"))?;
    Ok(outcome.value)
}

/// A verdict is a two-field JSON object; anything longer is the model
/// rambling, and cutting it off early is cheaper than reading it.
const MAX_VERDICT_TOKENS: u32 = 200;

fn apply_model_override(config: &mut Config, model: &str) {
    config.default_model = Some(model.to_string());
}
