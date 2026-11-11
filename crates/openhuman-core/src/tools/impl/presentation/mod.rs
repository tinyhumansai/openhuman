//! Tool: `generate_presentation` — build a `.pptx` deck from a
//! structured slide spec via the native-Rust [`engine`] module.
//!
//! Flow:
//! 1. Validate source-agnostic request metadata through the bus contract before
//!    touching image references, resolve readable images under host policy,
//!    then validate the resolved bus spec before artifact production.
//! 2. Start a [`Production`]: it reserves `<title>.pptx` in the files folder
//!    (the card shows "generating…") and opens the versioned artifact — or,
//!    with an `artifact_id` arg or a regenerate in scope, a new version of
//!    that artifact under the same id and file.
//! 3. Generate the deck bytes via [`engine::generate`] — pure Rust,
//!    `ppt-rs`-backed, no Python runtime, no subprocess. Wrapped in
//!    `spawn_blocking` + `tokio::time::timeout` so the synchronous
//!    library work neither blocks the async executor nor can wedge
//!    the agent loop.
//! 4. Write the bytes and [`Production::complete`]: the file becomes the
//!    artifact's newest version and the card turns ready.
//! 5. On failure: [`Production::fail`] so the UI can surface the reason.
//!
//! Originally shipped in #2778 against a managed python-pptx venv;
//! refactored to a native-Rust engine in #2780-follow-up to drop the
//! Python runtime + first-call venv-install latency + 50 MB+ Python
//! disk footprint. Tool name / input schema / output schema / artifact
//! layout are byte-identical across the swap so #3017 ArtifactCard,
//! #3026 Files panel, and the orchestrator grounding rule in #3029
//! continue to work without change.

use crate::agent::tinyagents::host::run_context::dispatch_ctx_for_tool;
use crate::core::runtime::DispatchCtx;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::artifacts::production::Production;
use crate::artifacts::read_artifact_bytes;
use crate::security::SecurityPolicy;
use tinytools::{PermissionLevel, Tool, ToolResult};

mod engine;
mod types;

#[cfg(test)]
#[path = "presentation_tests.rs"]
mod tests;

use tinydocs_bus::spec::SlideImage;

use self::types::{
    to_bus_spec, validate_image_references, validate_request, GeneratePresentationInput,
    GeneratePresentationOutput, PresentationError, SlideImageRequest, SlideImageSource,
    MAX_IMAGE_BYTES,
};

/// Generation timeout. `ppt-rs` typically completes the full 64-slide
/// cap in well under a second; the 30 s ceiling is a defensive bound
/// against pathological inputs slipping past bus spec validation and
/// the worst-case `spawn_blocking` thread-acquisition latency on a
/// saturated runtime.
const GENERATION_TIMEOUT: Duration = Duration::from_secs(30);

/// Tool name surfaced to the agent. Stable; do not rename without
/// coordinating with the orchestrator agent definition list.
pub const TOOL_NAME: &str = "generate_presentation";

#[async_trait]
trait ImageInspector: Sync {
    async fn inspect(&self, bytes: &[u8]) -> Result<tinydocs_bus::ImageFacts, String>;
}

#[async_trait]
trait ImageResolver: Sync {
    async fn resolve(
        &self,
        tool: &PresentationTool,
        input: &GeneratePresentationInput,
        ctx: &DispatchCtx,
    ) -> (Vec<Vec<SlideImage>>, Vec<String>);
}

struct HostImageResolver;

#[async_trait]
impl ImageResolver for HostImageResolver {
    async fn resolve(
        &self,
        tool: &PresentationTool,
        input: &GeneratePresentationInput,
        ctx: &DispatchCtx,
    ) -> (Vec<Vec<SlideImage>>, Vec<String>) {
        tool.resolve_images(input, ctx).await
    }
}

struct ModuleImageInspector<'a> {
    config: Option<&'a crate::config::Config>,
    ctx: &'a DispatchCtx,
}

