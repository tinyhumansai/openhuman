//! Ephemeral provider request preparation for durable workspace attachments.
use super::AttachmentAccessScope;
use crate::config::Config;
use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD, Engine};
use std::sync::Arc;
use tinyinference_llm::{
    message::{ContentBlock, ImageRef, MediaRef, Message},
    model::{
        ChatModel, InputModality, InputSource, ModelProfile, ModelRequest, ModelResponse,
        ModelStream,
    },
};

pub(crate) fn wrap(
    ctx: &crate::core::runtime::DispatchCtx,
    inner: Arc<dyn ChatModel<()>>,
    config: Arc<Config>,
    model: &str,
    provider: &str,
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
        profile.modalities.image_in =
            crate::inference::provider::factory::oh_tier_supports_vision(model);
    }
    let route = if matches!(provider, "openhuman" | "managed") {
        "openhuman".to_string()
    } else {
        format!("{provider}:{model}")
    };
    if let Some(request) =
        crate::inference::provider::factory::model_limits_request("chat", &route, model, &config)
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
    Arc::new(AttachmentModel {
        inner,
        config,
        core: ctx.downgrade(),
        profile,
        fallback_cache: std::sync::Mutex::new(std::collections::HashMap::new()),
    })
}

/// Injected models supply their own selected-model facts through ModelProfile.
/// Transport support is still required for every native media block.
pub(crate) fn wrap_injected(
    ctx: &crate::core::runtime::DispatchCtx,
    inner: Arc<dyn ChatModel<()>>,
    config: Arc<Config>,
) -> Arc<dyn ChatModel<()>> {
    let profile = inner.profile().cloned().unwrap_or_default();
    Arc::new(AttachmentModel {
        inner,
        config,
        core: ctx.downgrade(),
        profile,
        fallback_cache: std::sync::Mutex::new(std::collections::HashMap::new()),
    })
}

struct AttachmentModel {
    inner: Arc<dyn ChatModel<()>>,
    config: Arc<Config>,
    /// The context the wrapped turn runs under. Weak: a model held by a turn
    /// must not keep a tenant's context alive.
    core: Option<std::sync::Weak<crate::core::runtime::CoreContext>>,
    profile: ModelProfile,
    fallback_cache: std::sync::Mutex<std::collections::HashMap<String, String>>,
}

const MAX_HISTORICAL_MEDIA_BLOCKS: usize = 8;

fn is_media_block(block: &ContentBlock) -> bool {
    matches!(
        block,
        ContentBlock::Image(_)
            | ContentBlock::Audio(_)
            | ContentBlock::Video(_)
            | ContentBlock::Document(_)
    )
}

fn recoverable_historical_media_error(error: &tinyinference_llm::Error) -> bool {
    let error = error.to_string();
    error.contains(super::SOURCE_MISSING_MARKER)
        || error.contains("attachments are disabled for this origin")
        || error.contains("remote attachment fetch is disabled")
}

impl AttachmentModel {
    /// The dispatch context of the turn this model serves.
    pub(super) fn dispatch_ctx(&self) -> crate::core::runtime::DispatchCtx {
        crate::core::runtime::DispatchCtx::from_weak(self.core.as_ref())
    }

