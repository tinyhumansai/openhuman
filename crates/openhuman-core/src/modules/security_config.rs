//! Module source settings bound to the authenticated dispatch, without auth,
//! environment or operator fallback in SaaS mode.
use super::*;

pub(super) async fn host_config(workspace: &std::path::Path, saas: bool) -> Result<Config, String> {
    if !saas {
        return match crate::core::runtime::CoreContext::current_embedder_config() {
            Some(config) => Ok(config),
            None => crate::config::ops::load_config_for_workspace_with_timeout(workspace).await,
        };
    }
    let context = crate::core::runtime::CoreContext::scoped()
        .ok_or("TinySecurity requires an authenticated SaaS dispatch scope")?;
    if context
        .workspace_dir()
        .map_err(|_| "TinySecurity SaaS workspace unavailable")?
        != workspace
    {
        return Err("TinySecurity SaaS workspace does not match authenticated scope".into());
    }
    if let Some(config) = context.embedder_config() {
        return Ok(config.clone());
    }
    // No native call has started: missing authority/configuration refuses this
    // request but must not latch the process-wide module client as faulted.
    tokio::time::timeout(LOAD_TIMEOUT, scoped_module_file(workspace))
        .await
        .map_err(|_| "TinySecurity SaaS configuration loading timed out")?
}

#[derive(Default, serde::Deserialize)]
#[serde(default)]
struct ModuleDocument {
    modules: crate::config::schema::ModulesConfig,
}

async fn scoped_module_file(workspace: &std::path::Path) -> Result<Config, String> {
    // Only the known account/workspace layout permits looking beside the
    // workspace. Arbitrary workspaces never inherit a common parent's config.
    let account_config = (workspace.file_name() == Some(std::ffi::OsStr::new("workspace")))
        .then(|| workspace.parent().map(|parent| parent.join("config.toml")))
        .flatten();
    for candidate in [Some(workspace.join("config.toml")), account_config]
        .into_iter()
        .flatten()
    {
        let text = match tokio::fs::read_to_string(&candidate).await {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return Err("TinySecurity SaaS configuration unreadable".into()),
        };
        // Generic Config reloads can synthesize defaults and apply process env
        // when a source disappears. Parse only module settings from these exact
        // bytes, so a missing/invalid tenant source cannot import operator state.
        let document = tokio::task::spawn_blocking(move || toml::from_str::<ModuleDocument>(&text))
            .await
            .map_err(|_| "TinySecurity SaaS configuration parsing failed")?
            .map_err(|_| "TinySecurity SaaS configuration invalid")?;
        return Ok(Config {
            modules: document.modules,
            workspace_dir: workspace.to_path_buf(),
            action_dir: workspace.to_path_buf(),
            config_path: candidate,
            ..Config::default()
        });
    }
    Err("TinySecurity SaaS configuration missing from authenticated workspace".into())
}

#[cfg(test)]
#[path = "security_config_tests.rs"]
mod tests;
