//! `create_artifact` for a sandbox that keeps boxes: a file `path` names a file
//! in the calling tenant's box, not on the host.
//!
//! With a backend that keeps one box per tenant (`openshell`; see
//! [`SandboxBackendKind::keeps_boxes`](crate::sandbox::types::SandboxBackendKind::keeps_boxes)),
//! the files an agent makes live in its box, and the host has never seen
//! them. [`BoxSourcedCreate`] wraps the artifact crate's `create_artifact`
//! tool: for an `image` or `file` artifact it reads the file out of the
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
use tiny_artifacts::{ArtifactKind, CreateArtifactTool};
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
    async fn keeps_boxes(&self) -> bool;

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
    async fn keeps_boxes(&self) -> bool {
        match self.ctx.load_config().await {
            Ok(config) => crate::sandbox::ops::sandbox_keeps_boxes(&self.ctx, &config),
            // Without a configuration there is no sandbox to ask: read the host.
            Err(_) => false,
        }
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

/// The crate's `create_artifact`, reading a file `path` from the caller's box
/// when the sandbox keeps one.
pub(crate) struct BoxSourcedCreate {
    inner: CreateArtifactTool,
    boxes: Arc<dyn BoxFiles>,
    tenant: Result<Tenant, NoTenant>,
    staging: PathBuf,
}

impl BoxSourcedCreate {
    /// `tenant` is the calling tenant (its box is read); `staging` is the
    /// folder (see [`staging_dir`]) a box file is copied into for the crate.
    pub(crate) fn new(
        inner: CreateArtifactTool,
        boxes: Arc<dyn BoxFiles>,
        tenant: Result<Tenant, NoTenant>,
        staging: PathBuf,
    ) -> Self {
        Self {
            inner,
            boxes,
            tenant,
            staging,
        }
    }

    /// The file to read from the box, when this call is one that does: an
    /// `image` or `file` artifact, under a sandbox that keeps boxes. (A call
    /// the crate would reject is left to it.)
    async fn box_file(&self, args: &Value) -> Option<String> {
        let kind = ArtifactKind::parse(args.get("kind")?.as_str()?)?;
        if !kind.holds_file() {
            return None;
        }
        // The crate takes `body` as the path too, for callers that predate `path`.
        let path = ["path", "body"]
            .iter()
            .filter_map(|key| args.get(key).and_then(Value::as_str))
            .map(str::trim)
            .find(|path| !path.is_empty())?;
        self.boxes
            .keeps_boxes()
            .await
            .then(|| box_path(path).to_owned())
    }
}

#[async_trait]
impl Tool for BoxSourcedCreate {
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
        let Some(path) = self.box_file(&args).await else {
            return self.inner.execute(args).await;
        };
        let bytes = match self
            .boxes
            .read(self.tenant.clone(), &path, MAX_BOX_FILE_BYTES)
            .await
        {
            Ok(bytes) => bytes,
            Err(error) => return Ok(ToolResult::error(error.to_string())),
        };
        let staged = match stage(&self.staging, &path, &bytes).await {
            Ok(staged) => staged,
            Err(error) => {
                tracing::warn!(%error, "[artifacts] could not stage a sandbox file");
                return Ok(ToolResult::error(
                    "the file could not be copied out of the sandbox",
                ));
            }
        };
        args["path"] = Value::String(staged.file.display().to_string());
        let created = self.inner.execute(args).await;
        if let Err(error) = tokio::fs::remove_dir_all(&staged.dir).await {
            tracing::warn!(%error, "[artifacts] could not remove a staged sandbox file");
        }
        created
    }
}

/// A box file copied onto the host for one call.
struct Staged {
    dir: PathBuf,
    file: PathBuf,
}

/// Copy `bytes` into a folder of its own under `staging`, keeping the file's
/// name (its extension decides the artifact's media type).
async fn stage(staging: &Path, box_path: &str, bytes: &[u8]) -> std::io::Result<Staged> {
    let name = Path::new(box_path)
        .file_name()
        .map_or_else(|| "file".into(), ToOwned::to_owned);
    let dir = staging.join(uuid::Uuid::new_v4().simple().to_string());
    tokio::fs::create_dir_all(&dir).await?;
    let file = dir.join(name);
    if let Err(error) = tokio::fs::write(&file, bytes).await {
        let _ = tokio::fs::remove_dir_all(&dir).await;
        return Err(error);
    }
    Ok(Staged { dir, file })
}

#[cfg(test)]
#[path = "box_source_tests.rs"]
mod tests;
