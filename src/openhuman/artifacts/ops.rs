use serde_json::{json, Value};

use crate::openhuman::approval::{ApprovalChatContext, APPROVAL_CHAT_CONTEXT};
use crate::openhuman::config::Config;
use crate::openhuman::security::SecurityPolicy;
use crate::openhuman::tools::traits::Tool;
use crate::openhuman::tools::PresentationTool;
use crate::rpc::RpcOutcome;

use super::store;
use super::types::ArtifactKind;

/// Default page size for `ai_list_artifacts`.
const DEFAULT_LIMIT: usize = 50;
/// Maximum page size cap for `ai_list_artifacts`.
const MAX_LIMIT: usize = 200;

/// List artifacts in the workspace with pagination.
///
/// When `thread_id` is `Some(_)` the listing is filtered to artifacts whose
/// persisted `meta.json` was produced in that chat thread (#3226) — the
/// pagination `total` reflects the filtered set, not the workspace total,
/// so the UI's "showing N of M" tally is meaningful per-thread. Legacy
/// artifacts written before `ArtifactMeta.thread_id` existed have
/// `thread_id = None` and are excluded from thread-scoped listings.
///
/// Returns `{ "artifacts": [...], "total": N, "offset": M, "limit": L }`.
pub async fn ai_list_artifacts(
    config: &Config,
    offset: Option<usize>,
    limit: Option<usize>,
    thread_id: Option<&str>,
) -> Result<RpcOutcome<Value>, String> {
    let offset = offset.unwrap_or(0);
    let limit = limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT);
    log::debug!(
        "[artifacts] ai_list_artifacts: workspace={:?} offset={offset} limit={limit} thread_id={:?}",
        config.workspace_dir,
        thread_id,
    );

    let (artifacts, total) =
        store::list_artifacts(&config.workspace_dir, offset, limit, thread_id).await?;

    log::debug!(
        "[artifacts] ai_list_artifacts: returning {} of {total} total",
        artifacts.len()
    );

    let value = json!({
        "artifacts": artifacts,
        "total": total,
        "offset": offset,
        "limit": limit,
    });
    Ok(RpcOutcome::new(value, vec![]))
}

/// Retrieve a single artifact by ID.
///
/// Returns the serialized `ArtifactMeta` plus an `absolute_path` field
/// pointing to the full on-disk location of the artifact files.
pub async fn ai_get_artifact(
    config: &Config,
    artifact_id: &str,
) -> Result<RpcOutcome<Value>, String> {
    log::debug!(
        "[artifacts] ai_get_artifact: id={artifact_id} workspace={:?}",
        config.workspace_dir
    );

    if artifact_id.is_empty() {
        return Err("[artifacts] artifact_id must not be empty".to_string());
    }

    let meta = store::get_artifact(&config.workspace_dir, artifact_id).await?;

    // Compute absolute path for the caller's convenience.
    // Guard against a corrupt or adversarial meta.path that escapes the artifacts root.
    let artifacts_root = config.workspace_dir.join("artifacts");
    let resolved = artifacts_root.join(&meta.path);
    if !resolved.starts_with(&artifacts_root) {
        return Err(format!(
            "[artifacts] meta.path {:?} escapes artifacts root for id={artifact_id}",
            meta.path
        ));
    }
    let absolute_path = resolved.to_string_lossy().into_owned();

    let mut value =
        serde_json::to_value(&meta).map_err(|e| format!("[artifacts] serialization error: {e}"))?;
    if let Some(obj) = value.as_object_mut() {
        obj.insert(
            "absolute_path".to_string(),
            Value::String(absolute_path.clone()),
        );
    }

    log::debug!(
        "[artifacts] ai_get_artifact: found id={artifact_id} absolute_path={absolute_path}"
    );
    Ok(RpcOutcome::new(value, vec![]))
}

