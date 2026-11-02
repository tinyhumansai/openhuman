//! Ephemeral provider request preparation for durable workspace attachments.
use super::AttachmentAccessScope;
use crate::config::Config;
use crate::inference::host_controls::HostControlSlot;
use base64::{engine::general_purpose::STANDARD, Engine};
use std::sync::Arc;
use tinyinference_llm::{
    message::{ContentBlock, ImageRef, MediaRef, Message},
    model::{ChatModel, InputModality, InputSource, ModelProfile, ModelRequest},
};

#[cfg(test)]
use async_trait::async_trait;
#[cfg(test)]
use tinyinference_llm::model::{ModelResponse, ModelStream};

pub(crate) async fn wrap(
    ctx: &crate::core::runtime::DispatchCtx,
    inner: Arc<dyn ChatModel<()>>,
    config: Arc<Config>,
    model: &str,
    provider: &str,
) -> Arc<dyn ChatModel<()>> {
    wrap_controlled(ctx, inner, config, model, provider, None)
}

/// [`wrap`] with the turn's spend controls placed beneath attachment rendering.
pub(crate) fn wrap_controlled(
    ctx: &crate::core::runtime::DispatchCtx,
    inner: Arc<dyn ChatModel<()>>,
    config: Arc<Config>,
    model: &str,
    provider: &str,
    controls: Option<&HostControlSlot>,
) -> Arc<dyn ChatModel<()>> {
    let mut profile = inner.profile().cloned().unwrap_or_default();
    // Transport profiles describe serialization support, not selected-model facts.
    let catalog = tinyagents_registry::catalog::ModelCatalog::seed().ok();
    profile.modalities = catalog
        .as_ref()
        .and_then(|c| {
            c.profile(provider, model).or_else(|| {
                c.get_by_model_id(model)
                    .and_then(|e| c.profile(&e.provider, &e.model_id))
            })
        })
        .map(|p| p.modalities)
        .unwrap_or_default();
    if provider == "openhuman" || provider == "managed" {
        let resolved_context = ctx.resolved();
        profile.modalities.image_in =
            crate::inference::registry::provider_model_supports_vision_for(
                resolved_context
                    .as_deref()
                    .map(crate::core::runtime::CoreContext::extensions),
                crate::inference::provider::factory::PROVIDER_OPENHUMAN,
                model,
            );
    }
    let route = if matches!(provider, "openhuman" | "managed") {
        "openhuman".to_string()
    } else {
        format!("{provider}:{model}")
    };
    if let Some(request) =
        crate::inference::context_window::model_limits_request("chat", &route, model, &config).await
    {
        if let Some(inputs) = tinyinference_llm::model::discover::model_limits_cache()
            .get_variant(&request.endpoint, &request.model, &request.cache_variant())
            .effective()
            .and_then(|l| l.input_modalities)
        {
            profile.modalities.image_in = inputs.iter().any(|m| m == "image");
            profile.modalities.audio_in = inputs.iter().any(|m| m == "audio");
            profile.modalities.video_in = inputs.iter().any(|m| m == "video");
            profile.modalities.document_in = inputs
                .iter()
                .any(|m| matches!(m.as_str(), "document" | "pdf" | "file"));
        }
    }
    if tinyinference_local::profile::is_local_provider_string(provider) {
        profile.modalities.image_in |= tinyinference_llm::model::model_id_supports_vision(model);
    }
    profile.modalities.image_in |=
        crate::inference::model_context::model_vision_enabled(model, &config);
    owner::wrap(ctx, inner, config, Some(profile), controls)
}

/// Injected models supply their own selected-model facts through ModelProfile.
/// Transport support is still required for every native media block.
pub(crate) fn wrap_injected(
    ctx: &crate::core::runtime::DispatchCtx,
    inner: Arc<dyn ChatModel<()>>,
    config: Arc<Config>,
) -> Arc<dyn ChatModel<()>> {
    wrap_injected_controlled(ctx, inner, config, None)
}

/// [`wrap_injected`] with the turn's spend controls beneath attachment rendering.
pub(crate) fn wrap_injected_controlled(
    ctx: &crate::core::runtime::DispatchCtx,
    inner: Arc<dyn ChatModel<()>>,
    config: Arc<Config>,
    controls: Option<&HostControlSlot>,
) -> Arc<dyn ChatModel<()>> {
    let profile = inner.profile().cloned();
    owner::wrap(ctx, inner, config, profile, controls)
}

struct AttachmentModel {
    config: Arc<Config>,
    /// The context the wrapped turn runs under. Weak: a model held by a turn
    /// must not keep a tenant's context alive.
    core: Option<std::sync::Weak<crate::core::runtime::CoreContext>>,
    fallback_cache: std::sync::Mutex<std::collections::HashMap<String, String>>,
    controls: Option<HostControlSlot>,
}