#[async_trait]
impl ImageInspector for ModuleImageInspector<'_> {
    async fn inspect(&self, bytes: &[u8]) -> Result<tinydocs_bus::ImageFacts, String> {
        let loaded;
        let config = match self.config {
            Some(config) => config,
            None => {
                loaded = self
                    .ctx
                    .load_config()
                    .await
                    .map_err(|error| format!("config unavailable: {error}"))?;
                &loaded
            }
        };
        crate::modules::documents::ensure_ready(config)
            .await
            .map_err(|error| error.to_string())?;
        tokio::time::timeout(
            GENERATION_TIMEOUT,
            crate::modules::documents::inspect_image(config, bytes),
        )
        .await
        .map_err(|_| "image inspection timed out".to_string())?
        .map_err(|error| error.to_string())
    }
}

/// One-shot `.pptx` generator. See module docs for the request flow.
pub struct PresentationTool {
    workspace_dir: PathBuf,
    /// Visible folder the generated deck is written into (#5505).
    files_dir: crate::artifacts::FileRoots,
    /// Existing host config when the caller already owns the authoritative
    /// runtime snapshot. Keeping this optional preserves the ordinary agent
    /// constructor while avoiding a process-global config reload during
    /// artifact regeneration.
    config: Option<crate::config::Config>,
    /// Security policy used to validate agent-supplied `File` image paths
    /// before any filesystem read — an image path must pass the same
    /// `validate_path` checks (allowed-location, symlink-escape, forbidden
    /// dirs) as any other file-read operation.
    security: Arc<SecurityPolicy>,
    /// The artifact id a Retry re-dispatches this generation under
    /// (`artifacts::ops::ai_regenerate`); `None` mints a fresh artifact.
    regenerate_target: Option<String>,
}

impl PresentationTool {
    /// Production constructor. The engine is stateless — no runtime
    /// resolution, venv setup, or cache directory needed. Pass the
    /// workspace directory the artifact pipeline writes into, plus the
    /// active [`SecurityPolicy`] for validating `File`-source image paths.
    pub fn new(
        workspace_dir: PathBuf,
        files_dir: impl Into<crate::artifacts::FileRoots>,
        security: Arc<SecurityPolicy>,
    ) -> Self {
        Self {
            workspace_dir,
            files_dir: files_dir.into(),
            config: None,
            security,
            regenerate_target: None,
        }
    }

    /// The agent-registry constructor: the artifact metadata goes to
    /// `config.workspace_dir`, the deck to the files folder (#5505).
    pub fn for_config(config: &crate::config::Config, security: Arc<SecurityPolicy>) -> Self {
        Self::with_config(
            config.workspace_dir.clone(),
            crate::artifacts::FileRoots::from_config(config),
            security,
            config.clone(),
        )
    }

    /// Construct the tool with an authoritative host config snapshot.
    pub(crate) fn with_config(
        workspace_dir: PathBuf,
        files_dir: impl Into<crate::artifacts::FileRoots>,
        security: Arc<SecurityPolicy>,
        config: crate::config::Config,
    ) -> Self {
        Self {
            workspace_dir,
            files_dir: files_dir.into(),
            config: Some(config),
            security,
            regenerate_target: None,
        }
    }

    /// Re-dispatch this generation under an existing artifact id, so the
    /// failed card is replaced in place (#3162).
    pub(crate) fn with_regenerate_target(mut self, artifact_id: impl Into<String>) -> Self {
        self.regenerate_target = Some(artifact_id.into());
        self
    }
}

#[async_trait]
impl Tool for PresentationTool {
    fn name(&self) -> &str {
        TOOL_NAME
    }

    fn description(&self) -> &str {
        // Router-rule format per the existing tool conventions (see
        // `current_time.rs` etc.): tell the orchestrator when to use
        // this tool and when NOT to.
        "Generate a PowerPoint (.pptx) presentation from a structured slide spec. \
         USE THIS when the user asks for slides, a deck, a presentation, or a \
         slide-by-slide breakdown of a topic. Provide `title` plus a `slides` \
         array of `{title, body?, bullets?, speaker_notes?}` objects. NOT for: \
         per-slide image generation, live editing of existing decks, or non-PPT \
         formats (PDF, Keynote, Google Slides exports). The generated file is \
         persisted as an artifact in the workspace and the tool returns the \
         artifact id + absolute path so the agent can reference it in the reply."
    }