    fn native(&self, modality: InputModality, mime: &str) -> bool {
        let known = match modality {
            InputModality::Image => self.profile.modalities.image_in,
            InputModality::Audio => self.profile.modalities.audio_in,
            InputModality::Video => self.profile.modalities.video_in,
            InputModality::Document => self.profile.modalities.document_in,
        };
        known
            && self
                .inner
                .supports_input(modality, mime, InputSource::Base64)
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
    async fn prepare(&self, mut request: ModelRequest) -> tinyinference_llm::Result<ModelRequest> {
        let scope = super::take_request_scope(&mut request.metadata);
        if let Some(Message::User(user)) = request
            .messages
            .iter()
            .rev()
            .find(|m| matches!(m, Message::User(_)))
        {
            let images = user
                .content
                .iter()
                .filter(|b| matches!(b, ContentBlock::Image(_)))
                .count();
            let files = user
                .content
                .iter()
                .filter(|b| {
                    matches!(
                        b,
                        ContentBlock::Audio(_) | ContentBlock::Video(_) | ContentBlock::Document(_)
                    )
                })
                .count();
            let (max_images, _) = self.config.multimodal.effective_limits();
            let (max_files, _, _) = self.config.multimodal_files.effective_limits();
            if images > max_images
                || files > max_files
                || self.config.multimodal_files.max_files == 0 && images + files > 0
            {
                return Err(tinyinference_llm::Error::Model(
                    "attachment count exceeds configured limit".into(),
                ));
            }
        }
        let latest_user_message = request
            .messages
            .iter()
            .rposition(|message| matches!(message, Message::User(_)));
        let mut historical_media_to_resolve = std::collections::HashSet::new();
        let mut remaining_historical = MAX_HISTORICAL_MEDIA_BLOCKS;
        for (message_index, message) in request.messages.iter().enumerate().rev() {
            if Some(message_index) == latest_user_message {
                continue;
            }
            let Message::User(user) = message else {
                continue;
            };
            for (block_index, block) in user.content.iter().enumerate().rev() {
                if is_media_block(block) && remaining_historical > 0 {
                    historical_media_to_resolve.insert((message_index, block_index));
                    remaining_historical -= 1;
                }
            }
        }
        if scope.external_channel
            && request.messages.iter().any(|message| {
                matches!(message, Message::User(user) if user.content.iter().any(is_media_block))
            })
        {
            return Err(tinyinference_llm::Error::Model(
                "attachment resolution is disabled for external channel input".into(),
            ));
        }
        for (message_index, message) in request.messages.iter_mut().enumerate() {
            let Message::User(user) = message else {
                continue;
            };
            let mut out = Vec::new();
            for (block_index, block) in std::mem::take(&mut user.content).into_iter().enumerate() {
                if Some(message_index) != latest_user_message
                    && is_media_block(&block)
                    && !historical_media_to_resolve.contains(&(message_index, block_index))
                {
                    let hint = historical_source_hint(&block, &scope);
                    out.push(ContentBlock::Text(format!(
                        "[Earlier attachment omitted by history budget; workspace path: {hint}]"
                    )));
                    continue;
                }
                let resolved = match self.resolve_block(&block, &scope).await {
                    Ok(resolved) => resolved,
                    Err(error)
                        if Some(message_index) != latest_user_message
                            && recoverable_historical_media_error(&error) =>
                    {
                        let hint = historical_source_hint(&block, &scope);
                        out.push(ContentBlock::Text(format!(
                            "[Earlier attachment unavailable or omitted at workspace path: {hint}]"
                        )));
                        continue;
                    }
                    Err(error) => return Err(error),
                };
                let Some((modality, path, mut mime, bytes, recovered)) = resolved else {
                    out.push(block);
                    continue;
                };
                if modality == InputModality::Image && bytes.is_empty() {
                    return Err(tinyinference_llm::Error::Model(
                        "image reference has no bytes".into(),
                    ));
                }
                if modality == InputModality::Image {
                    mime = tinyagents_harness::multimodal::mime::image_mime_from_magic(&bytes)
                        .map(str::to_owned)
                        .or_else(|| {
                            tinyagents_harness::multimodal::mime::detect_image_mime(
                                Some(std::path::Path::new(&path)),
                                &bytes,
                                None,
                            )
                        })
                        .ok_or_else(|| {
                            tinyinference_llm::Error::Model(
                                "image attachment type cannot be determined".into(),
                            )
                        })?;
                }
                if self.native(modality, &mime) {
                    if recovered {
                        out.push(ContentBlock::Text(format!(
                            "[Recovered attachment: {mime}; {} bytes; workspace path: {path}]",
                            bytes.len()
                        )));
                    }

                    let bytes = if mime == "image/png" {
                        tinyagents_harness::multimodal::optimize_png_lossless(&bytes)
                            .unwrap_or(bytes)
                    } else {
                        bytes
                    };
                    let data = STANDARD.encode(bytes);
                    out.push(match modality {
                        InputModality::Image => ContentBlock::Image(ImageRef {
                            url: format!("data:{mime};base64,{data}"),
                            mime_type: Some(mime),
                        }),
                        InputModality::Audio => ContentBlock::Audio(MediaRef::base64(data, mime)),
                        InputModality::Video => ContentBlock::Video(MediaRef::base64(data, mime)),
                        InputModality::Document => {
                            ContentBlock::Document(MediaRef::base64(data, mime))
                        }
                    });
                } else {
                    let key = self.fallback_key(&path, &mime, &bytes);
                    let cached = self
                        .fallback_cache
                        .lock()
                        .expect("fallback cache")
                        .get(&key)
                        .cloned();
                    let cached = match cached {
                        Some(text) => Some(text),
                        None => self.cached_fallback(&path, &key, &scope).await,
                    };
                    let text = match cached {
                        Some(text) => text,
                        None => {
                            let (text, cacheable) =
                                self.fallback(modality, &path, &mime, &bytes).await?;
                            if cacheable {
                                self.save_fallback(&path, &key, &text, &scope).await;
                            }
                            let mut cache = self.fallback_cache.lock().expect("fallback cache");
                            if cache.len() < 16 {
                                cache.insert(key, text.clone());
                            }
                            text
                        }
                    };
                    out.push(ContentBlock::Text(text));
                }
            }
            user.content = out;
        }
        Ok(request)
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

fn is_local_reference(source: &str) -> bool {
    let bytes = source.as_bytes();
    let windows_drive_path = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'/' | b'\\');
    windows_drive_path || url::Url::parse(source).is_err()
}

#[path = "provider_open.rs"]
pub(crate) mod open;
#[cfg(any(test, windows))]
use open::normalize_windows_path_for_comparison;
use open::secure_open;

#[async_trait]
impl ChatModel<()> for AttachmentModel {
    fn profile(&self) -> Option<&ModelProfile> {
        Some(&self.profile)
    }
    fn cache_identity(&self) -> Option<String> {
        self.inner.cache_identity()
    }
    fn supports_input(&self, m: InputModality, t: &str, s: InputSource) -> bool {
        self.inner.supports_input(m, t, s)
    }
    async fn invoke(
        &self,
        state: &(),
        request: ModelRequest,
    ) -> tinyinference_llm::Result<ModelResponse> {
        self.inner.invoke(state, self.prepare(request).await?).await
    }
    async fn stream(
        &self,
        state: &(),
        request: ModelRequest,
    ) -> tinyinference_llm::Result<ModelStream> {
        self.inner.stream(state, self.prepare(request).await?).await
    }
}

#[path = "provider_cache.rs"]
mod cache;
#[path = "provider_fallback.rs"]
mod fallback;
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
