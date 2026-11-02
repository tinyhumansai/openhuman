//! Onboarding completion and workspace flag operations.

use crate::core::runtime::DispatchCtx;
use crate::core::Outcome;

use super::loader::fallback_workspace_dir;

/// Resolves a workspace onboarding flag, creating or checking its existence.
pub async fn workspace_onboarding_flag_resolve(
    ctx: &DispatchCtx,
    flag_name: Option<String>,
    default_name: &str,
) -> Result<Outcome<bool>, String> {
    let name = flag_name.unwrap_or_else(|| default_name.to_string());
    let trimmed = name.trim();
    if trimmed.is_empty()
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.contains("..")
    {
        return Err("Invalid onboarding flag name".to_string());
    }
    let workspace_dir = match ctx.load_config().await {
        Ok(cfg) => cfg.workspace_dir,
        Err(_) => fallback_workspace_dir(),
    };
    workspace_onboarding_flag_exists(workspace_dir, trimmed)
}

/// Checks if a specific onboarding flag file exists in the workspace.
pub fn workspace_onboarding_flag_exists(
    workspace_dir: std::path::PathBuf,
    flag_name: &str,
) -> Result<Outcome<bool>, String> {
    let trimmed = flag_name.trim();
    if trimmed.is_empty()
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.contains("..")
    {
        return Err("Invalid onboarding flag name".to_string());
    }
    Ok(Outcome::single_log(
        workspace_dir.join(trimmed).is_file(),
        "onboarding flag checked",
    ))
}
