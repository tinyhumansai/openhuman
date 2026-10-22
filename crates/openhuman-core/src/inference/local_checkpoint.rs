//! Rollback for a local-runtime connection attempt (#8172).
//!
//! The settings screen saves `local_ai` before it can check that the runtime
//! answers, and it never reads the stored key back, so it cannot restore the
//! previous settings itself. A checkpointed update keeps the prior `local_ai`
//! section here, in the core, and returns an opaque id. Rolling back that id
//! restores the prior section only while `local_ai` still holds exactly what
//! the attempt wrote: a newer save, from another attempt or anywhere else,
//! wins over an older attempt's rollback.

use crate::config::rpc as config_rpc;
use crate::config::schema::LocalAiConfig;
use crate::config::Config;
use crate::core::Outcome;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::LazyLock;
use tokio::sync::Mutex;
use tracing::debug;

const LOG_PREFIX: &str = "[inference::local_checkpoint]";

struct Checkpoint {
    id: String,
    prior: LocalAiConfig,
    written: Value,
}

/// The latest checkpoint per config file. Keyed by `config_path`, so a
/// profile's checkpoint can only be restored into that profile's own config.
static CHECKPOINTS: LazyLock<Mutex<HashMap<PathBuf, Checkpoint>>> = LazyLock::new(Default::default);

fn section(config: &Config) -> Result<Value, String> {
    serde_json::to_value(&config.local_ai).map_err(|e| format!("serialize local_ai: {e}"))
}

/// Applies `patch` and records the prior `local_ai` for [`rollback`]. The
/// returned snapshot carries the checkpoint id as `checkpoint_id`.
pub(crate) async fn apply_with_checkpoint(
    config: &mut Config,
    patch: config_rpc::LocalAiSettingsPatch,
) -> Result<Outcome<Value>, String> {
    let mut checkpoints = CHECKPOINTS.lock().await;
    let prior = config.local_ai.clone();
    let mut outcome = config_rpc::apply_local_ai_settings(config, patch).await?;
    let id = uuid::Uuid::new_v4().to_string();
    checkpoints.insert(
        config.config_path.clone(),
        Checkpoint {
            id: id.clone(),
            prior,
            written: section(config)?,
        },
    );
    debug!("{LOG_PREFIX} checkpoint recorded id={id}");
    if let Value::Object(snapshot) = &mut outcome.value {
        snapshot.insert("checkpoint_id".to_string(), Value::String(id));
    }
    Ok(outcome)
}

/// Restores the `local_ai` section recorded under `id`. Returns `false`, and
/// changes nothing, when `id` is not this config's latest checkpoint or the
/// section has changed since that attempt wrote it.
pub(crate) async fn rollback(config: &mut Config, id: &str) -> Result<bool, String> {
    let mut checkpoints = CHECKPOINTS.lock().await;
    let Some(checkpoint) = checkpoints.get(&config.config_path) else {
        debug!("{LOG_PREFIX} rollback skipped id={id}: no checkpoint");
        return Ok(false);
    };
    if checkpoint.id != id {
        debug!("{LOG_PREFIX} rollback skipped id={id}: superseded by a newer attempt");
        return Ok(false);
    }
    if section(config)? != checkpoint.written {
        debug!("{LOG_PREFIX} rollback skipped id={id}: local_ai changed since the attempt");
        checkpoints.remove(&config.config_path);
        return Ok(false);
    }
    let Some(checkpoint) = checkpoints.remove(&config.config_path) else {
        return Ok(false);
    };
    config.local_ai = checkpoint.prior;
    config
        .save()
        .await
        .map_err(|e| format!("save config: {e}"))?;
    debug!("{LOG_PREFIX} rollback restored id={id}");
    Ok(true)
}

#[cfg(test)]
#[path = "local_checkpoint_tests.rs"]
mod tests;
