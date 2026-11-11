//! Media generation agent tools.
//!
//! `media_generate_image` and `media_generate_video` are TinyAgents'
//! [`GenerateImageTool`] / [`GenerateVideoTool`] bound to the managed
//! generators from [`super::provider`], under the names the `media` tool pack
//! and the image/video agents' allowlists already use. `media_list_models`
//! lists what the generators can run.

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use tinyagents_harness::media::{GenerateImageTool, GenerateVideoTool, MediaOutput};
use tinyagents_harness::tinyinference_image::ImageGenerator;
use tinyagents_harness::tinyinference_video::{VideoGenerator, WaitPolicy};
use tinyagents_harness_tools::media::MediaJobProgress;
use tinytools::{PermissionLevel, Tool, ToolCategory, ToolResult};

use super::artifact_tool::MediaArtifactTool;
use super::provider::MediaGenerators;
use crate::artifacts::ArtifactKind;
use crate::config::Config;

/// Image tool name (pinned by the `media` pack and agent allowlists).
pub const IMAGE_TOOL_NAME: &str = "media_generate_image";
/// Video tool name.
pub const VIDEO_TOOL_NAME: &str = "media_generate_video";
/// Model-listing tool name.
pub const LIST_MODELS_TOOL_NAME: &str = "media_list_models";

/// Poll cadence and budget for a video job.
const VIDEO_POLL_INTERVAL: Duration = Duration::from_secs(5);
const VIDEO_WAIT_BUDGET: Duration = Duration::from_secs(600);

pub type MediaReferenceAllowlist = Arc<Mutex<HashSet<PathBuf>>>;

const IMAGE_DESCRIPTION: &str = "Generate or edit images from a text prompt via OpenRouter \
     (default model: Seedream 5.0 Lite). Pass `references` (https URLs, workspace paths, or \
     paths returned by earlier media calls) \
     to edit, restyle, or keep a subject consistent. Saves each image in the Files folder and \
     returns its local file path. Billed per call: after an error that \
     says the call was billed, do not call again — report it to the user.";

const VIDEO_DESCRIPTION: &str = "Generate a short video clip via OpenRouter (default model: \
     Seedance 2.0 Mini, 4–15 s, 480p/720p, optional audio). Optionally start from \
     `first_frame` or end on `last_frame` (URL, workspace path, or path returned by an earlier \
     media call). Blocks until the clip is \
     ready (minutes) and saves it in the Files folder. Billed per call: if it times out, \
     call again with `resume_job_id` instead of submitting a new job.";

/// Builds the media tools over `generators` for a configuration: the
/// reference-file policy, the files roots and the video wait come from
/// `root_config`; `allowed_references` is the session's shared allowlist of
/// artifact paths. The generators come from the host that owns the media
/// service (the core has none of its own). TinySecurity's protected-path
/// floor is reached here, through `root_config`'s module, for the sync
/// reference policy.
pub async fn media_tools_for_config(
    generators: MediaGenerators,
    root_config: &Config,
    action_dir: &Path,
    allowed_references: MediaReferenceAllowlist,
) -> Vec<Box<dyn Tool>> {
    let floor = ReferenceFloor::fetch(Some(root_config)).await;
    media_tools_from_with_allowlist(
        generators,
        action_dir,
        &root_config.workspace_dir,
        crate::artifacts::FileRoots::from_config(root_config),
        // Each poll's job state feeds the call's progress heartbeat
        // (`super::progress`, wired per call by `MediaArtifactTool`), so a
        // multi-minute wait is not silent.
        WaitPolicy::new(VIDEO_POLL_INTERVAL, VIDEO_WAIT_BUDGET),
        allowed_references,
        floor,
    )
}

/// Builds the tool set over any generators (the managed ones in production,
/// mocks in tests), writing under `action_dir`; `video_wait` bounds each
/// video job. It has no async context to fetch TinySecurity's protected-path
/// floor, so it fails closed: local reference files are refused and URL
/// references still work.
pub fn media_tools_from(
    generators: MediaGenerators,
    action_dir: &Path,
    workspace_dir: &Path,
    files_dir: impl Into<crate::artifacts::FileRoots>,
    video_wait: WaitPolicy,
) -> Vec<Box<dyn Tool>> {
    media_tools_from_with_allowlist(
        generators,
        action_dir,
        workspace_dir,
        files_dir,
        video_wait,
        Arc::new(Mutex::new(HashSet::new())),
        ReferenceFloor::Unavailable("media_tools_from does not fetch TinySecurity's floor".into()),
    )
}

