//! Resolve every admitted media source on a disposable provider request.
use super::super::{is_missing_local_reference, SOURCE_MISSING_MARKER};
use super::AttachmentAccessScope;
use super::*;
use sha2::{Digest, Sha256};
use tinyagents_harness::multimodal::{
    config::FileLimits, resolve::resolve_attachment, UnknownMimePolicy,
};

impl AttachmentModel {
    pub(super) async fn resolve_block(
        &self,
        block: &ContentBlock,
        scope: &AttachmentAccessScope,
        allow_committed_history: bool,
    ) -> tinyinference_llm::Result<Option<(InputModality, String, String, Vec<u8>, bool)>> {
        let (modality, source, mime, is_local) = match block {
            ContentBlock::Image(image) => (
                InputModality::Image,
                image.url.clone(),
                image.mime_type.clone(),
                !image.url.starts_with("data:")
                    && !image.url.starts_with("http://")
                    && !image.url.starts_with("https://"),
            ),
            ContentBlock::Audio(media) => Self::media_source(InputModality::Audio, media)?,
            ContentBlock::Video(media) => Self::media_source(InputModality::Video, media)?,
            ContentBlock::Document(media) => Self::media_source(InputModality::Document, media)?,
            _ => return Ok(None),
        };
        if let Some(opaque_id) = source.strip_prefix("openhuman-staged://") {
            let thread_id = scope.thread_id.as_deref().ok_or_else(|| {
                tinyinference_llm::Error::Model("native attachment session is unavailable".into())
            })?;
            let resolved = crate::threads::attachments::read_authorized_native_attachment(
                &self.dispatch_ctx(),
                &self.config,
                thread_id,
                opaque_id,
                &scope.native_staged_parts,
                allow_committed_history,
            )
            .await
            .map_err(tinyinference_llm::Error::Model)?;
            let expected_image = modality == InputModality::Image;
            if expected_image != resolved.mime.starts_with("image/") {
                return Err(tinyinference_llm::Error::Model(
                    "native attachment type does not match its staged record".into(),
                ));
            }
            return Ok(Some((
                modality,
                resolved.path,
                resolved.mime,
                resolved.bytes,
                false,
            )));
        }
        if self.config.multimodal_files.max_files == 0 {
            return Err(tinyinference_llm::Error::Model(
                "attachments are disabled for this origin".into(),
            ));
        }
        if scope.external_channel {
            return Err(tinyinference_llm::Error::Model(
                "attachment resolution is disabled for external channel input".into(),
            ));
        }
        if is_local {
            if is_missing_local_reference(&self.config, &source, scope).await {
                return Err(tinyinference_llm::Error::Model(format!(
                    "{SOURCE_MISSING_MARKER} local attachment source is missing"
                )));
            }
            // Old transcripts can point into the private host image stash.
            // Publish only its validated managed originals into the acting
            // workspace, and retain that stable path for metadata and caches.
            let migrated = super::super::legacy::migrate_path(&self.config, &source, scope)
                .await
                .map_err(|e| tinyinference_llm::Error::Model(e.to_string()))?;
            let recovered = migrated.is_some();
            let source = migrated.unwrap_or(source);
            if is_missing_local_reference(&self.config, &source, scope).await {
                return Err(tinyinference_llm::Error::Model(format!(
                    "{SOURCE_MISSING_MARKER} local attachment source is missing"
                )));
            }
            let bytes = self.read(&source, modality, scope).await?;
            return Ok(Some((
                modality,
                source,
                mime.unwrap_or_else(|| "application/octet-stream".into()),
                bytes,
                recovered,
            )));
        }
        let (_, image_mb) = self.config.multimodal.effective_limits();
        let (_, file_mb, text_limit) = self.config.multimodal_files.effective_limits();
        let mb = if modality == InputModality::Image {
            image_mb
        } else {
            file_mb
        };
        if modality == InputModality::Image && self.config.multimodal.max_images == 0 {
            return Err(tinyinference_llm::Error::Model(
                "image attachments are disabled".into(),
            ));
        }
        let allow_remote_fetch = if modality == InputModality::Image {
            self.config.multimodal.allow_remote_fetch
        } else {
            self.config.multimodal_files.allow_remote_fetch
        };
        if (source.starts_with("http://") || source.starts_with("https://")) && !allow_remote_fetch
        {
            return Err(tinyinference_llm::Error::Model(
                "remote attachment fetch is disabled".into(),
            ));
        }
        if source.starts_with("data:") && source.len() > mb * 1024 * 1024 * 4 / 3 + 4096 {
            return Err(tinyinference_llm::Error::Model(
                "attachment exceeds configured file limit".into(),
            ));
        }
        let client =
            crate::config::build_runtime_proxy_client_with_timeouts("inference.attachment", 30, 10);
        let limits = FileLimits {
            max_files: 1,
            max_file_size_mb: mb,
            max_extracted_text_chars: text_limit,
            allow_remote_fetch,
            allowed_mime_types: Vec::new(),
        };
        let resolved = resolve_attachment(
            &source,
            &limits,
            mb * 1024 * 1024,
            &client,
            UnknownMimePolicy::Accept,
        )
        .await
        .map_err(|e| tinyinference_llm::Error::Model(e.to_string()))?;
        if resolved.bytes.is_empty() {
            return Err(tinyinference_llm::Error::Model(
                "media reference has no bytes".into(),
            ));
        }
        let mime = if resolved.mime == "application/octet-stream" {
            mime.unwrap_or(resolved.mime)
        } else {
            resolved.mime
        };
        let path = self
            .materialize_legacy(&resolved.name, &mime, &resolved.bytes, modality, scope)
            .await?;
        Ok(Some((modality, path, mime, resolved.bytes, true)))
    }
    fn media_source(
        modality: InputModality,
        media: &MediaRef,
    ) -> tinyinference_llm::Result<(InputModality, String, Option<String>, bool)> {
        Ok(match media {
            MediaRef::Path { path, media_type } => {
                (modality, path.clone(), media_type.clone(), true)
            }
            MediaRef::Url { url, media_type } => {
                if !url.starts_with("data:")
                    && !url.starts_with("http://")
                    && !url.starts_with("https://")
                {
                    return Err(tinyinference_llm::Error::Model(
                        "unsupported media URL scheme".into(),
                    ));
                }
                (modality, url.clone(), media_type.clone(), false)
            }
            MediaRef::Base64 { data, media_type } => {
                // Bound before allocating the data URI. The per-modality limit
                // is applied again by the resolver before decoded allocation.
                if data.len() > 50 * 1024 * 1024 * 4 / 3 + 4 {
                    return Err(tinyinference_llm::Error::Model(
                        "inline media exceeds size limit".into(),
                    ));
                }
                (
                    modality,
                    format!("data:{media_type};base64,{data}"),
                    Some(media_type.clone()),
                    false,
                )
            }
        })
    }
    async fn materialize_legacy(
        &self,
        name: &str,
        mime: &str,
        bytes: &[u8],
        modality: InputModality,
        scope: &AttachmentAccessScope,
    ) -> tinyinference_llm::Result<String> {
        let mut hash = Sha256::new();
        hash.update((mime.len() as u64).to_le_bytes());
        hash.update(mime.as_bytes());
        hash.update(bytes);
        let id = format!("{:x}", hash.finalize());
        let relative = std::path::PathBuf::from("uploads")
            .join("legacy-media")
            .join(id)
            .join(super::super::filename(name));
        let root = super::super::action_root(&self.config, scope);
        let preliminary = crate::security::SecurityPolicy::from_config(
            &self.config.autonomy,
            &self.config.workspace_dir,
            &root,
        );
        preliminary
            .validate_parent_path(&root.join(&relative).to_string_lossy())
            .await
            .map_err(tinyinference_llm::Error::Model)?;
        tokio::fs::create_dir_all(&root)
            .await
            .map_err(|e| tinyinference_llm::Error::Model(e.to_string()))?;
        let root = tokio::fs::canonicalize(root)
            .await
            .map_err(|e| tinyinference_llm::Error::Model(e.to_string()))?;
        let policy = crate::security::SecurityPolicy::from_config(
            &self.config.autonomy,
            &self.config.workspace_dir,
            &root,
        );
        let path = root.join(&relative);
        policy
            .validate_parent_path(&path.to_string_lossy())
            .await
            .map_err(tinyinference_llm::Error::Model)?;
        let mut parent = root.clone();
        for component in relative
            .parent()
            .expect("legacy upload parent")
            .components()
        {
            parent.push(component);
            match tokio::fs::create_dir(&parent).await {
                Ok(()) => (),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
                Err(e) => return Err(tinyinference_llm::Error::Model(e.to_string())),
            }
            let metadata = tokio::fs::symlink_metadata(&parent)
                .await
                .map_err(|e| tinyinference_llm::Error::Model(e.to_string()))?;
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err(tinyinference_llm::Error::Model(
                    "legacy upload directory is not a regular directory".into(),
                ));
            }
        }
        use tokio::io::AsyncWriteExt;
        let temp = path.with_file_name(format!(".legacy-{}.tmp", uuid::Uuid::new_v4()));
        let write = async {
            let mut file = tokio::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temp)
                .await?;
            file.write_all(bytes).await?;
            file.sync_all().await
        }
        .await;
        if let Err(error) = write {
            let _ = tokio::fs::remove_file(&temp).await;
            return Err(tinyinference_llm::Error::Model(error.to_string()));
        }
        let linked = tokio::fs::hard_link(&temp, &path).await;
        let _ = tokio::fs::remove_file(&temp).await;
        match linked {
            Ok(()) => (),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let saved = self
                    .read(&relative.to_string_lossy(), modality, scope)
                    .await?;
                if saved != bytes {
                    return Err(tinyinference_llm::Error::Model(
                        "legacy upload original has changed".into(),
                    ));
                }
            }
            Err(_) => {
                // Some user filesystems cannot hard-link. Preserve bytes using
                // the normal collision-free upload store rather than overwrite.
                return super::super::save(&self.config, "legacy-media", name, mime, bytes, scope)
                    .await
                    .map(|a| a.path)
                    .map_err(|e| tinyinference_llm::Error::Model(e.to_string()));
            }
        }
        Ok(relative.to_string_lossy().replace('\\', "/"))
    }
}
