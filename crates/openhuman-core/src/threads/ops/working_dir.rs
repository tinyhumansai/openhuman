//! A thread's working folder: the `action_dir` its agent acts in, picked
//! above the composer when a conversation starts.
//!
//! The folder is bound before the first message and is fixed from then on. A
//! resumed session keeps its first system prompt and tool surface verbatim
//! (see the Sessions notes in `CLAUDE.md`), so moving a live thread to another
//! folder would leave the model describing one directory while its tools act
//! in another.

use super::support::{counts, envelope, thread_to_summary, workspace_dir};
use crate::config::Config;
use crate::core::runtime::DispatchCtx;
use crate::core::Outcome;
use crate::security::SecurityPolicy;
use crate::threads::store as conversations;
use crate::threads::ThreadsError;
use crate::threads::{
    ApiEnvelope, ConversationThreadSummary, UpdateConversationThreadWorkingDirRequest,
};
use std::path::{Path, PathBuf};

/// Validates a requested working folder and returns its canonical form, or
/// `None` for an empty request (use the global `action_dir`).
///
/// The folder must be an existing absolute directory outside the paths the
/// security policy never opens (credential stores, system roots), the same
/// floor that holds with the autonomy policy disabled. Like the global
/// `action_dir`, it must neither sit inside nor contain OpenHuman's own state.
pub(crate) async fn validate_working_dir(
    raw: Option<&str>,
    config: &Config,
) -> Result<Option<String>, String> {
    let Some(raw) = raw.map(str::trim).filter(|raw| !raw.is_empty()) else {
        return Ok(None);
    };
    if raw.contains('\0') {
        return Err("working folder must not contain a null byte".to_string());
    }
    let path = Path::new(raw);
    if !path.is_absolute() {
        return Err("working folder must be an absolute path".to_string());
    }
    let canonical = path
        .canonicalize()
        .map_err(|err| format!("working folder is not accessible: {err}"))?;
    if !canonical.is_dir() {
        return Err("working folder is not a directory".to_string());
    }
    if SecurityPolicy::is_always_forbidden_async(&canonical).await {
        return Err("working folder is in a protected location".to_string());
    }
    crate::config::ops::refuse_data_folder("working folder", &canonical, raw, config)?;
    Ok(Some(canonical.to_string_lossy().into_owned()))
}

/// [`validate_working_dir`], also refusing the workspace the thread store
/// uses. In single-user mode the store's `workspace_dir()` (the disk config)
/// can differ from the dispatch context's config.
pub(crate) async fn validate_working_dir_for_store(
    raw: Option<&str>,
    config: &Config,
    store: &Path,
) -> Result<Option<String>, String> {
    let validated = validate_working_dir(raw, config).await?;
    if let Some(dir) = &validated {
        if store != config.workspace_dir {
            let mut store_config = config.clone();
            store_config.workspace_dir = store.to_path_buf();
            crate::config::ops::refuse_data_folder(
                "working folder",
                Path::new(dir),
                dir,
                &store_config,
            )?;
        }
    }
    Ok(validated)
}

/// The working folder bound to `thread_id`, if the thread exists and has one.
///
/// An unusable bound folder is an error, rather than a reason to run in the
/// global folder. This preserves the conversation's filesystem boundary.
pub(crate) async fn thread_working_dir(
    ctx: &DispatchCtx,
    config: &Config,
    workspace_dir: PathBuf,
    thread_id: &str,
) -> Result<Option<PathBuf>, String> {
    let threads = conversations::blocking::list_threads(ctx.clone(), workspace_dir.clone())
        .await
        .map_err(|err| format!("failed to load thread working folder: {err}"))?;
    let bound = threads
        .into_iter()
        .find(|thread| thread.id == thread_id)
        .and_then(|thread| thread.working_dir);
    let Some(bound) = bound else {
        return Ok(None);
    };
    match validate_working_dir_for_store(Some(&bound), config, &workspace_dir).await {
        Ok(Some(dir)) => Ok(Some(PathBuf::from(dir))),
        Ok(None) => Ok(None),
        Err(err) => Err(format!("thread working folder is unusable: {err}")),
    }
}

/// Binds or clears the working folder of a thread that has no messages yet.
pub async fn thread_update_working_dir(
    ctx: &DispatchCtx,
    request: UpdateConversationThreadWorkingDirRequest,
) -> Result<Outcome<ApiEnvelope<ConversationThreadSummary>>, ThreadsError> {
    let dir = workspace_dir().await?;
    let config = ctx
        .load_config()
        .await
        .map_err(|e| format!("load config: {e}"))?;
    let working_dir =
        validate_working_dir_for_store(Some(&request.action_dir), &config, &dir).await?;
    let Some(thread) = conversations::blocking::list_threads(ctx.clone(), dir.clone())
        .await?
        .into_iter()
        .find(|thread| thread.id == request.thread_id)
    else {
        return Err(ThreadsError::not_found(request.thread_id));
    };
    if thread.message_count > 0 {
        return Err(
            "the working folder can only change before the conversation's first message"
                .to_string()
                .into(),
        );
    }
    let updated = conversations::blocking::update_thread_working_dir(
        ctx.clone(),
        dir,
        request.thread_id.clone(),
        working_dir,
        chrono::Utc::now().to_rfc3339(),
    )
    .await
    .map_err(|err| ThreadsError::from_thread_scoped_store_error(&request.thread_id, err))?;
    tracing::debug!(
        thread_id = %request.thread_id,
        bound = updated.working_dir.is_some(),
        "[threads][working_dir] updated thread working folder"
    );
    Ok(envelope(
        thread_to_summary(updated),
        Some(counts([("num_threads", 1)])),
        None,
    ))
}

#[cfg(test)]
#[path = "working_dir_tests.rs"]
mod tests;
