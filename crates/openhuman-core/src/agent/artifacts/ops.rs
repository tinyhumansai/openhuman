use serde_json::{json, Value};

use crate::config::Config;
use crate::core::Outcome;

// Imports used only by the `documents`-gated presentation regeneration helper
// below; when the feature is off the PresentationTool is compiled out.
#[cfg(feature = "documents")]
use crate::security::approval::ApprovalChatContext;
#[cfg(feature = "documents")]
use crate::security::SecurityPolicy;
#[cfg(feature = "documents")]
use crate::tools::PresentationTool;

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
) -> Result<Outcome<Value>, String> {
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
    Ok(Outcome::new(value, vec![]))
}

/// Retrieve a single artifact by ID.
///
/// Returns the serialized `ArtifactMeta` plus an `absolute_path` field
/// pointing to the full on-disk location of the artifact files.
pub async fn ai_get_artifact(config: &Config, artifact_id: &str) -> Result<Outcome<Value>, String> {
    log::debug!(
        "[artifacts] ai_get_artifact: id={artifact_id} workspace={:?}",
        config.workspace_dir
    );

    if artifact_id.is_empty() {
        return Err("[artifacts] artifact_id must not be empty".to_string());
    }

    let meta = store::get_artifact(&config.workspace_dir, artifact_id).await?;

    // Resolve through the escape guard (#5505): a record's `file` must sit
    // inside its recorded files folder, a legacy `meta.path` inside the
    // artifacts root. A Ready record whose file was moved or deleted outside
    // OpenHuman is an error rather than a path that points at nothing.
    let roots = super::files::FileRoots::from_config(config);
    let resolved = super::files::resolve_file(&config.workspace_dir, &meta, &roots).await?;
    if matches!(meta.status, super::types::ArtifactStatus::Ready)
        && !tokio::fs::try_exists(&resolved).await.unwrap_or(false)
    {
        return Err(super::files::missing_file_error(artifact_id, &resolved));
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
    Ok(Outcome::new(value, vec![]))
}

/// Delete an artifact and all associated files.
///
/// Returns `{ "artifact_id": "...", "deleted": true }`.
pub async fn ai_delete_artifact(
    config: &Config,
    artifact_id: &str,
) -> Result<Outcome<Value>, String> {
    log::debug!(
        "[artifacts] ai_delete_artifact: id={artifact_id} workspace={:?}",
        config.workspace_dir
    );

    if artifact_id.is_empty() {
        return Err("[artifacts] artifact_id must not be empty".to_string());
    }

    store::delete_artifact(
        &config.workspace_dir,
        super::files::FileRoots::from_config(config),
        artifact_id,
    )
    .await?;

    log::debug!("[artifacts] ai_delete_artifact: deleted id={artifact_id}");
    let value = json!({
        "artifact_id": artifact_id,
        "deleted": true,
    });
    Ok(Outcome::new(value, vec![]))
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
///    - `PresentationTool::with_regenerate_target(artifact_id)` so `create_artifact`
///      reuses the id + directory instead of minting a new one, and
///    - the chat context `{thread_id, client_id}` handed to the tool so the
///      Pending/Ready/Failed events route back to the originating chat
///      surface (the RPC carries no ambient chat context of its own).
///
/// The returned value is a thin ack — the card's live state is driven by
/// the socket events the re-run publishes, not by this RPC's result.
pub async fn ai_regenerate(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    artifact_id: &str,
    thread_id: &str,
    client_id: &str,
) -> Result<Outcome<Value>, String> {
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

    regenerate_presentation(ctx, config, artifact_id, thread_id, client_id).await
}

/// Re-run the presentation producer tool against an existing artifact id.
/// Extracted so the whole `documents`-gated tool path (PresentationTool +
/// approval scope) compiles only when the feature is on; `ai_regenerate` stays
/// a thin, always-compiled RPC handler.
#[cfg(feature = "documents")]
async fn regenerate_presentation(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    artifact_id: &str,
    thread_id: &str,
    client_id: &str,
) -> Result<Outcome<Value>, String> {
    let args = store::read_artifact_args(&config.workspace_dir, artifact_id).await?;

    // A fresh policy from the live config — cheap, sync, and mirrors how
    // the agent harness builds the same tool (see `tools::ops`).
    let security = std::sync::Arc::new(SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
        &config.action_dir,
    ));
    // The regenerate target is an explicit constructor argument of the tool
    // that produces the artifact: its `create_artifact_regenerating` call
    // reuses this id instead of minting a new one.
    let tool = PresentationTool::with_config(
        config.workspace_dir.clone(),
        super::files::FileRoots::from_config(config),
        security,
        config.clone(),
    )
    .with_regenerate_target(artifact_id);

    let chat_ctx = ApprovalChatContext {
        thread_id: thread_id.to_string(),
        client_id: client_id.to_string(),
        // No turn request_id in scope on this path (artifact regeneration is
        // not itself a chat turn).
        request_id: None,
    };

    let result = tool.run(args, Some(&chat_ctx), ctx).await;

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
            Ok(Outcome::new(value, vec![]))
        }
        Err(err) => Err(format!(
            "[artifacts] regenerate execution error for id={artifact_id}: {err}"
        )),
    }
}

