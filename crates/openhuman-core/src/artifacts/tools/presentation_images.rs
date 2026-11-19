//! Slide-image resolution for `generate_presentation`.
//!
//! Turns the agent's per-slide image references (a prior artifact id or a
//! local file path) into validated, embeddable [`SlideImage`]s before the bus
//! spec is built. Kept apart from the tool so the host-policy reads
//! (`SecurityPolicy::validate_path`, artifact bytes, size caps) and the
//! `tinydocs` image inspection sit behind two small seams the tool's tests can
//! replace: [`ImageResolver`] (the whole step) and [`ImageInspector`] (format
//! and dimension facts for one image).
//!
//! A bad image is never fatal: it is skipped with a human-readable warning so
//! the deck still renders, minus that image.

use std::path::Path;
use std::time::Duration;

use async_trait::async_trait;
use tinydocs_bus::spec::SlideImage;

use crate::artifacts::{read_artifact_bytes, FileRoots};
use crate::config::Config;
use crate::core::runtime::DispatchCtx;
use crate::security::SecurityPolicy;

use super::presentation_types::{
    GeneratePresentationInput, SlideImageRequest, SlideImageSource, MAX_IMAGE_BYTES,
};

/// Format/dimension facts for one image's bytes.
#[async_trait]
pub(super) trait ImageInspector: Sync {
    async fn inspect(&self, bytes: &[u8]) -> Result<tinydocs_bus::ImageFacts, String>;
}

/// The whole image-resolution step, as the tool drives it.
#[async_trait]
pub(super) trait ImageResolver: Sync {
    async fn resolve(
        &self,
        sources: &ImageSources<'_>,
        input: &GeneratePresentationInput,
        ctx: &DispatchCtx,
    ) -> (Vec<Vec<SlideImage>>, Vec<String>);
}

/// Production resolver: [`ImageSources::resolve`] with the `tinydocs` module
/// as inspector.
pub(super) struct HostImageResolver;

#[async_trait]
impl ImageResolver for HostImageResolver {
    async fn resolve(
        &self,
        sources: &ImageSources<'_>,
        input: &GeneratePresentationInput,
        ctx: &DispatchCtx,
    ) -> (Vec<Vec<SlideImage>>, Vec<String>) {
        sources.resolve(input, ctx).await
    }
}

/// Inspects image bytes through the `tinydocs` module, bounded by `timeout`.
struct ModuleImageInspector<'a> {
    config: Option<&'a Config>,
    ctx: &'a DispatchCtx,
    timeout: Duration,
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
            self.timeout,
            crate::modules::documents::inspect_image(config, bytes),
        )
        .await
        .map_err(|_| "image inspection timed out".to_string())?
        .map_err(|error| error.to_string())
    }
}

/// Where slide images may be read from, and under which policy.
pub(super) struct ImageSources<'a> {
    /// Artifact metadata root, for `Artifact`-source images.
    pub(super) workspace_dir: &'a Path,
    /// Files folder the artifact bytes live in.
    pub(super) files_dir: &'a FileRoots,
    /// Policy an agent-supplied `File` path must pass before any read.
    pub(super) security: &'a SecurityPolicy,
    /// Host config snapshot for the module inspector; `None` loads it from
    /// the dispatch context.
    pub(super) config: Option<&'a Config>,
    /// Bound on one module image inspection.
    pub(super) inspection_timeout: Duration,
}

impl ImageSources<'_> {
    /// Resolve + validate every slide's images at the async boundary,
    /// returning a per-slide vec of embeddable images (aligned 1:1 with
    /// `input.slides`) plus a flat list of human-readable warnings for
    /// images that were skipped.
    ///
    /// A skip is never fatal: a deck with one bad image still renders,
    /// minus that image, and the agent is told why via the warning list.
    pub(super) async fn resolve(
        &self,
        input: &GeneratePresentationInput,
        ctx: &DispatchCtx,
    ) -> (Vec<Vec<SlideImage>>, Vec<String>) {
        self.resolve_with_inspector(
            input,
            &ModuleImageInspector {
                config: self.config,
                ctx,
                timeout: self.inspection_timeout,
            },
        )
        .await
    }

    pub(super) async fn resolve_with_inspector(
        &self,
        input: &GeneratePresentationInput,
        inspector: &impl ImageInspector,
    ) -> (Vec<Vec<SlideImage>>, Vec<String>) {
        let mut per_slide: Vec<Vec<SlideImage>> = Vec::with_capacity(input.slides.len());
        let mut warnings: Vec<String> = Vec::new();

        for (slide_idx, spec) in input.slides.iter().enumerate() {
            let mut resolved = Vec::with_capacity(spec.images.len());
            for (img_idx, image) in spec.images.iter().enumerate() {
                match self.resolve_one(image, inspector).await {
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
    async fn resolve_one(
        &self,
        image: &SlideImageRequest,
        inspector: &impl ImageInspector,
    ) -> Result<SlideImage, String> {
        let bytes = match &image.source {
            SlideImageSource::Artifact { artifact_id } => {
                read_artifact_bytes(self.workspace_dir, self.files_dir, artifact_id)
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

#[cfg(test)]
#[path = "presentation_images_tests.rs"]
mod tests;
