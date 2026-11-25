//! OpenHuman adapters for TinyAgents' generic media request decorator.
use super::*;
use async_trait::async_trait;
use tinyagents_harness::multimodal::{
    config::{FileLimits, ImageLimits},
    MediaFallbackCache, MediaFallbackRenderer, MediaHistoryFallback, MediaRequestConfig,
    MediaRequestModel, MediaSourceKind, MediaSourceMetadata, MediaSourceResolver,
    ResolvedAttachment, ResolvedMedia, TextExtractor,
};

pub(super) fn wrap(
    ctx: &crate::core::runtime::DispatchCtx,
    inner: Arc<dyn ChatModel<()>>,
    config: Arc<Config>,
    profile: Option<ModelProfile>,
    controls: Option<&HostControlSlot>,
) -> Arc<dyn ChatModel<()>> {
    let inner = match controls {
        Some(controls) => controls.wrap(inner),
        None => inner,
    };
    let host = Arc::new(AttachmentModel {
        config: config.clone(),
        core: ctx.downgrade(),
        fallback_cache: std::sync::Mutex::new(std::collections::HashMap::new()),
        controls: controls.cloned(),
    });
    let (max_images, image_mb) = config.multimodal.effective_limits();
    let (max_files, file_mb, text_limit) = config.multimodal_files.effective_limits();
    let request_config = MediaRequestConfig {
        images: ImageLimits {
            max_images,
            max_image_size_mb: image_mb,
            allow_remote_fetch: config.multimodal.allow_remote_fetch,
        },
        files: FileLimits {
            max_files: if config.multimodal_files.max_files == 0 {
                0
            } else {
                max_files
            },
            max_file_size_mb: file_mb,
            max_extracted_text_chars: text_limit,
            allow_remote_fetch: config.multimodal_files.allow_remote_fetch,
            allowed_mime_types: config.multimodal_files.allowed_mime_types.clone(),
        },
        max_historical_media_items: MAX_HISTORICAL_MEDIA_BLOCKS,
    };
    let extractor = Arc::new(OwnedDocumentsExtractor {
        ctx: ctx.downgrade(),
    });
    let model = MediaRequestModel::new(
        inner,
        request_config,
        |_, request| Ok(super::super::take_request_scope(&mut request.metadata)),
        host.clone(),
        extractor,
    );
    let model = if let Some(profile) = profile {
        model.with_profile(profile)
    } else {
        model
    };
    let model = model
        .with_fallback_renderer(host.clone())
        .with_fallback_cache(host.clone())
        .with_history_fallback(host.clone());
    Arc::new(model)
}

#[async_trait]
impl MediaSourceResolver<AttachmentAccessScope> for AttachmentModel {
    async fn resolve(
        &self,
        scope: &AttachmentAccessScope,
        modality: InputModality,
        source: &str,
        max_bytes: usize,
    ) -> Result<ResolvedMedia, String> {
        self.resolve_with_mime_hint(scope, modality, source, max_bytes, None)
            .await
    }

    async fn resolve_with_mime_hint(
        &self,
        scope: &AttachmentAccessScope,
        modality: InputModality,
        source: &str,
        max_bytes: usize,
        mime_hint: Option<&str>,
    ) -> Result<ResolvedMedia, String> {
        self.resolve_with_source_metadata(
            scope,
            modality,
            source,
            max_bytes,
            &MediaSourceMetadata {
                kind: MediaSourceKind::Untyped,
                mime_hint: mime_hint.map(str::to_owned),
                encoded_len: None,
            },
        )
        .await
    }