/// Disabled variant: with the `documents` feature off the PresentationTool is
/// compiled out (and no presentation artifacts can exist), so report the build
/// limitation rather than pretend to regenerate.
#[cfg(not(feature = "documents"))]
async fn regenerate_presentation(
    _ctx: &crate::core::runtime::DispatchCtx,
    _config: &Config,
    artifact_id: &str,
    _thread_id: &str,
    _client_id: &str,
) -> Result<Outcome<Value>, String> {
    log::debug!(
        "[artifacts] presentation regeneration rejected for id={artifact_id}: built without the `documents` feature"
    );
    Err(format!(
        "[artifacts] presentation regeneration is unavailable for id={artifact_id}: built without the `documents` feature"
    ))
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;

/// Save the user's edit of a versioned artifact as a new operator version
/// (the preview panel's autosave). See
/// [`super::tiny_store::save_operator_version`].
pub async fn ai_save_artifact_version(
    config: &Config,
    artifact_id: &str,
    body: &str,
) -> Result<Outcome<Value>, String> {
    let saved = super::tiny_store::save_operator_version(
        &config.workspace_dir,
        super::files::FileRoots::from_config(config),
        artifact_id,
        body,
    )
    .await?;
    log::debug!(
        "[artifacts] ai_save_artifact_version id={artifact_id} version={} saved={}",
        saved.version,
        saved.saved
    );
    Ok(Outcome::new(
        json!({ "artifact_id": artifact_id, "version": saved.version, "saved": saved.saved }),
        vec![],
    ))
}

/// Share a version of an artifact through cloud file storage (the preview
/// panel's Share button): uploads it and returns its link, which is also
/// recorded on the version. A version that already has a live link of the same
/// kind returns it without uploading again.
pub async fn ai_share_artifact(
    config: &Config,
    artifact_id: &str,
    version: Option<u32>,
    public: bool,
) -> Result<Outcome<Value>, String> {
    let publisher = super::artifact_publisher(config)
        .ok_or_else(|| "sign in to share artifacts".to_string())?;
    let artifacts = super::tiny_store::artifacts_service(
        &config.workspace_dir,
        super::files::FileRoots::from_config(config),
        None,
        None,
    )
    .with_publisher(publisher);
    let options = tiny_artifacts::PublishOptions {
        public,
        ..tiny_artifacts::PublishOptions::default()
    };
    let shared = artifacts
        .publish(artifact_id, version, &options)
        .await
        .map_err(|e| e.to_string())?;
    log::debug!(
        "[artifacts] ai_share_artifact id={artifact_id} version={} public={public} reused={}",
        shared.version,
        shared.reused
    );
    Ok(Outcome::new(
        json!({
            "artifact_id": artifact_id,
            "version": shared.version,
            "url": shared.link.url,
            "public": shared.link.public,
            "expires_at_millis": shared.link.expires_at_millis,
            "reused": shared.reused,
        }),
        vec![],
    ))
}
