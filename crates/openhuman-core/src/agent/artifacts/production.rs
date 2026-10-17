//! [`Production`]: how a producer tool (a deck, a document, a generated image
//! or video) files what it makes as a versioned artifact.
//!
//! Every artifact goes through the `tiny-artifacts` service; a producer only
//! adds the part that is OpenHuman's — reserving a file name in the visible
//! files folder, so it can write its bytes there while the user already sees
//! a "generating…" card. The flow:
//!
//! 1. [`Production::start`] reserves the file and the card, and opens the
//!    versioned record under the same id. A revision (`revise`: an
//!    `artifact_id` argument or a regenerate target) reuses the existing id
//!    and file, and keeps the record's history.
//! 2. The producer writes its bytes to [`Production::path`].
//! 3. [`Production::complete`] appends the file as a new agent version; the UI
//!    bridge marks the card ready. [`Production::fail`] fails both instead.
//!
//! The record is written before the card is marked ready, so a ready card
//! always has its versioned record.

use std::path::{Path, PathBuf};

use tiny_artifacts::{ArtifactKind, Artifacts, Author, Content};

use super::files::FileRoots;
use super::tiny_store::{artifacts_service, legacy_kind_for_extension};
use super::types::ArtifactKind as LegacyArtifactKind;
use crate::security::approval::ApprovalChatContext;

/// Starts the card's failure reason when a version was recorded but the card
/// could not show it (its file in the files folder was not updated). The UI
/// keys on it to say so, instead of "generation failed".
pub const SAVED_NOT_SHOWN: &str = "saved-not-shown:";

/// A file being produced as an artifact.
pub struct Production {
    artifacts: Artifacts,
    workspace_dir: PathBuf,
    files: FileRoots,
    chat: Option<ApprovalChatContext>,
    id: String,
    path: PathBuf,
    /// Whether this adds a version to an artifact that already has one: its
    /// failure must leave that artifact as it was, not fail it.
    revision: bool,
}