fn media_tools_from_with_allowlist(
    generators: MediaGenerators,
    action_dir: &Path,
    workspace_dir: &Path,
    files_dir: impl Into<crate::artifacts::FileRoots>,
    video_wait: WaitPolicy,
    allowed_artifacts: MediaReferenceAllowlist,
    floor: ReferenceFloor,
) -> Vec<Box<dyn Tool>> {
    let files = files_dir.into();
    let generators = Arc::new(generators);
    let Some(blobs) = crate::storage::sessions::artifact_blobs_for(workspace_dir) else {
        log::warn!("[media] scoped blob storage unavailable; media tools are not registered");
        return Vec::new();
    };
    let output = MediaOutput::new(action_dir, blobs).with_reference_policy(reference_policy(
        action_dir,
        workspace_dir,
        Arc::clone(&allowed_artifacts),
        floor,
    ));
    let output_blobs = Arc::clone(&output.blobs);
    let image_tool = |image: &Arc<dyn ImageGenerator>| {
        GenerateImageTool::new(Arc::clone(image), output.clone())
            .with_name(IMAGE_TOOL_NAME)
            .with_description(IMAGE_DESCRIPTION)
            .with_permission_level(PermissionLevel::Execute)
            .with_category(ToolCategory::Workflow)
    };
    let tools: Vec<Box<dyn Tool>> = vec![
        Box::new(
            MediaArtifactTool::new(
                image_tool(&generators.image),
                ArtifactKind::Image,
                workspace_dir.to_path_buf(),
                files.clone(),
            )
            .with_blobs(Arc::clone(&output_blobs))
            .with_reference_allowlist(Arc::clone(&allowed_artifacts))
            // A tool per call, around generators that disclose to that call's
            // chat.
            .with_per_call_inner({
                let generators = Arc::clone(&generators);
                let output = output.clone();
                move |_observer, context| {
                    let approval =
                        crate::agent::tinyagents::host::run_context::approval_for_tool(context);
                    let routed = generators.routed(approval.chat.as_ref());
                    GenerateImageTool::new(routed.image, output.clone())
                        .with_name(IMAGE_TOOL_NAME)
                        .with_description(IMAGE_DESCRIPTION)
                        .with_permission_level(PermissionLevel::Execute)
                        .with_category(ToolCategory::Workflow)
                }
            }),
        ),
        Box::new(
            MediaArtifactTool::new(
                video_tool(&generators.video, &output, video_wait.clone()),
                ArtifactKind::Video,
                workspace_dir.to_path_buf(),
                files,
            )
            .with_blobs(output_blobs)
            .with_reference_allowlist(Arc::clone(&allowed_artifacts))
            .with_per_call_inner({
                let generators = Arc::clone(&generators);
                let output = output.clone();
                move |progress, context| {
                    let approval =
                        crate::agent::tinyagents::host::run_context::approval_for_tool(context);
                    let routed = generators.routed(approval.chat.as_ref());
                    let observer = Arc::new(
                        move |status: &tinyagents_harness::tinyinference_video::VideoJobStatus| {
                            progress(&MediaJobProgress {
                                id: status.id.clone(),
                                state: status.state.to_string(),
                                outputs: status.outputs,
                                output_indices: status.output_indices.clone(),
                                cost_usd: status.cost_usd,
                                error: status.error.clone(),
                            });
                        },
                    );
                    video_tool(
                        &routed.video,
                        &output,
                        video_wait.clone().with_progress(observer),
                    )
                }
            }),
        ),
        Box::new(MediaListModelsTool {
            image: Arc::clone(&generators.image),
            video: Arc::clone(&generators.video),
        }),
    ];
    tracing::debug!("[media_generation] registered {} media tools", tools.len());
    tools
}

/// The video generation tool over `video`, with `wait` bounding each job.
fn video_tool(
    video: &Arc<dyn VideoGenerator>,
    output: &MediaOutput,
    wait: WaitPolicy,
) -> GenerateVideoTool {
    GenerateVideoTool::new(Arc::clone(video), output.clone())
        .with_name(VIDEO_TOOL_NAME)
        .with_description(VIDEO_DESCRIPTION)
        .with_permission_level(PermissionLevel::Execute)
        .with_category(ToolCategory::Workflow)
        .with_wait_policy(wait)
}