const MAX_HISTORICAL_MEDIA_BLOCKS: usize = 8;

impl AttachmentModel {
    /// The dispatch context of the turn this model serves.
    pub(super) fn dispatch_ctx(&self) -> crate::core::runtime::DispatchCtx {
        crate::core::runtime::DispatchCtx::from_weak(self.core.as_ref())
    }

    async fn read(
        &self,
        path: &str,
        modality: InputModality,
        scope: &AttachmentAccessScope,
    ) -> tinyinference_llm::Result<Vec<u8>> {
        if scope.external_channel {
            return Err(tinyinference_llm::Error::Model(
                "local attachment reads are disabled for external channel input".into(),
            ));
        }
        let path = super::resolve_path(&self.config, path, scope)
            .await
            .map_err(|e| tinyinference_llm::Error::Model(e.to_string()))?;
        let (_, file_mb, _) = self.config.multimodal_files.effective_limits();
        let (_, image_mb) = self.config.multimodal.effective_limits();
        if modality == InputModality::Image && self.config.multimodal.max_images == 0 {
            return Err(tinyinference_llm::Error::Model(
                "image attachments are disabled".into(),
            ));
        }
        let mb = if modality == InputModality::Image {
            image_mb
        } else {
            file_mb
        };
        let root = tokio::fs::canonicalize(super::action_root(&self.config, scope))
            .await
            .map_err(|error| tinyinference_llm::Error::Model(error.to_string()))?;
        let file = secure_open(&path, &root).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                tinyinference_llm::Error::Model(format!(
                    "{} local attachment source disappeared",
                    super::SOURCE_MISSING_MARKER
                ))
            } else {
                tinyinference_llm::Error::Model(error.to_string())
            }
        })?;
        let metadata = file
            .metadata()
            .map_err(|e| tinyinference_llm::Error::Model(e.to_string()))?;
        if !metadata.is_file() || metadata.len() > (mb * 1024 * 1024) as u64 {
            return Err(tinyinference_llm::Error::Model(
                "attachment exceeds configured file limit".into(),
            ));
        }
        use tokio::io::AsyncReadExt;
        let file = tokio::fs::File::from_std(file);
        let mut bytes = Vec::new();
        file.take((mb * 1024 * 1024 + 1) as u64)
            .read_to_end(&mut bytes)
            .await
            .map_err(|e| tinyinference_llm::Error::Model(e.to_string()))?;
        if bytes.len() > mb * 1024 * 1024 {
            return Err(tinyinference_llm::Error::Model(
                "attachment exceeds configured file limit".into(),
            ));
        }
        Ok(bytes)
    }
}

fn historical_source_hint(block: &ContentBlock, scope: &AttachmentAccessScope) -> String {
    let source = match block {
        ContentBlock::Image(image) if is_local_reference(&image.url) => &image.url,
        ContentBlock::Audio(MediaRef::Path { path, .. })
        | ContentBlock::Video(MediaRef::Path { path, .. })
        | ContentBlock::Document(MediaRef::Path { path, .. }) => path,
        _ => return "remote or inline media omitted".into(),
    };
    let path = std::path::Path::new(source);
    let source = if path.is_absolute() {
        let root = scope.workspace.as_deref();
        root.and_then(|root| {
            path.strip_prefix(root)
                .ok()
                .map(std::path::Path::to_path_buf)
        })
        .or_else(|| path.file_name().map(std::path::PathBuf::from))
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
    } else {
        source.to_owned()
    };
    source.chars().take(180).collect()
}

#[path = "provider_owner.rs"]
mod owner;

fn is_local_reference(source: &str) -> bool {
    let bytes = source.as_bytes();
    let windows_drive_path = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'/' | b'\\');
    windows_drive_path || url::Url::parse(source).is_err()
}

#[cfg(any(test, windows))]
use crate::attachments::secure_open::normalize_windows_path_for_comparison;
use crate::attachments::secure_open::secure_open;

#[path = "provider_cache.rs"]
mod cache;
#[path = "provider_fallback.rs"]
mod fallback;
#[cfg(test)]
#[path = "provider_injected_tests.rs"]
mod injected_tests;
#[path = "provider_source.rs"]
mod source;
#[cfg(test)]
#[path = "provider_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "provider_context_tests.rs"]
mod context_tests;

#[cfg(test)]
#[path = "provider_history_tests.rs"]
mod history_tests;

#[cfg(test)]
#[path = "provider_routing_tests.rs"]
mod routing_tests;

#[cfg(test)]
#[path = "provider_security_tests.rs"]
mod security_tests;
