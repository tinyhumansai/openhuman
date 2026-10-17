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
use tinytools::{PermissionLevel, Tool, ToolCategory, ToolResult};

use super::artifact_tool::MediaArtifactTool;
use super::provider::{managed_generators, MediaGenerators};
use crate::agent::artifacts::ArtifactKind;
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

pub(crate) type MediaReferenceAllowlist = Arc<Mutex<HashSet<PathBuf>>>;

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

/// Registers the media tools, or nothing when no backend is reachable.
pub fn build_media_tools(root_config: &Config, action_dir: &Path) -> Vec<Box<dyn Tool>> {
    build_media_tools_with_allowlist(
        root_config,
        action_dir,
        Arc::new(Mutex::new(HashSet::new())),
    )
}

pub(crate) fn build_media_tools_with_allowlist(
    root_config: &Config,
    action_dir: &Path,
    allowed_references: MediaReferenceAllowlist,
) -> Vec<Box<dyn Tool>> {
    let Some(generators) = managed_generators(root_config) else {
        return Vec::new();
    };
    media_tools_from_with_allowlist(
        generators,
        action_dir,
        &root_config.workspace_dir,
        crate::agent::artifacts::FileRoots::from_config(root_config),
        // Each poll's job state feeds the call's progress heartbeat
        // (`super::progress`, wired per call by `MediaArtifactTool`), so a
        // multi-minute wait is not silent.
        WaitPolicy::new(VIDEO_POLL_INTERVAL, VIDEO_WAIT_BUDGET),
        allowed_references,
    )
}

/// Builds the tool set over any generators (the managed ones in production,
/// mocks in tests), writing under `action_dir`; `video_wait` bounds each
/// video job.
pub fn media_tools_from(
    generators: MediaGenerators,
    action_dir: &Path,
    workspace_dir: &Path,
    files_dir: impl Into<crate::agent::artifacts::FileRoots>,
    video_wait: WaitPolicy,
) -> Vec<Box<dyn Tool>> {
    media_tools_from_with_allowlist(
        generators,
        action_dir,
        workspace_dir,
        files_dir,
        video_wait,
        Arc::new(Mutex::new(HashSet::new())),
    )
}

fn media_tools_from_with_allowlist(
    generators: MediaGenerators,
    action_dir: &Path,
    workspace_dir: &Path,
    files_dir: impl Into<crate::agent::artifacts::FileRoots>,
    video_wait: WaitPolicy,
    allowed_artifacts: MediaReferenceAllowlist,
) -> Vec<Box<dyn Tool>> {
    let files = files_dir.into();
    let generators = Arc::new(generators);
    let output = MediaOutput::new(action_dir).with_reference_policy(reference_policy(
        action_dir,
        workspace_dir,
        Arc::clone(&allowed_artifacts),
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
                move |_observer, chat| {
                    let routed = generators.routed(chat);
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
                move |observer, chat| {
                    let routed = generators.routed(chat);
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

/// Local reference files may be read and uploaded only from the action
/// directory or the workspace directory, never through `..`, and never from
/// an always-forbidden location (credential stores, system roots).
pub(crate) fn reference_policy(
    action_dir: &Path,
    workspace_dir: &Path,
    allowed_artifacts: MediaReferenceAllowlist,
) -> tinyagents_harness::media::ReferencePathPolicy {
    let roots: Vec<PathBuf> = vec![action_dir.to_path_buf(), workspace_dir.to_path_buf()];
    Arc::new(move |path: &Path| {
        if path.components().any(|c| matches!(c, Component::ParentDir)) {
            return Err(format!(
                "reference path {} may not contain '..'",
                path.display()
            ));
        }
        if crate::security::SecurityPolicy::is_always_forbidden(path) {
            return Err(format!(
                "reference path {} is in a protected location",
                path.display()
            ));
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
                    )))
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
                    )))
                }
            }
        }
        Ok(ToolResult::json(Value::Object(out)))
    }
}

#[cfg(test)]
#[path = "tools_tests.rs"]
mod tools_tests;