/// A synchronous TinySecurity floor decision for one path: `Ok(true)` when
/// `ProtectedPath2` reports it protected.
pub(crate) type ProtectedPathCheck =
    Arc<dyn Fn(&Path) -> Result<bool, String> + Send + Sync>;

/// TinySecurity's protected-path floor for local reference files. The
/// reference policy is a sync callback, so it asks the module through the
/// bounded sync bridge (`modules::security::text::protected_path_check`): the
/// decision is TinySecurity's own `ProtectedPath2`, not a host copy of it.
#[derive(Clone)]
pub(crate) enum ReferenceFloor {
    /// Security is disabled, so no floor applies; the same switch as
    /// `SecurityPolicy::is_always_forbidden_async` for the other file tools.
    Disabled,
    /// TinySecurity answers for each path. A failed answer refuses the path.
    Enforced(ProtectedPathCheck),
    /// The floor could not be reached: every local reference is refused.
    Unavailable(String),
}

impl std::fmt::Debug for ReferenceFloor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disabled => f.write_str("Disabled"),
            Self::Enforced(_) => f.write_str("Enforced(..)"),
            Self::Unavailable(why) => f.debug_tuple("Unavailable").field(why).finish(),
        }
    }
}

/// Why a reference was refused, for the log: never the path itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Refusal {
    Traversal,
    Unresolved,
    FloorUnavailable,
    Protected,
    OutsideRoots,
}

impl Refusal {
    fn as_str(self) -> &'static str {
        match self {
            Self::Traversal => "parent_traversal",
            Self::Unresolved => "unresolved",
            Self::FloorUnavailable => "floor_unavailable",
            Self::Protected => "protected_floor",
            Self::OutsideRoots => "outside_roots",
        }
    }
}

impl ReferenceFloor {
    /// Asks TinySecurity, through `config`'s module, for its floor unless
    /// security is disabled for `config` (or the process). With no `config`
    /// the current dispatch's is used. A module that cannot answer fails
    /// closed.
    pub(crate) async fn fetch(config: Option<&Config>) -> Self {
        if let Some(reason) = crate::security::switch::disabled_for(config) {
            log::debug!(
                "[media][reference] security disabled ({}); no protected floor",
                reason.describe()
            );
            return Self::Disabled;
        }
        #[cfg(feature = "security-module")]
        {
            match crate::modules::security::text::protected_path_check(config).await {
                Ok(check) => Self::Enforced(check),
                Err(error) => {
                    log::debug!(
                        "[media][reference] protected floor unavailable; local references refused: {error}"
                    );
                    Self::Unavailable(error)
                }
            }
        }
        #[cfg(not(feature = "security-module"))]
        {
            Self::Unavailable("TinySecurity is not built into this host".into())
        }
    }

    /// Why `paths` (the path as named and as resolved) may not be read as a
    /// reference, when the floor covers any of them or cannot answer.
    fn refusal(&self, named: &Path, paths: &[&Path]) -> Option<(Refusal, String)> {
        let unavailable = || {
            (
                Refusal::FloorUnavailable,
                format!(
                    "reference path {} refused: TinySecurity's protected-path floor is unavailable, \
                     so local reference files cannot be checked; use a URL instead",
                    named.display()
                ),
            )
        };
        match self {
            Self::Disabled => None,
            Self::Unavailable(_) => Some(unavailable()),
            Self::Enforced(check) => {
                for path in paths {
                    match check(path) {
                        Ok(false) => {}
                        Ok(true) => {
                            return Some((
                                Refusal::Protected,
                                format!(
                                    "reference path {} is in a protected location (a credential \
                                     store or system directory); use a URL or a file inside the \
                                     workspace",
                                    named.display()
                                ),
                            ))
                        }
                        Err(_) => return Some(unavailable()),
                    }
                }
                None
            }
        }
    }
}

/// `path` with its `.` segments and symlinks resolved. A path that does not
/// exist (yet) is only normalized lexically: nothing can be read from it, and
/// `..` is refused before this runs.
fn resolve(path: &Path) -> Result<PathBuf, std::io::Error> {
    match path.canonicalize() {
        Ok(resolved) => Ok(resolved),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(path
            .components()
            .filter(|component| !matches!(component, Component::CurDir))
            .collect()),
        Err(error) => Err(error),
    }
}