/// Delete an artifact and all associated files.
///
/// Returns `{ "artifact_id": "...", "deleted": true }`.
pub async fn ai_delete_artifact(
    config: &Config,
    artifact_id: &str,
) -> Result<RpcOutcome<Value>, String> {
    log::debug!(
        "[artifacts] ai_delete_artifact: id={artifact_id} workspace={:?}",
        config.workspace_dir
    );

    if artifact_id.is_empty() {
        return Err("[artifacts] artifact_id must not be empty".to_string());
    }

    store::delete_artifact(&config.workspace_dir, artifact_id).await?;

    log::debug!("[artifacts] ai_delete_artifact: deleted id={artifact_id}");
    let value = json!({
        "artifact_id": artifact_id,
        "deleted": true,
    });
    Ok(RpcOutcome::new(value, vec![]))
}

/// Re-dispatch the producing tool for a failed (or any) artifact using
/// the args persisted at create-time, reusing the original `artifact_id`
/// so the in-chat card swaps in place (#3162).
///
/// Drives the failed-card Retry affordance. The flow:
///
/// 1. Load the artifact's `meta.json`; only `presentation` artifacts are
///    regenerable today (the single producing tool that persists args).
/// 2. Load the verbatim `args.json` sidecar — absent for artifacts made
///    before #3162 or by a non-persisting producer, which surfaces as a
///    "not regenerable" error rather than a silent no-op.
/// 3. Rebuild the producer tool + a fresh [`SecurityPolicy`] from the
///    live config and run it inside both:
///    - `REGENERATE_TARGET_ID.scope(artifact_id, …)` so `create_artifact`
///      reuses the id + directory instead of minting a new one, and
///    - `APPROVAL_CHAT_CONTEXT.scope({thread_id, client_id}, …)` so the
///      Pending/Ready/Failed events route back to the originating chat
///      surface (the RPC carries no ambient chat context of its own).
///
/// The returned value is a thin ack — the card's live state is driven by
/// the socket events the re-run publishes, not by this RPC's result.
pub async fn ai_regenerate(
    config: &Config,
    artifact_id: &str,
    thread_id: &str,
    client_id: &str,
) -> Result<RpcOutcome<Value>, String> {
    log::info!(
        "[artifacts] ai_regenerate: id={artifact_id} thread_id={thread_id} client_id={client_id} workspace={:?}",
        config.workspace_dir
    );

    if artifact_id.is_empty() {
        return Err("[artifacts] artifact_id must not be empty".to_string());
    }
    if thread_id.is_empty() || client_id.is_empty() {
        return Err(
            "[artifacts] regenerate requires thread_id + client_id for event routing".to_string(),
        );
    }

    let meta = store::get_artifact(&config.workspace_dir, artifact_id).await?;
    if meta.kind != ArtifactKind::Presentation {
        return Err(format!(
            "[artifacts] regenerate is only supported for presentations (artifact id={artifact_id} is {})",
            meta.kind.as_str()
        ));
    }

    let args = store::read_artifact_args(&config.workspace_dir, artifact_id).await?;

    // A fresh policy from the live config — cheap, sync, and mirrors how
    // the agent harness builds the same tool (see `tools::ops`).
    let security = std::sync::Arc::new(SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
        &config.action_dir,
    ));
    let tool = PresentationTool::new(config.workspace_dir.clone(), security);

    let chat_ctx = ApprovalChatContext {
        thread_id: thread_id.to_string(),
        client_id: client_id.to_string(),
    };

    let result = store::REGENERATE_TARGET_ID
        .scope(
            artifact_id.to_string(),
            APPROVAL_CHAT_CONTEXT.scope(chat_ctx, async move { tool.execute(args).await }),
        )
        .await;

    match result {
        Ok(tool_result) => {
            // Even when the engine fails, `execute` returns `Ok(error)` and
            // has already published `ArtifactFailed` for the reused id, so
            // the card reflects the outcome via the socket. Report the flag
            // back so the caller can log it.
            log::info!(
                "[artifacts] ai_regenerate: id={artifact_id} re-dispatched (is_error={})",
                tool_result.is_error
            );
            let value = json!({
                "artifact_id": artifact_id,
                "regenerated": true,
                "is_error": tool_result.is_error,
            });
            Ok(RpcOutcome::new(value, vec![]))
        }
        Err(err) => Err(format!(
            "[artifacts] regenerate execution error for id={artifact_id}: {err}"
        )),
    }
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