impl std::fmt::Debug for Production {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Production")
            .field("id", &self.id)
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl Production {
    /// Reserve the file (`<title>.<extension>` in the files folder) and open
    /// the artifact. `revise` names an existing artifact to add a version to;
    /// otherwise a new artifact is made. `tool_call_id` ties the card to the
    /// producing call's bubble; `chat` is the turn the card is announced to
    /// (`None` outside a chat turn).
    ///
    /// # Errors
    ///
    /// Returns an error when the file cannot be reserved or the record cannot
    /// be opened; the card is failed in the second case.
    pub async fn start(
        workspace_dir: &Path,
        files: FileRoots,
        kind: ArtifactKind,
        title: &str,
        extension: &str,
        tool_call_id: Option<&str>,
        revise: Option<&str>,
        chat: Option<&ApprovalChatContext>,
    ) -> Result<Self, String> {
        let legacy_kind = if kind == ArtifactKind::Image {
            LegacyArtifactKind::Image
        } else {
            legacy_kind_for_extension(&extension.to_ascii_lowercase())
        };
        let target = revise.filter(|id| !id.trim().is_empty());
        let (meta, path) = super::store::create_artifact_regenerating(
            workspace_dir,
            files.clone(),
            legacy_kind,
            title,
            extension,
            tool_call_id,
            target,
            chat,
        )
        .await?;

        let artifacts = artifacts_service(workspace_dir, files.clone(), None, chat);
        let mut production = Self {
            artifacts,
            workspace_dir: workspace_dir.to_path_buf(),
            files,
            chat: chat.cloned(),
            id: meta.id.clone(),
            path,
            revision: false,
        };
        // A revision keeps its record and history; a new artifact opens one.
        let existing = production
            .artifacts
            .get(&meta.id)
            .await
            .map_err(|e| e.to_string());
        let opened = match existing {
            Ok(Some(record)) => {
                production.revision = !record.versions.is_empty();
                Ok(())
            }
            Ok(None) => production
                .artifacts
                .begin(kind, title, Some(meta.id.clone()))
                .await
                .map(|_| ())
                .map_err(|e| e.to_string()),
            Err(err) => Err(err),
        };
        if let Err(err) = opened {
            let reason = format!("could not open the artifact record: {err}");
            production.fail_card(&reason).await;
            return Err(reason);
        }
        Ok(production)
    }

    /// The artifact's id.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Where the producer writes its bytes.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// File the written bytes as a new version and mark the card ready.
    /// Returns the file's size.
    ///
    /// # Errors
    ///
    /// Returns an error when the version cannot be recorded or the card cannot
    /// be marked ready; the card is failed then.
    pub async fn complete(&self) -> Result<u64, String> {
        let path = self.path.display().to_string();
        let saved = self
            .artifacts
            .revise(
                &self.id,
                Content::File(path),
                &Author::agent(super::tiny_store::AGENT_AUTHOR_ID),
                None,
            )
            .await;
        match saved {
            Ok(saved) => match (
                saved.notice,
                saved.record.latest().and_then(|v| v.file.clone()),
            ) {
                (None, Some(file)) => Ok(file.size_bytes),
                // The version is recorded, so the production succeeded; only
                // the card (and the file in the files folder) could not follow.
                // The card says exactly that.
                (Some(notice), Some(file)) => {
                    tracing::warn!(
                        target: "tiny-artifacts",
                        artifact_id = %self.id,
                        version = saved.version,
                        notice = %notice,
                        "[artifacts] version saved but not shown"
                    );
                    self.fail_card(&format!(
                        "{SAVED_NOT_SHOWN} version {} is saved, but {notice}",
                        saved.version
                    ))
                    .await;
                    Ok(file.size_bytes)
                }
                (_, None) => {
                    let reason = "failed to finalize artifact: the recorded version has no file";
                    self.fail_card(reason).await;
                    Err(reason.to_string())
                }
            },
            // Nothing was recorded: end it as any failed production ends.
            Err(err) => {
                let reason = format!("failed to finalize artifact: {err}");
                self.fail(&reason).await;
                Err(reason)
            }
        }
    }

    /// End the production with `reason`. A new artifact is failed, record and
    /// card. A revision leaves the artifact as it was: the record keeps its
    /// versions and the card shows the newest of them again.
    pub async fn fail(&self, reason: &str) {
        if self.revision {
            self.restore_card(reason).await;
            return;
        }
        let shown = match self.artifacts.fail(&self.id, reason).await {
            Ok(saved) => saved.notice,
            Err(err) => Some(err.to_string()),
        };
        // The record took the failure but the card did not follow (or the
        // record could not take it): fail the card directly, so it never stays
        // "generating".
        if let Some(why) = shown {
            tracing::warn!(
                target: "tiny-artifacts",
                artifact_id = %self.id,
                err = %why,
                "[artifacts] the failure did not reach the card; failing it directly"
            );
            self.fail_card(reason).await;
        }
    }

    /// Show a revised artifact's newest recorded version again after its
    /// revision failed. Failing the card is the fallback when that is not
    /// possible.
    async fn restore_card(&self, reason: &str) {
        tracing::warn!(
            target: "tiny-artifacts",
            artifact_id = %self.id,
            reason,
            "[artifacts] revision failed; keeping the artifact's newest version"
        );
        let record = match self.artifacts.get(&self.id).await {
            Ok(Some(record)) if record.latest().is_some() => record,
            _ => return self.fail_card(reason).await,
        };
        let bridge = super::tiny_store::LegacyUiBridge::new(
            &self.workspace_dir,
            self.files.clone(),
            self.chat.as_ref(),
        );
        if let Err(err) = bridge.show_newest(&record).await {
            tracing::warn!(
                target: "tiny-artifacts",
                artifact_id = %self.id,
                err = %err,
                "[artifacts] could not show the newest version again"
            );
            self.fail_card(reason).await;
        }
    }

    /// Fail the card directly (when the record could not take the change).
    async fn fail_card(&self, reason: &str) {
        let _ = super::store::fail_artifact(
            &self.workspace_dir,
            self.files.clone(),
            &self.id,
            reason,
            self.chat.as_ref(),
        )
        .await;
    }
}

#[cfg(test)]
#[path = "production_tests.rs"]
mod tests;
