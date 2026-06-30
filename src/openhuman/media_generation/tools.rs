//! Agent-facing media-generation tools (image + video) backed by GMI via the
//! OpenHuman backend's `media_generation` provider.
//!
//! **Endpoints** (see `backend/docs/media-generation.md`):
//!   - `POST /agent-integrations/media-generation/images`
//!   - `POST /agent-integrations/media-generation/videos`
//!   - `GET  /agent-integrations/media-generation/requests/{requestId}`
//!   - `GET  /agent-integrations/media-generation/models`
//!
//! Generation is asynchronous. These tools **block with progress**: they submit
//! (`wait:false`, so the backend charges + returns a request id immediately),
//! then poll the request until it reaches a terminal state, download each
//! resulting artifact into the agent's `generated-media/` root, and return the
//! local file paths. The backend owns GMI keys, billing, and rate limiting.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::openhuman::config::Config;
use crate::openhuman::integrations::IntegrationClient;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolCategory, ToolResult};

use super::download::persist_media;
use super::types::MediaResponse;

const IMAGES_PATH: &str = "/agent-integrations/media-generation/images";
const VIDEOS_PATH: &str = "/agent-integrations/media-generation/videos";
const MODELS_PATH: &str = "/agent-integrations/media-generation/models";

/// Poll cadence + caps. Images are fast; video can take minutes.
const POLL_INTERVAL: Duration = Duration::from_secs(4);
const IMAGE_MAX_WAIT_SECS: u64 = 180;
const VIDEO_MAX_WAIT_SECS: u64 = 420;

/// Shared submit-then-poll-then-persist flow for both modalities.
async fn generate_and_persist(
    client: &IntegrationClient,
    action_dir: &std::path::Path,
    submit_path: &str,
    body: Value,
    max_wait_secs: u64,
) -> ToolResult {
    // Submit without server-side blocking; the backend charges on submit and
    // returns a request id we poll ourselves (so the core owns the progress UX).
    let submitted: MediaResponse = match client.post::<MediaResponse>(submit_path, &body).await {
        Ok(resp) => resp,
        Err(e) => return ToolResult::error(format!("Media generation submit failed: {e}")),
    };

    let request_id = submitted.request_id.clone();
    tracing::info!(
        "[media_generation] submitted request={} status={} cost=${:.4}",
        request_id,
        submitted.status,
        submitted.cost_usd
    );

    let status_path = format!(
        "/agent-integrations/media-generation/requests/{}",
        request_id
    );

    let mut latest = submitted;
    let deadline = Instant::now() + Duration::from_secs(max_wait_secs);
    while !latest.is_terminal() {
        if Instant::now() >= deadline {
            tracing::warn!(
                "[media_generation] wait budget elapsed for request={} (status={})",
                request_id,
                latest.status
            );
            return ToolResult::success(format!(
                "Media generation is still {} after {}s. It was accepted (request_id: {}) and \
                 billed; it may finish shortly — check again later.",
                latest.status, max_wait_secs, request_id
            ));
        }
        tokio::time::sleep(POLL_INTERVAL).await;
        match client.get::<MediaResponse>(&status_path).await {
            Ok(resp) => {
                tracing::debug!(
                    "[media_generation] poll request={} status={}",
                    request_id,
                    resp.status
                );
                latest = resp;
            }
            Err(e) => {
                tracing::warn!(
                    "[media_generation] poll error for request={}: {e}",
                    request_id
                );
                // Transient poll failures shouldn't abort a paid generation —
                // keep polling until the deadline.
            }
        }
    }

    if latest.is_failed() {
        return ToolResult::error(format!(
            "Media generation failed (request_id: {request_id})."
        ));
    }

    if latest.media.is_empty() {
        return ToolResult::error(format!(
            "Media generation reported success but returned no media (request_id: {request_id})."
        ));
    }

    match persist_media(action_dir, &request_id, &latest.media).await {
        Ok(artifacts) => {
            let mut lines = vec![format!(
                "Generated {} artifact(s) (request_id: {}, model: {}):",
                artifacts.len(),
                request_id,
                latest.model
            )];
            for art in &artifacts {
                lines.push(format!("- {} → {}", art.kind, art.path.display()));
                if let Some(thumb) = &art.thumbnail_url {
                    lines.push(format!("    thumbnail: {thumb}"));
                }
            }
            lines.push(format!("\nCost: ${:.4}", latest.cost_usd));
            let payload = json!({
                "request_id": request_id,
                "model": latest.model,
                "cost_usd": latest.cost_usd,
                "artifacts": artifacts.iter().map(|a| json!({
                    "type": a.kind,
                    "path": a.path.display().to_string(),
                    "source_url": a.source_url,
                    "thumbnail_url": a.thumbnail_url,
                })).collect::<Vec<_>>(),
            });
            ToolResult::success_with_markdown(payload, lines.join("\n"))
        }
        Err(e) => ToolResult::error(format!(
            "Generation succeeded but persisting media failed (request_id: {request_id}): {e}"
        )),
    }
}

