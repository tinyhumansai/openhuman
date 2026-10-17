//! Bounded plain-text derivatives beside durable originals. Originals and
//! transcript references remain unchanged; cache misses never reject uploads.
use super::AttachmentAccessScope;
use super::*;
use sha2::{Digest, Sha256};
const MAX_CACHE_BYTES: u64 = 1024 * 1024;

impl AttachmentModel {
    pub(super) fn fallback_key(&self, path: &str, mime: &str, bytes: &[u8]) -> String {
        let route = if mime.starts_with("image/") || mime == "application/pdf" {
            crate::inference::provider::factory::create_chat_model_with_model_id_in(
                &self.dispatch_ctx(),
                "vision",
                &self.config,
                0.0,
            )
            .ok()
            .map(|(model, id)| {
                format!(
                    "{id}:{}:{}",
                    model
                        .profile()
                        .and_then(|p| p.provider.as_deref())
                        .unwrap_or("unknown"),
                    model.cache_identity().unwrap_or_default()
                )
            })
            .unwrap_or_else(|| "vision-unavailable".into())
        } else {
            "no-vision".into()
        };
        let mut hash = Sha256::new();
        hash.update(b"openhuman-file-intake-v1:pages4:edge2048:");
        hash.update([u8::from(cfg!(feature = "documents"))]);
        #[cfg(feature = "documents")]
        hash.update(
            crate::modules::registry::find("tinydocs")
                .map(|record| record.version)
                .unwrap_or("unavailable")
                .as_bytes(),
        );
        for field in [path, mime, &route] {
            hash.update((field.len() as u64).to_le_bytes());
            hash.update(field.as_bytes());
        }
        hash.update(
            self.config
                .multimodal_files
                .effective_limits()
                .2
                .to_le_bytes(),
        );
        hash.update(bytes);
        format!("{:x}", hash.finalize())
    }
    async fn cache_path(
        &self,
        path: &str,
        scope: &AttachmentAccessScope,
    ) -> Option<std::path::PathBuf> {
        let original = super::super::resolve_path(&self.config, path, scope)
            .await
            .ok()?;
        let root = tokio::fs::canonicalize(super::super::action_root(&self.config, scope))
            .await
            .ok()?;
        if !original.starts_with(root.join("uploads")) {
            return None;
        }
        let cache = original
            .parent()?
            .join(".openhuman-derived")
            .join("readout.txt");
        let policy = crate::security::SecurityPolicy::from_config(
            &self.config.autonomy,
            &self.config.workspace_dir,
            &root,
        );
        if !policy.is_resolved_path_allowed_for(&cache, true) {
            return None;
        }
        Some(cache)
    }
    pub(super) async fn cached_fallback(
        &self,
        path: &str,
        key: &str,
        scope: &AttachmentAccessScope,
    ) -> Option<String> {
        let cache = self.cache_path(path, scope).await?;
        let parent = tokio::fs::symlink_metadata(cache.parent()?).await.ok()?;
        if !parent.is_dir() || parent.file_type().is_symlink() {
            return None;
        }
        let metadata = tokio::fs::symlink_metadata(&cache).await.ok()?;
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || metadata.len() > MAX_CACHE_BYTES
        {
            return None;
        }
        use tokio::io::AsyncReadExt;
        let file = tokio::fs::File::open(cache).await.ok()?;
        let mut bytes = Vec::new();
        file.take(MAX_CACHE_BYTES + 1)
            .read_to_end(&mut bytes)
            .await
            .ok()?;
        if bytes.len() as u64 > MAX_CACHE_BYTES {
            return None;
        }
        let content = String::from_utf8(bytes).ok()?;
        let (stored_key, text) = content.split_once('\n')?;
        (stored_key == key).then(|| text.to_owned())
    }
    pub(super) async fn save_fallback(
        &self,
        path: &str,
        key: &str,
        text: &str,
        scope: &AttachmentAccessScope,
    ) {
        if (text.len() + key.len() + 1) as u64 > MAX_CACHE_BYTES {
            return;
        }
        let Some(cache) = self.cache_path(path, scope).await else {
            return;
        };
        let parent = cache.parent().expect("derivative parent");
        let root = super::super::action_root(&self.config, scope);
        let policy = crate::security::SecurityPolicy::from_config(
            &self.config.autonomy,
            &self.config.workspace_dir,
            &root,
        );
        if policy
            .validate_parent_path(&cache.to_string_lossy())
            .await
            .is_err()
        {
            return;
        }
        match tokio::fs::create_dir(parent).await {
            Ok(()) => (),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(_) => return,
        };
        let Ok(metadata) = tokio::fs::symlink_metadata(parent).await else {
            return;
        };
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return;
        }
        if policy
            .validate_parent_path(&cache.to_string_lossy())
            .await
            .is_err()
        {
            return;
        }
        let temp = cache.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
        use tokio::io::AsyncWriteExt;
        let result = async {
            let mut file = tokio::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)
                .await?;
            file.write_all(key.as_bytes()).await?;
            file.write_all(b"\n").await?;
            file.write_all(text.as_bytes()).await?;
            file.sync_all().await?;
            tokio::fs::rename(&temp, cache).await
        }
        .await;
        if result.is_err() {
            let _ = tokio::fs::remove_file(temp).await;
        }
    }
}