    async fn resolve_with_source_metadata(
        &self,
        scope: &AttachmentAccessScope,
        modality: InputModality,
        source: &str,
        max_bytes: usize,
        metadata: &MediaSourceMetadata,
    ) -> Result<ResolvedMedia, String> {
        // Staged native attachments are opaque host references that
        // `resolve_block` authorizes against the thread, not URLs to fetch.
        let staged = source.starts_with("openhuman-staged://");
        if !staged
            && ((metadata.kind == MediaSourceKind::Url
                && !source.starts_with("data:")
                && !source.starts_with("http://")
                && !source.starts_with("https://"))
                || (metadata.kind == MediaSourceKind::Untyped
                    && !source.starts_with("data:")
                    && source.contains("://")
                    && !source.starts_with("http://")
                    && !source.starts_with("https://")))
        {
            return Err("unsupported media URL scheme".into());
        }
        if metadata.kind == MediaSourceKind::Base64
            && metadata.encoded_len.unwrap_or(source.len()) > 50 * 1024 * 1024 * 4 / 3 + 4
        {
            return Err("inline media exceeds size limit".into());
        }
        let block = match modality {
            InputModality::Image => ContentBlock::Image(ImageRef {
                url: source.to_owned(),
                mime_type: metadata.mime_hint.clone(),
            }),
            InputModality::Audio | InputModality::Video | InputModality::Document => {
                let mime = metadata.mime_hint.clone();
                let media = match metadata.kind {
                    MediaSourceKind::Path => MediaRef::Path {
                        path: source.to_owned(),
                        media_type: mime,
                    },
                    MediaSourceKind::Url => MediaRef::Url {
                        url: source.to_owned(),
                        media_type: mime,
                    },
                    MediaSourceKind::Base64 => MediaRef::Base64 {
                        data: source.to_owned(),
                        media_type: mime.unwrap_or_else(|| "application/octet-stream".into()),
                    },
                    MediaSourceKind::Untyped
                        if source.starts_with("data:")
                            || source.starts_with("http://")
                            || source.starts_with("https://") =>
                    {
                        MediaRef::Url {
                            url: source.to_owned(),
                            media_type: mime,
                        }
                    }
                    MediaSourceKind::Untyped => MediaRef::Path {
                        path: source.to_owned(),
                        media_type: mime,
                    },
                };
                match modality {
                    InputModality::Audio => ContentBlock::Audio(media),
                    InputModality::Video => ContentBlock::Video(media),
                    _ => ContentBlock::Document(media),
                }
            }
        };
        let Some((resolved_modality, path, mut mime, bytes, recovered)) = self
            .resolve_block(
                &block, scope,
                // The generic resolver port carries no message index, so staged
                // attachments committed to this thread stay readable for
                // history replay; `read_authorized_native_attachment` still
                // requires the record to be bound to the thread.
                true,
            )
            .await
            .map_err(|error| error.to_string())?
        else {
            return Err("unsupported media source".into());
        };
        if bytes.len() > max_bytes || resolved_modality != modality {
            return Err("attachment exceeds configured file limit".into());
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
                .ok_or_else(|| "image attachment type cannot be determined".to_string())?;
        } else if let Some(mime_hint) = metadata.mime_hint.as_deref() {
            // Typed per-block metadata is authoritative for that reference.
            // Marker references pass no hint and retain source detection.
            mime = mime_hint.to_owned();
        } else if mime == "application/octet-stream" {
            mime = tinyagents_harness::multimodal::mime::detect_file_mime(
                Some(std::path::Path::new(&path)),
                &bytes,
                None,
            )
            .unwrap_or(mime);
        }
        let mut resolved = ResolvedMedia::new(ResolvedAttachment {
            size_bytes: bytes.len(),
            bytes,
            name: path,
            mime,
        });
        if recovered {
            let annotation = format!(
                "[Recovered attachment: {}; {} bytes; workspace path: {}]",
                resolved.attachment.mime, resolved.attachment.size_bytes, resolved.attachment.name
            );
            resolved = resolved.with_annotation(annotation);
        }
        Ok(resolved)
    }
}

#[async_trait]
impl MediaFallbackRenderer<AttachmentAccessScope> for AttachmentModel {
    async fn render(
        &self,
        _scope: &AttachmentAccessScope,
        modality: InputModality,
        attachment: &ResolvedAttachment,
    ) -> Result<Option<String>, String> {
        if modality == InputModality::Image {
            if let Some(text) = MediaFallbackCache::get(self, _scope, modality, attachment).await {
                return Ok(Some(text));
            }
            let text = format!(
                "{}\nImage readout:\n{}",
                attachment_header(attachment),
                self.image_readout(&attachment.mime, &attachment.bytes)
                    .await
                    .map_err(|error| error.to_string())?
            );
            let text = self.limit_context(text);
            MediaFallbackCache::put(self, _scope, modality, attachment, &text).await;
            return Ok(Some(text));
        }
        #[cfg(feature = "documents")]
        if matches!(
            attachment.mime.as_str(),
            "application/pdf"
                | "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
                | "application/vnd.openxmlformats-officedocument.presentationml.presentation"
                | "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
        ) {
            if let Some(text) = MediaFallbackCache::get(self, _scope, modality, attachment).await {
                return Ok(Some(text));
            }
            let (body, cacheable) = self
                .document_fallback(&attachment.mime, &attachment.bytes)
                .await
                .map_err(|error| error.to_string())?;
            let text = format!("{}\n{body}", attachment_header(attachment));
            if cacheable {
                MediaFallbackCache::put(self, _scope, modality, attachment, &text).await;
            }
            return Ok(Some(text));
        }
        Ok(None)
    }
}

