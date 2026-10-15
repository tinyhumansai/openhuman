//! Durable inference choices for unattended continuation after app restart.
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use tinyagents_harness::store::Store;

#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Preferences {
    pub model: Option<String>,
    pub reasoning: Option<String>,
    pub temperature: Option<f64>,
}
fn db(root: &Path) -> Arc<dyn Store> {
    crate::agent::session_store::stores_or_ephemeral(root).kv
}
fn key(thread: &str) -> String {
    thread.bytes().map(|b| format!("{b:02x}")).collect()
}
pub(crate) async fn save(
    root: &Path,
    thread: &str,
    preferences: &Preferences,
) -> anyhow::Result<()> {
    db(root)
        .put(
            "inference-preferences",
            &key(thread),
            serde_json::to_value(preferences)?,
        )
        .await
        .map_err(Into::into)
}
pub(crate) async fn load(root: &Path, thread: &str) -> anyhow::Result<Option<Preferences>> {
    db(root)
        .get("inference-preferences", &key(thread))
        .await?
        .map(serde_json::from_value)
        .transpose()
        .map_err(Into::into)
}
#[cfg(test)]
#[path = "preferences_tests.rs"]
mod tests;
