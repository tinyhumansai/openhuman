//! Handlers for the Claude Code provider: version and auth probes, and its
//! full-access setting.

use crate::core::runtime::DispatchCtx;
use serde::Deserialize;
use serde_json::{Map, Value};

use super::{deserialize_params, to_json};
use crate::core::all::ControllerFuture;
use crate::core::Outcome;

#[derive(Debug, Deserialize)]
pub(super) struct InferenceClaudeCodeSetFullAccessParams {
    /// true → full access (`bypassPermissions` + full toolset); false → the
    /// default `acceptEdits` posture (file edits only).
    enabled: bool,
}

pub(super) fn handle_inference_claude_code_status(
    _ctx: DispatchCtx,
    _params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let status = tokio::task::spawn_blocking(
            tinyagents_harness::providers::claude_code::version_check::probe,
        )
        .await
        .map_err(|e| format!("claude_code_status join error: {e}"))?;
        to_json(Outcome::new(status, vec![]))
    })
}

pub(super) fn handle_inference_claude_code_auth_status(
    _ctx: DispatchCtx,
    _params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let auth = tokio::task::spawn_blocking(
            tinyagents_harness::providers::claude_code::auth_status::probe,
        )
        .await
        .map_err(|e| format!("claude_code_auth_status join error: {e}"))?;
        to_json(Outcome::new(auth, vec![]))
    })
}

pub(super) fn handle_inference_claude_code_settings(
    ctx: DispatchCtx,
    _params: Map<String, Value>,
) -> ControllerFuture {
    use tinyagents_harness::providers::claude_code::settings;
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let opened = crate::storage::local::open("claude_code::settings", &config.workspace_dir)
            .map_err(|error| error.to_string())?;
        let scoped = opened.scoped().map_err(|error| error.to_string())?;
        let kv = settings::store(scoped.documents()).map_err(|error| error.to_string())?;
        let settings = settings::load(&kv)
            .await
            .map_err(|error| error.to_string())?;
        log::debug!(
            "[rpc][inference.claude_code_settings] full_access={}",
            settings.full_access
        );
        to_json(Outcome::new(settings, vec![]))
    })
}

pub(super) fn handle_inference_claude_code_set_full_access(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    use tinyagents_harness::providers::claude_code::settings;
    Box::pin(async move {
        let p = deserialize_params::<InferenceClaudeCodeSetFullAccessParams>(params)?;
        let config = ctx.load_config().await?;
        let opened = crate::storage::local::open("claude_code::settings", &config.workspace_dir)
            .map_err(|error| error.to_string())?;
        let scoped = opened.scoped().map_err(|error| error.to_string())?;
        let kv = settings::store(scoped.documents()).map_err(|error| error.to_string())?;
        let settings = settings::ClaudeCodeSettings {
            full_access: p.enabled,
        };
        settings::save(&kv, &settings)
            .await
            .map_err(|e| format!("failed to persist claude code settings: {e}"))?;
        log::info!(
            "[rpc][inference.claude_code_set_full_access] persisted full_access={}",
            settings.full_access
        );
        to_json(Outcome::new(settings, vec![]))
    })
}
