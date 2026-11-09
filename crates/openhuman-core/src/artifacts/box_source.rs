//! `create_artifact` and `update_artifact` for a sandbox that keeps boxes: a
//! file `path` names a file in the calling tenant's box, not on the host.
//!
//! With a backend that keeps one box per tenant (`openshell`; see
//! [`SandboxBackendKind::keeps_boxes`](crate::sandbox::types::SandboxBackendKind::keeps_boxes)),
//! the files an agent makes live in its box, and the host has never seen
//! them. [`BoxSourced`] wraps the artifact crate's `create_artifact`
//! tool (and likewise `update_artifact`, whose artifact kind comes from its
//! stored record): for an `image` or `file` artifact it reads the file out of the
//! caller's box ([`crate::sandbox::openshell::read_file`]), stages the bytes in
//! a folder artifacts may reference, hands the staged path to the crate's tool
//! unchanged, and removes the staged copy. Everything after that is
//! [`create_artifact`](tiny_artifacts::CreateArtifactTool) as ever: the same
//! checks, the same versioned record, the same card.
//!
//! Every other call (text artifacts, and every call under a backend that keeps
//! no boxes) goes straight to the crate's tool and reads the host disk as it
//! always did.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tiny_artifacts::{ArtifactKind, Artifacts};
use tinytools::{PermissionLevel, Tool, ToolResult};

use super::tiny_store::version_files_dir;
use crate::core::runtime::{DispatchCtx, NoTenant, Tenant};
use crate::sandbox::openshell::BoxError;
use crate::sandbox::MAX_BOX_FILE_BYTES;

/// Where a call's box files are read from, and whether they are.
#[async_trait]
pub(crate) trait BoxFiles: Send + Sync {
    /// Whether this call's sandbox keeps a box per tenant, so a file `path`
    /// names a file in it.
    ///
    /// # Errors
    ///
    /// When the configuration cannot be read: the call's source is then
    /// unknown, and it must not fall back to reading the host's disk.
    async fn keeps_boxes(&self) -> Result<bool, BoxError>;

    /// The file at `path` in `tenant`'s box.
    async fn read(
        &self,
        tenant: Result<Tenant, NoTenant>,
        path: &str,
        max_bytes: u64,
    ) -> Result<Vec<u8>, BoxError>;
}

/// The process's own boxes, for the call running under `ctx`.
pub(crate) struct SandboxBoxFiles {
    ctx: DispatchCtx,
}

impl SandboxBoxFiles {
    pub(crate) fn new(ctx: DispatchCtx) -> Self {
        Self { ctx }
    }
}

#[async_trait]
impl BoxFiles for SandboxBoxFiles {
    async fn keeps_boxes(&self) -> Result<bool, BoxError> {
        let config =
            self.ctx.load_config().await.map_err(|error| {
                BoxError::Driver(format!("cannot read the configuration: {error}"))
            })?;
        Ok(crate::sandbox::ops::sandbox_keeps_boxes(&self.ctx, &config))
    }

    async fn read(
        &self,
        tenant: Result<Tenant, NoTenant>,
        path: &str,
        max_bytes: u64,
    ) -> Result<Vec<u8>, BoxError> {
        let tenant = crate::sandbox::openshell::tenant_for_box(tenant)?;
        let config =
            self.ctx.load_config().await.map_err(|error| {
                BoxError::Driver(format!("cannot read the configuration: {error}"))
            })?;
        crate::sandbox::read_box_file(Ok(tenant), &config, path, max_bytes).await
    }
}

/// Where a box file is staged on the host: a folder under the kept copies,
/// which artifacts may always reference.
pub(crate) fn staging_dir(workspace_dir: &Path) -> PathBuf {
    version_files_dir(workspace_dir).join(".box-incoming")
}

/// `path` as a name relative to the box's work folder: an agent in the box
/// knows that folder by its absolute name, so one under it is accepted.
fn box_path(path: &str) -> &str {
    let work_folder = crate::sandbox::openshell::BOX_WORKDIR;
    path.strip_prefix(work_folder)
        .and_then(|rest| rest.strip_prefix('/'))
        .filter(|rest| !rest.is_empty())
        .unwrap_or(path)
}

/// The kind of an existing artifact, from its stored record.
#[async_trait]
pub(crate) trait RecordKinds: Send + Sync {
    /// The kind of artifact `id` names; `None` when there is no such artifact.
    ///
    /// # Errors
    ///
    /// The store's message when it cannot be read.
    async fn kind_of(&self, id: &str) -> Result<Option<ArtifactKind>, String>;
}

#[async_trait]
impl RecordKinds for Artifacts {
    async fn kind_of(&self, id: &str) -> Result<Option<ArtifactKind>, String> {
        self.get(id)
            .await
            .map(|record| record.map(|record| record.kind))
            .map_err(|error| error.to_string())
    }
}

/// Where the kind of artifact a call writes comes from.
pub(crate) enum KindSource {
    /// A new artifact: the call's own `kind`.
    Args,
    /// An existing artifact: the kind of the record `artifact_id` names.
    Record(Arc<dyn RecordKinds>),
}

/// One of the crate's artifact tools (`create_artifact` or `update_artifact`),
/// reading a file `path` from the caller's box when the sandbox keeps one.
pub(crate) struct BoxSourced {
    inner: Box<dyn Tool>,
    kinds: KindSource,
    boxes: Arc<dyn BoxFiles>,
    tenant: Result<Tenant, NoTenant>,
    staging: PathBuf,
}