fn attachment_header(attachment: &ResolvedAttachment) -> String {
    format!(
        "[Attachment: {}; {} bytes; workspace path: {}]",
        attachment.mime, attachment.size_bytes, attachment.name
    )
}

#[async_trait]
impl MediaFallbackCache<AttachmentAccessScope> for AttachmentModel {
    async fn get(
        &self,
        scope: &AttachmentAccessScope,
        _modality: InputModality,
        attachment: &ResolvedAttachment,
    ) -> Option<String> {
        let key = self
            .fallback_key(&attachment.name, &attachment.mime, &attachment.bytes)
            .await;
        if let Some(text) = self.fallback_cache.lock().ok()?.get(&key).cloned() {
            return Some(text);
        }
        let text = self.cached_fallback(&attachment.name, &key, scope).await?;
        if let Ok(mut cache) = self.fallback_cache.lock() {
            cache.insert(key, text.clone());
        }
        Some(text)
    }

    async fn put(
        &self,
        scope: &AttachmentAccessScope,
        _modality: InputModality,
        attachment: &ResolvedAttachment,
        text: &str,
    ) {
        let key = self
            .fallback_key(&attachment.name, &attachment.mime, &attachment.bytes)
            .await;
        if let Ok(mut cache) = self.fallback_cache.lock() {
            cache.insert(key.clone(), text.to_owned());
        }
        self.save_fallback(&attachment.name, &key, text, scope)
            .await;
    }
}

impl MediaHistoryFallback<AttachmentAccessScope> for AttachmentModel {
    fn omitted(
        &self,
        scope: &AttachmentAccessScope,
        modality: InputModality,
        source: &str,
    ) -> String {
        let block = history_source_block(modality, source);
        let hint = super::historical_source_hint(&block, scope);
        format!("[Earlier attachment omitted by history budget; workspace path: {hint}]")
    }

    fn unavailable(
        &self,
        scope: &AttachmentAccessScope,
        modality: InputModality,
        source: &str,
        error: &str,
    ) -> Option<String> {
        let error = error.to_owned();
        if !error.contains(super::super::SOURCE_MISSING_MARKER)
            && !error.contains("attachments are disabled for this origin")
            && !error.contains("remote attachment fetch is disabled")
        {
            return None;
        }
        let block = history_source_block(modality, source);
        let hint = super::historical_source_hint(&block, scope);
        Some(format!(
            "[Earlier attachment unavailable or omitted at workspace path: {hint}]"
        ))
    }
}

fn history_source_block(modality: InputModality, source: &str) -> ContentBlock {
    match modality {
        InputModality::Image => ContentBlock::Image(ImageRef {
            url: source.to_owned(),
            mime_type: None,
        }),
        modality => {
            let media = if !super::is_local_reference(source) {
                MediaRef::Url {
                    url: source.to_owned(),
                    media_type: None,
                }
            } else {
                MediaRef::Path {
                    path: source.to_owned(),
                    media_type: None,
                }
            };
            match modality {
                InputModality::Audio => ContentBlock::Audio(media),
                InputModality::Video => ContentBlock::Video(media),
                InputModality::Document => ContentBlock::Document(media),
                InputModality::Image => unreachable!(),
            }
        }
    }
}

#[cfg(test)]
#[path = "provider_owner_tests.rs"]
mod tests;

struct OwnedDocumentsExtractor {
    ctx: Option<std::sync::Weak<crate::core::runtime::CoreContext>>,
}

#[async_trait]
impl TextExtractor for OwnedDocumentsExtractor {
    #[cfg(feature = "documents")]
    fn handles(&self, mime: &str) -> bool {
        mime == "application/pdf"
    }

    #[cfg(not(feature = "documents"))]
    fn handles(&self, _: &str) -> bool {
        false
    }

    async fn extract(&self, mime: &str, bytes: &[u8]) -> Result<String, String> {
        let ctx = crate::core::runtime::DispatchCtx::from_weak(self.ctx.as_ref());
        crate::agent::multimodal::DocumentsTextExtractor { ctx: &ctx }
            .extract(mime, bytes)
            .await
    }
}