// ── MediaGenerateImageTool ──────────────────────────────────────────

pub struct MediaGenerateImageTool {
    client: Arc<IntegrationClient>,
    action_dir: PathBuf,
}

impl MediaGenerateImageTool {
    pub fn new(client: Arc<IntegrationClient>, action_dir: PathBuf) -> Self {
        Self { client, action_dir }
    }
}

#[async_trait]
impl Tool for MediaGenerateImageTool {
    fn name(&self) -> &str {
        "media_generate_image"
    }

    fn description(&self) -> &str {
        "Generate or edit an image from a text prompt using GMI (Seedream / SeedEdit). \
         Optionally pass reference image URLs to edit/condition (image-to-image). \
         Blocks until the image is ready and saves it under the workspace \
         generated-media folder, returning the local file path. Cost is billed by the backend."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "prompt": { "type": "string", "description": "Detailed visual prompt or edit instruction" },
                "model": { "type": "string", "description": "Optional GMI model id (default: seedream-4-0-250828). Use media_list_models to discover." },
                "size": { "type": "string", "description": "Optional output size, e.g. 1024x1024 or 1536x1024" },
                "n": { "type": "integer", "minimum": 1, "maximum": 8, "description": "Number of images (default 1)" },
                "input_images": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional reference image URLs for edit / image-to-image"
                },
                "seed": { "type": "integer", "description": "Optional seed for reproducibility" }
            },
            "required": ["prompt"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Workflow
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let prompt = match args.get("prompt").and_then(|v| v.as_str()) {
            Some(p) if !p.trim().is_empty() => p,
            _ => return Ok(ToolResult::error("prompt is required")),
        };

        let mut body = json!({ "prompt": prompt, "wait": false });
        if let Some(model) = args.get("model").and_then(|v| v.as_str()) {
            body["model"] = json!(model);
        }
        if let Some(size) = args.get("size").and_then(|v| v.as_str()) {
            body["size"] = json!(size);
        }
        if let Some(n) = args.get("n").and_then(|v| v.as_u64()) {
            body["n"] = json!(n.clamp(1, 8));
        }
        if let Some(imgs) = args.get("input_images").and_then(|v| v.as_array()) {
            let urls: Vec<&str> = imgs.iter().filter_map(|v| v.as_str()).collect();
            if !urls.is_empty() {
                body["inputImages"] = json!(urls);
            }
        }
        if let Some(seed) = args.get("seed").and_then(|v| v.as_i64()) {
            body["seed"] = json!(seed);
        }

        tracing::info!("[media_generate_image] prompt_len={}", prompt.len());
        Ok(generate_and_persist(
            &self.client,
            &self.action_dir,
            IMAGES_PATH,
            body,
            IMAGE_MAX_WAIT_SECS,
        )
        .await)
    }
}

// ── MediaGenerateVideoTool ──────────────────────────────────────────

pub struct MediaGenerateVideoTool {
    client: Arc<IntegrationClient>,
    action_dir: PathBuf,
}

impl MediaGenerateVideoTool {
    pub fn new(client: Arc<IntegrationClient>, action_dir: PathBuf) -> Self {
        Self { client, action_dir }
    }
}

#[async_trait]
impl Tool for MediaGenerateVideoTool {
    fn name(&self) -> &str {
        "media_generate_video"
    }

