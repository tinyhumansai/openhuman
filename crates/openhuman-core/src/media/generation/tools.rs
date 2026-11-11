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
/// floor is fetched here, once, for the sync reference policy.
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

/// TinySecurity's protected-path floor for local reference files. The
/// reference policy is a sync callback that cannot ask the module per path, so
/// the floor vocabulary is fetched once, when the tool set is built, and
/// matched lexically ([`floor_covers`]).
#[derive(Debug, Clone)]
pub(crate) enum ReferenceFloor {
    /// Security is disabled, so no floor applies; the same switch as
    /// `SecurityPolicy::is_always_forbidden_async` for the other file tools.
    Disabled,
    /// TinySecurity's floor vocabulary, lower-cased for matching.
    Enforced(tinysecurity_bus::ProtectedFloor2),
    /// The floor could not be fetched: every local reference is refused.
    Unavailable(String),
}

impl ReferenceFloor {
    /// Asks TinySecurity for its floor vocabulary unless security is disabled
    /// for `config` (or the process). A failed query fails closed.
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
            match crate::modules::security::gap::protected_floor().await {
                Ok(floor) => Self::enforced(floor),
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

    /// The floor over `vocabulary`, lower-cased as TinySecurity matches it.
    pub(crate) fn enforced(vocabulary: tinysecurity_bus::ProtectedFloor2) -> Self {
        let lower = |names: Vec<String>| -> Vec<String> {
            names.iter().map(|name| name.to_ascii_lowercase()).collect()
        };
        Self::Enforced(tinysecurity_bus::ProtectedFloor2 {
            components: lower(vocabulary.components),
            files: lower(vocabulary.files),
            pairs: vocabulary
                .pairs
                .iter()
                .map(|[parent, child]| [parent.to_ascii_lowercase(), child.to_ascii_lowercase()])
                .collect(),
            prefixes: lower(vocabulary.prefixes),
        })
    }

    /// Why `path` may not be read as a reference, when the floor covers it.
    fn refusal(&self, path: &Path) -> Option<String> {
        match self {
            Self::Disabled => None,
            Self::Unavailable(_) => Some(format!(
                "reference path {} refused: TinySecurity's protected-path floor is unavailable, \
                 so local reference files cannot be checked; use a URL instead",
                path.display()
            )),
            Self::Enforced(floor) => floor_covers(floor, &path.to_string_lossy()).then(|| {
                format!(
                    "reference path {} is in a protected location (a credential store or \
                     system directory); use a URL or a file inside the workspace",
                    path.display()
                )
            }),
        }
    }
}

/// Whether TinySecurity's floor covers `path`: a lexical mirror of
/// `tinysecurity-policy`'s `path::floor` over the served vocabulary. Matching
/// is ASCII case-insensitive and treats `\` as `/`; a segment is compared up
/// to any `:` stream suffix and without trailing spaces or dots. Credential
/// names match at any depth, pairs as consecutive segments, and system
/// prefixes at the root, Windows ones under any drive letter.
fn floor_covers(floor: &tinysecurity_bus::ProtectedFloor2, path: &str) -> bool {
    let lowered = path.to_ascii_lowercase().replace('\\', "/");
    let normalized = ["//?/", "//./", "/??/"]
        .iter()
        .find_map(|namespace| lowered.strip_prefix(namespace))
        .unwrap_or(&lowered);
    let segments: Vec<&str> = normalized.split('/').filter(|s| !s.is_empty()).collect();
    let parent = |segment: &&str| {
        segment.starts_with("..") && segment.trim_end_matches([' ', '.']).is_empty()
    };
    if path.contains('\0') || segments.iter().any(parent) {
        return true;
    }
    let names: Vec<&str> = segments
        .iter()
        .map(|segment| {
            segment
                .split(':')
                .next()
                .unwrap_or(segment)
                .trim_end_matches([' ', '.'])
        })
        .collect();
    let credential_name = names.iter().any(|name| {
        floor
            .components
            .iter()
            .chain(&floor.files)
            .any(|store| store == name)
    });
    let credential_pair = names.windows(2).any(|window| {
        floor
            .pairs
            .iter()
            .any(|[parent, child]| window[0] == parent && window[1] == child)
    });
    credential_name
        || credential_pair
        || floor
            .prefixes
            .iter()
            .any(|prefix| under_prefix(normalized, prefix))
}

/// `path` is `prefix` or below it. A drive-qualified prefix (`c:/windows`)
/// matches under every drive letter, as TinySecurity's Windows roots do.
fn under_prefix(path: &str, prefix: &str) -> bool {
    let within = |path: &str, prefix: &str| {
        path.strip_prefix(prefix)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
    };
    let drive_relative = |path: &str| {
        let bytes = path.as_bytes();
        (bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':')
            .then(|| path[2..].to_owned())
    };
    within(path, prefix)
        || matches!(
            (drive_relative(path), drive_relative(prefix)),
            (Some(path), Some(prefix)) if within(&path, &prefix)
        )
}

/// Local reference files may be read and uploaded only from the action
/// directory or the workspace directory, never through `..`, and never from
/// a location TinySecurity's protected floor covers (credential stores,
/// system roots) — session-filed artifacts included.
pub(crate) fn reference_policy(
    action_dir: &Path,
    workspace_dir: &Path,
    allowed_artifacts: MediaReferenceAllowlist,
    floor: ReferenceFloor,
) -> tinyagents_harness::media::ReferencePathPolicy {
    let roots: Vec<PathBuf> = vec![action_dir.to_path_buf(), workspace_dir.to_path_buf()];
    Arc::new(move |path: &Path| {
        if path.components().any(|c| matches!(c, Component::ParentDir)) {
            return Err(format!(
                "reference path {} may not contain '..'",
                path.display()
            ));
        }
        if let Some(refusal) = floor.refusal(path) {
            log::debug!("[media][reference] refused local reference: {refusal}");
            return Err(refusal);
        }
        let is_filed_artifact = allowed_artifacts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains(path);
        if !is_filed_artifact && !roots.iter().any(|root| path.starts_with(root)) {
            return Err(format!(
                "reference path {} is outside the workspace; use a URL or a file inside the workspace",
                path.display()
            ));
        }
        Ok(path.to_path_buf())
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