    fn parameters_schema(&self) -> Value {
        // Built as separate `json!` bindings (rather than one deeply-nested
        // literal) to keep macro expansion within the crate's default
        // recursion limit — the per-slide `images` sub-schema adds enough
        // depth to overflow a single combined literal.
        let image_item_schema = json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["source"],
            "properties": {
                "source": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["type"],
                    "description": "Image data source. Use `{type:'artifact', artifact_id}` for a prior tool's output, or `{type:'file', path}` for a readable local file.",
                    "properties": {
                        "type": { "type": "string", "enum": ["artifact", "file"] },
                        "artifact_id": { "type": "string", "description": "Required when type='artifact'." },
                        "path": { "type": "string", "description": "Required when type='file'. Absolute or action-dir-relative path." }
                    }
                },
                "caption": {
                    "type": "string",
                    "maxLength": types::MAX_TEXT_CHARS,
                    "description": "Optional caption rendered as a text bullet beneath the image."
                }
            }
        });

        let images_schema = json!({
            "type": "array",
            "maxItems": types::MAX_IMAGES_PER_SLIDE,
            "description": "Images to embed beneath the slide text, single-column. PNG or JPEG only (≤5 MB each, ≤8 per deck). Each image's bytes come from a workspace artifact or a local file path the agent can read. Only attach an image whose content you have actually inspected — do not claim an image shows something you have not verified.",
            "items": image_item_schema,
        });

        let slide_item_schema = json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "title": { "type": "string", "maxLength": types::MAX_TEXT_CHARS },
                "body": { "type": "string", "maxLength": types::MAX_TEXT_CHARS },
                "bullets": {
                    "type": "array",
                    "maxItems": types::MAX_BULLETS_PER_SLIDE,
                    "items": { "type": "string", "maxLength": types::MAX_TEXT_CHARS }
                },
                "speaker_notes": { "type": "string", "maxLength": types::MAX_TEXT_CHARS },
                "images": images_schema,
            }
        });

        json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["title", "slides"],
            "properties": {
                "title": {
                    "type": "string",
                    "description": "Deck title. Surfaced on the title slide and used as the artifact's human-readable name. Required, non-empty.",
                    "maxLength": types::MAX_TEXT_CHARS,
                },
                "artifact_id": {
                    "type": "string",
                    "description": "Optional artifact id to update. Pass an id from a prior generate_presentation call to save this deck as a new version of that artifact instead of creating a new one.",
                },
                "author": {
                    "type": "string",
                    "description": "Optional author byline shown on the title slide.",
                    "maxLength": types::MAX_TEXT_CHARS,
                },
                "theme": {
                    "type": "string",
                    "description": "Reserved for future template-selection work. Currently informational only.",
                    "maxLength": types::MAX_TEXT_CHARS,
                },
                "slides": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": types::MAX_SLIDES,
                    "description": "Slide specs in display order. At least one entry required; hard cap to bound generation time + output size.",
                    "items": slide_item_schema,
                }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        // We write files to the workspace artifacts dir. Treat as
        // Write rather than ReadOnly. No subprocess / network reach.
        PermissionLevel::Write
    }

    fn supports_markdown(&self) -> bool {
        true
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        self.run(args, None, &dispatch_ctx_for_tool(None)).await
    }

    async fn execute_with_context(
        &self,
        args: Value,
        _options: tinytools::ToolCallOptions,
        context: Option<&dyn tinytools::ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        let approval = crate::agent::tinyagents::host::run_context::approval_for_tool(context);
        self.run(
            args,
            approval.chat.as_ref(),
            &dispatch_ctx_for_tool(context),
        )
        .await
    }
}

impl PresentationTool {
    /// Produces the deck for a turn whose chat routing is `chat` (`None`
    /// outside a chat turn).
    pub(crate) async fn run(
        &self,
        args: Value,
        chat: Option<&crate::security::approval::ApprovalChatContext>,
        ctx: &DispatchCtx,
    ) -> anyhow::Result<ToolResult> {
        self.run_with_resolver(args, chat, ctx, &HostImageResolver)
            .await
    }