    fn description(&self) -> &str {
        "Generate a short video from a text prompt using GMI (Seedance / Veo). \
         Optionally pass a first-frame/reference image URL for image-to-video. \
         Video can take a few minutes; this blocks until it is ready, saves the \
         clip under the workspace generated-media folder, and returns the local \
         file path. Cost is billed by the backend."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "prompt": { "type": "string", "description": "Detailed description of the video to generate" },
                "model": { "type": "string", "description": "Optional GMI model id (default: seedance-1-0-pro-fast-251015). Use media_list_models to discover." },
                "input_image": { "type": "string", "description": "Optional first-frame / reference image URL for image-to-video" },
                "duration_seconds": { "type": "integer", "minimum": 1, "maximum": 60, "description": "Optional clip duration in seconds" },
                "aspect_ratio": { "type": "string", "description": "Optional aspect ratio, e.g. 16:9, 9:16, 1:1" },
                "negative_prompt": { "type": "string", "description": "Optional description of what to avoid" },
                "seed": { "type": "integer", "description": "Optional seed for reproducibility" }
            },
            "required": ["prompt"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Workflow
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let prompt = match args.get("prompt").and_then(|v| v.as_str()) {
            Some(p) if !p.trim().is_empty() => p,
            _ => return Ok(ToolResult::error("prompt is required")),
        };

        let mut body = json!({ "prompt": prompt, "wait": false });
        if let Some(model) = args.get("model").and_then(|v| v.as_str()) {
            body["model"] = json!(model);
        }
        if let Some(img) = args.get("input_image").and_then(|v| v.as_str()) {
            body["inputImage"] = json!(img);
        }
        if let Some(d) = args.get("duration_seconds").and_then(|v| v.as_u64()) {
            body["durationSeconds"] = json!(d.clamp(1, 60));
        }
        if let Some(ar) = args.get("aspect_ratio").and_then(|v| v.as_str()) {
            body["aspectRatio"] = json!(ar);
        }
        if let Some(np) = args.get("negative_prompt").and_then(|v| v.as_str()) {
            body["negativePrompt"] = json!(np);
        }
        if let Some(seed) = args.get("seed").and_then(|v| v.as_i64()) {
            body["seed"] = json!(seed);
        }

        tracing::info!("[media_generate_video] prompt_len={}", prompt.len());
        Ok(generate_and_persist(
            &self.client,
            &self.action_dir,
            VIDEOS_PATH,
            body,
            VIDEO_MAX_WAIT_SECS,
        )
        .await)
    }
}

// ── MediaListModelsTool ─────────────────────────────────────────────

pub struct MediaListModelsTool {
    client: Arc<IntegrationClient>,
}

impl MediaListModelsTool {
    pub fn new(client: Arc<IntegrationClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for MediaListModelsTool {
    fn name(&self) -> &str {
        "media_list_models"
    }

    fn description(&self) -> &str {
        "List available image/video generation models — a curated catalog with \
         pricing, plus (with include_upstream) GMI's full live model list. Use to \
         pick a `model` id for media_generate_image / media_generate_video."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "include_upstream": {
                    "type": "boolean",
                    "description": "Also fetch GMI's full live model list (default false)"
                }
            }
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Workflow
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let include_upstream = args
            .get("include_upstream")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let path = if include_upstream {
            format!("{MODELS_PATH}?includeUpstream=true")
        } else {
            MODELS_PATH.to_string()
        };
        match self.client.get::<Value>(&path).await {
            Ok(resp) => Ok(ToolResult::success_with_markdown(
                resp.clone(),
                serde_json::to_string_pretty(&resp).unwrap_or_else(|_| resp.to_string()),
            )),
            Err(e) => Ok(ToolResult::error(format!(
                "Failed to list media models: {e}"
            ))),
        }
    }
}

// ── Builder ─────────────────────────────────────────────────────────

/// Build the media-generation tool surface. Returns empty when no integration
/// client is configured (no backend URL / not signed in), mirroring the other
/// backend-proxied tool families.
pub fn build_media_tools(root_config: &Config, action_dir: &std::path::Path) -> Vec<Box<dyn Tool>> {
    let Some(client) = crate::openhuman::integrations::build_client(root_config) else {
        tracing::debug!("[media_generation] no integration client — media tools skipped");
        return Vec::new();
    };

    let action_dir = action_dir.to_path_buf();
    let tools: Vec<Box<dyn Tool>> = vec![
        Box::new(MediaGenerateImageTool::new(
            Arc::clone(&client),
            action_dir.clone(),
        )),
        Box::new(MediaGenerateVideoTool::new(Arc::clone(&client), action_dir)),
        Box::new(MediaListModelsTool::new(Arc::clone(&client))),
    ];
    tracing::debug!("[media_generation] registered {} media tools", tools.len());
    tools
}

#[cfg(test)]
#[path = "tools_tests.rs"]
mod tools_tests;