/// Local reference files may be read and uploaded only from the action
/// directory or the workspace directory, never through `..`, and never from
/// a location TinySecurity's protected floor covers (credential stores,
/// system roots) — session-filed artifacts included. The path is resolved
/// (`.` segments, symlinks) first; the floor is asked about the path as named
/// and as resolved, the roots are matched against the resolved path, and the
/// resolved path is what is returned for reading.
pub(crate) fn reference_policy(
    action_dir: &Path,
    workspace_dir: &Path,
    allowed_artifacts: MediaReferenceAllowlist,
    floor: ReferenceFloor,
) -> tinyagents_harness::media::ReferencePathPolicy {
    let mut roots: Vec<PathBuf> = Vec::new();
    for root in [action_dir, workspace_dir] {
        roots.push(root.to_path_buf());
        if let Ok(resolved) = root.canonicalize() {
            roots.push(resolved);
        }
    }
    Arc::new(move |path: &Path| {
        let refuse = |refusal: Refusal, message: String| {
            tracing::debug!(
                category = refusal.as_str(),
                "[media][reference] refused local reference"
            );
            Err(message)
        };
        if path.components().any(|c| matches!(c, Component::ParentDir)) {
            return refuse(
                Refusal::Traversal,
                format!("reference path {} may not contain '..'", path.display()),
            );
        }
        let resolved = match resolve(path) {
            Ok(resolved) => resolved,
            Err(error) => {
                return refuse(
                    Refusal::Unresolved,
                    format!(
                        "reference path {} could not be resolved: {error}",
                        path.display()
                    ),
                )
            }
        };
        if let Some((refusal, message)) = floor.refusal(path, &[path, &resolved]) {
            return refuse(refusal, message);
        }
        let is_filed_artifact = {
            let allowed = allowed_artifacts
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            allowed.contains(path) || allowed.contains(&resolved)
        };
        if !is_filed_artifact && !roots.iter().any(|root| resolved.starts_with(root)) {
            return refuse(
                Refusal::OutsideRoots,
                format!(
                    "reference path {} is outside the workspace; use a URL or a file inside the workspace",
                    path.display()
                ),
            );
        }
        Ok(resolved)
    })
}

/// Lists the image and video models the generators can run.
pub struct MediaListModelsTool {
    image: Arc<dyn ImageGenerator>,
    video: Arc<dyn VideoGenerator>,
}

#[async_trait]
impl Tool for MediaListModelsTool {
    fn name(&self) -> &str {
        LIST_MODELS_TOOL_NAME
    }

    fn description(&self) -> &str {
        "List the image and video generation models available to media_generate_image and \
         media_generate_video, with their ids. Use only when the user asks for a specific model \
         or style the default model cannot do."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "kind": { "type": "string", "enum": ["image", "video", "all"], "description": "Which catalog (default all)." },
                "search": { "type": "string", "description": "Case-insensitive substring filter on id or name." }
            }
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Workflow
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let kind = args.get("kind").and_then(Value::as_str).unwrap_or("all");
        let search = args
            .get("search")
            .and_then(Value::as_str)
            .map(str::to_ascii_lowercase);
        let keep = |id: &str, name: Option<&str>| {
            search.as_deref().is_none_or(|needle| {
                id.to_ascii_lowercase().contains(needle)
                    || name.is_some_and(|n| n.to_ascii_lowercase().contains(needle))
            })
        };
        let mut out = serde_json::Map::new();
        if kind != "video" {
            match self.image.list_models().await {
                Ok(models) => {
                    let list: Vec<Value> = models
                        .iter()
                        .filter(|m| keep(&m.id, m.name.as_deref()))
                        .map(|m| json!({ "id": m.id, "name": m.name }))
                        .collect();
                    out.insert(
                        "image".into(),
                        json!({ "default": self.image.default_model(), "models": list }),
                    );
                }
                Err(error) => {
                    return Ok(ToolResult::error(format!(
                        "Listing image models failed: {error}"
                    )));
                }
            }
        }
        if kind != "image" {
            match self.video.list_models().await {
                Ok(models) => {
                    let list: Vec<Value> = models
                        .iter()
                        .filter(|m| keep(&m.id, m.name.as_deref()))
                        .map(|m| json!({ "id": m.id, "name": m.name }))
                        .collect();
                    out.insert(
                        "video".into(),
                        json!({ "default": self.video.default_model(), "models": list }),
                    );
                }
                Err(error) => {
                    return Ok(ToolResult::error(format!(
                        "Listing video models failed: {error}"
                    )));
                }
            }
        }
        Ok(ToolResult::json(Value::Object(out)))
    }
}

#[cfg(test)]
#[path = "tools_tests.rs"]
mod tools_tests;