    async fn run_with_resolver(
        &self,
        args: Value,
        chat: Option<&crate::security::approval::ApprovalChatContext>,
        ctx: &DispatchCtx,
        resolver: &impl ImageResolver,
    ) -> anyhow::Result<ToolResult> {
        let input: GeneratePresentationInput = match serde_json::from_value(args.clone()) {
            Ok(v) => v,
            Err(err) => {
                let msg = format!("invalid generate_presentation arguments: {err}");
                tracing::warn!(target: "presentation", err = %err, "[presentation] deserialisation failed");
                return Ok(ToolResult::error(msg));
            }
        };

        if let Err(err) = validate_request(&input).and_then(|()| validate_image_references(&input))
        {
            tracing::debug!(target: "presentation", err = %err, "[presentation] validation rejected input");
            return Ok(ToolResult::error(err.to_string()));
        }

        // Resolve + validate per-slide images at the async boundary. A
        // bad image is skipped with a warning (partial success) rather
        // than failing the whole deck — mirrors the #3076 ethos.
        let (resolved_images, image_warnings) = resolver.resolve(self, &input, ctx).await;
        if !image_warnings.is_empty() {
            tracing::info!(
                target: "presentation",
                warning_count = image_warnings.len(),
                "[presentation] some images skipped with warnings"
            );
        }

        let spec = match to_bus_spec(&input, resolved_images) {
            Ok(spec) => spec,
            Err(err) => return Ok(ToolResult::error(err.to_string())),
        };
        if let Err(err) = spec.validate().map_err(PresentationError::from) {
            tracing::debug!(target: "presentation", err = %err, "[presentation] validation rejected input");
            return Ok(ToolResult::error(err.to_string()));
        }

        tracing::info!(
            target: "presentation",
            title_chars = input.title.chars().count(),
            has_author = input.author.is_some(),
            slide_count = input.slides.len(),
            "[presentation] generation request accepted"
        );

        // Reserve the file and open the artifact: a new one, or a new version
        // of `artifact_id` / the regenerate target (same id, same file,
        // history kept).
        let production = Production::start(
            &self.workspace_dir,
            self.files_dir.clone(),
            tiny_artifacts::ArtifactKind::File,
            &input.title,
            "pptx",
            None,
            input
                .artifact_id
                .as_deref()
                .or(self.regenerate_target.as_deref()),
            chat,
        )
        .await
        .map_err(anyhow::Error::msg)?;
        let output_path = production.path().to_path_buf();

        // Persist the verbatim args next to meta.json so a failed card's
        // Retry can re-dispatch this exact spec deterministically (#3162).
        // Best-effort: a write failure only forfeits future regeneration,
        // it must not abort an otherwise-successful generation.
        if let Err(err) =
            crate::artifacts::store::save_artifact_args(&self.workspace_dir, production.id(), &args)
                .await
        {
            tracing::warn!(
                target: "presentation",
                err = %err,
                artifact_id = %production.id(),
                "[presentation] failed to persist args.json; artifact will not be regenerable"
            );
        }

        let bytes = match engine::generate(&spec, GENERATION_TIMEOUT, self.config.as_ref()).await {
            Ok(bytes) => bytes,
            Err(err) => {
                production.fail(&err.to_string()).await;
                tracing::warn!(
                    target: "presentation",
                    err = %err,
                    "[presentation] engine generation failed"
                );
                return Ok(ToolResult::error(err.to_string()));
            }
        };

        if let Err(err) = tokio::fs::write(&output_path, &bytes).await {
            let filename = output_path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let reason = format!("failed to write generated deck ({filename}): {err}");
            production.fail(&reason).await;
            tracing::warn!(
                target: "presentation",
                err = %err,
                artifact_id = %production.id(),
                filename = %filename,
                "[presentation] artifact file write failed"
            );
            return Ok(ToolResult::error(reason));
        }

        // Record the file as the artifact's newest version; the card turns
        // ready from that record.
        let size_bytes = match production.complete().await {
            Ok(size) => size,
            Err(reason) => {
                tracing::warn!(
                    target: "presentation",
                    artifact_id = %production.id(),
                    "[presentation] {reason}"
                );
                return Ok(ToolResult::error(reason));
            }
        };

        tracing::info!(
            target: "presentation",
            artifact_id = %production.id(),
            size_bytes,
            slide_count = input.slides.len(),
            "[presentation] generation complete"
        );

        let out = GeneratePresentationOutput {
            artifact_id: production.id().to_string(),
            artifact_path: output_path.display().to_string(),
            slide_count: input.slides.len(),
            size_bytes,
            image_warnings,
        };
        let payload = serde_json::to_value(&out)?;
        let mut markdown = format!(
            "Generated {}-slide presentation at `{}` (artifact `{}`, {} bytes).",
            out.slide_count, out.artifact_path, out.artifact_id, out.size_bytes
        );
        if !out.image_warnings.is_empty() {
            markdown.push_str("\n\n⚠️ Some images were skipped:");
            for warning in &out.image_warnings {
                markdown.push_str(&format!("\n- {warning}"));
            }
        }
        Ok(ToolResult::success_with_markdown(payload, markdown))
    }
}