impl BoxSourced {
    /// `tenant` is the calling tenant (its box is read); `staging` is the
    /// folder (see [`staging_dir`]) a box file is copied into for the crate.
    pub(crate) fn new(
        inner: impl Tool + 'static,
        kinds: KindSource,
        boxes: Arc<dyn BoxFiles>,
        tenant: Result<Tenant, NoTenant>,
        staging: PathBuf,
    ) -> Self {
        Self {
            inner: Box::new(inner),
            kinds,
            boxes,
            tenant,
            staging,
        }
    }

    /// The kind of artifact this call writes, when it can be told.
    ///
    /// # Errors
    ///
    /// When the stored record cannot be read: the call must then fail, not
    /// fall back to the host's disk.
    async fn kind(&self, args: &Value) -> Result<Option<ArtifactKind>, BoxError> {
        match &self.kinds {
            KindSource::Args => Ok(args
                .get("kind")
                .and_then(Value::as_str)
                .and_then(ArtifactKind::parse)),
            KindSource::Record(records) => {
                // A missing argument or an unknown id is the crate's tool to report.
                let Some(id) = args.get("artifact_id").and_then(Value::as_str) else {
                    return Ok(None);
                };
                records
                    .kind_of(id)
                    .await
                    .map_err(|error| BoxError::Driver(format!("cannot read the artifact: {error}")))
            }
        }
    }

    /// The file to read from the box, when this call is one that does: an
    /// `image` or `file` artifact, under a sandbox that keeps boxes. (A call
    /// the crate would reject is left to it.)
    ///
    /// # Errors
    ///
    /// When it cannot be told whether the sandbox keeps boxes.
    async fn box_file(&self, args: &Value) -> Result<Option<String>, BoxError> {
        let Some(kind) = self.kind(args).await? else {
            return Ok(None);
        };
        if !kind.holds_file() {
            return Ok(None);
        }
        // The crate takes `body` as the path too, for callers that predate `path`.
        let path = ["path", "body"]
            .iter()
            .filter_map(|key| args.get(key).and_then(Value::as_str))
            .map(str::trim)
            .find(|path| !path.is_empty());
        let Some(path) = path else {
            return Ok(None);
        };
        Ok(self
            .boxes
            .keeps_boxes()
            .await?
            .then(|| box_path(path).to_owned()))
    }
}

#[async_trait]
impl Tool for BoxSourced {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn description(&self) -> &str {
        self.inner.description()
    }

    fn parameters_schema(&self) -> Value {
        self.inner.parameters_schema()
    }

    fn permission_level(&self) -> PermissionLevel {
        self.inner.permission_level()
    }

    fn is_concurrency_safe(&self, args: &Value) -> bool {
        self.inner.is_concurrency_safe(args)
    }

    async fn execute(&self, mut args: Value) -> anyhow::Result<ToolResult> {
        let path = match self.box_file(&args).await {
            Ok(Some(path)) => path,
            Ok(None) => return self.inner.execute(args).await,
            Err(error) => return Ok(ToolResult::error(error.to_string())),
        };
        let bytes = match self
            .boxes
            .read(self.tenant.clone(), &path, MAX_BOX_FILE_BYTES)
            .await
        {
            Ok(bytes) => bytes,
            Err(error) => return Ok(ToolResult::error(error.to_string())),
        };
        let staged = match stage(&self.staging, &path, &bytes) {
            Ok(staged) => staged,
            Err(error) => {
                tracing::warn!(%error, "[artifacts] could not stage a sandbox file");
                return Ok(ToolResult::error(
                    "the file could not be copied out of the sandbox",
                ));
            }
        };
        args["path"] = Value::String(staged.file.display().to_string());
        // `staged` removes its folder when dropped, so a call cancelled while
        // the crate's tool runs leaves nothing behind either.
        self.inner.execute(args).await
    }
}

/// A box file copied onto the host for one call. Dropping it removes the copy,
/// whether the call finished or was cancelled.
struct Staged {
    dir: PathBuf,
    file: PathBuf,
}

impl Drop for Staged {
    fn drop(&mut self) {
        match std::fs::remove_dir_all(&self.dir) {
            Ok(()) => {}
            // Nothing was made yet (a failed or cancelled stage).
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                tracing::warn!(%error, "[artifacts] could not remove a staged sandbox file")
            }
        }
    }
}

/// Copy `bytes` into a folder of its own under `staging`, as `file` plus the
/// box file's extension (which decides the artifact's media type). The name is
/// generated, so a box name the host's filesystem rejects (`?` on Windows)
/// still stages.
///
/// The folder is owned by the returned [`Staged`] from before it exists, and
/// nothing here awaits, so a call cancelled at any point leaves no copy behind.
fn stage(staging: &Path, box_path: &str, bytes: &[u8]) -> std::io::Result<Staged> {
    let name = match Path::new(box_path)
        .extension()
        .and_then(|extension| extension.to_str())
        .filter(|extension| {
            extension.len() <= 16 && extension.chars().all(|c| c.is_ascii_alphanumeric())
        }) {
        Some(extension) => format!("file.{extension}"),
        None => "file".to_owned(),
    };
    let dir = staging.join(uuid::Uuid::new_v4().simple().to_string());
    let staged = Staged {
        file: dir.join(name),
        dir,
    };
    std::fs::create_dir_all(&staged.dir)?;
    std::fs::write(&staged.file, bytes)?;
    Ok(staged)
}

#[cfg(test)]
#[path = "box_source_tests.rs"]
mod tests;