impl PresentationTool {
    /// Resolve + validate every slide's images at the async boundary,
    /// returning a per-slide vec of embeddable images (aligned 1:1 with
    /// `input.slides`) plus a flat list of human-readable warnings for
    /// images that were skipped.
    ///
    /// A skip is never fatal: a deck with one bad image still renders,
    /// minus that image, and the agent is told why via the warning list.
    async fn resolve_images(
        &self,
        input: &GeneratePresentationInput,
        ctx: &DispatchCtx,
    ) -> (Vec<Vec<SlideImage>>, Vec<String>) {
        self.resolve_images_with_inspector(
            input,
            &ModuleImageInspector {
                config: self.config.as_ref(),
                ctx,
            },
        )
        .await
    }

    async fn resolve_images_with_inspector(
        &self,
        input: &GeneratePresentationInput,
        inspector: &impl ImageInspector,
    ) -> (Vec<Vec<SlideImage>>, Vec<String>) {
        let mut per_slide: Vec<Vec<SlideImage>> = Vec::with_capacity(input.slides.len());
        let mut warnings: Vec<String> = Vec::new();

        for (slide_idx, spec) in input.slides.iter().enumerate() {
            let mut resolved = Vec::with_capacity(spec.images.len());
            for (img_idx, image) in spec.images.iter().enumerate() {
                match self.resolve_one_image(image, inspector).await {
                    Ok(r) => resolved.push(r),
                    Err(reason) => {
                        // 1-based indices in the message (matches how the
                        // agent thinks about "slide 2, image 1").
                        warnings.push(format!(
                            "slide {} image {}: {reason}",
                            slide_idx + 1,
                            img_idx + 1
                        ));
                    }
                }
            }
            per_slide.push(resolved);
        }

        (per_slide, warnings)
    }

    /// Resolve a single [`SlideImageRequest`] to validated, embeddable bytes.
    /// Returns `Err(reason)` (a short human-readable string) when the
    /// image cannot be embedded — the caller turns that into a skip
    /// warning.
    async fn resolve_one_image(
        &self,
        image: &SlideImageRequest,
        inspector: &impl ImageInspector,
    ) -> Result<SlideImage, String> {
        let bytes = match &image.source {
            SlideImageSource::Artifact { artifact_id } => {
                read_artifact_bytes(&self.workspace_dir, &self.files_dir, artifact_id)
                    .await
                    .map_err(|e| format!("artifact {artifact_id} unreadable: {e}"))?
            }
            SlideImageSource::File { path } => {
                // Validate the agent-supplied path against the security
                // policy BEFORE touching the filesystem. This enforces the
                // same allowed-location / symlink-escape / forbidden-dir
                // checks as every other file-read tool — without it an
                // agent could embed `/etc/shadow` or `~/.ssh/id_rsa`.
                // `validate_path` returns the canonical, in-policy path.
                let resolved = self
                    .security
                    .validate_path(path)
                    .await
                    .map_err(|e| format!("file {path} not allowed: {e}"))?;
                // Stat first so a pathologically large file is rejected
                // before we pull it into memory.
                let meta = tokio::fs::metadata(&resolved)
                    .await
                    .map_err(|e| format!("file {path} unreadable: {e}"))?;
                if meta.len() as usize > MAX_IMAGE_BYTES {
                    return Err(format!(
                        "file {path} is {} bytes, exceeds {MAX_IMAGE_BYTES}-byte cap",
                        meta.len()
                    ));
                }
                tokio::fs::read(&resolved)
                    .await
                    .map_err(|e| format!("file {path} unreadable: {e}"))?
            }
        };

        if bytes.len() > MAX_IMAGE_BYTES {
            return Err(format!(
                "image is {} bytes, exceeds {MAX_IMAGE_BYTES}-byte cap",
                bytes.len()
            ));
        }

        let facts = inspector.inspect(&bytes).await?;

        Ok(SlideImage {
            bytes,
            format: facts.format,
            width_px: facts.width_px,
            height_px: facts.height_px,
            caption: image.caption.clone(),
        })
    }
}
