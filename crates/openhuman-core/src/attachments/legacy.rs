//! Read only exact host-owned legacy image sidecars, then publish an acting
//! workspace original. This is not an exemption for arbitrary private paths.
use super::*;

pub(crate) async fn migrate_path(
    config: &Config,
    reference: &str,
    scope: &AttachmentAccessScope,
) -> anyhow::Result<Option<String>> {
    migrate_from(
        config,
        reference,
        &crate::agent::multimodal::attachments_dir(),
        scope,
    )
    .await
}

pub(super) async fn migrate_from(
    config: &Config,
    reference: &str,
    stash: &Path,
    scope: &AttachmentAccessScope,
) -> anyhow::Result<Option<String>> {
    let source = Path::new(reference);
    // Rehydration emits direct absolute children from the host's stash index.
    // Never interpret a relative path, nested member or unrelated file as one.
    if !source.is_absolute() || source.parent() != Some(stash) {
        return Ok(None);
    }
    if config.multimodal_files.max_files == 0 || config.multimodal.max_images == 0 {
        anyhow::bail!("legacy attachments are disabled");
    }
    if scope.external_channel {
        anyhow::bail!("legacy attachment reads are disabled for external channel input");
    }
    let stem = source
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    if stem.is_empty()
        || !stem
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    {
        anyhow::bail!("invalid legacy attachment id");
    }
    if !source
        .extension()
        .and_then(|s| s.to_str())
        .is_some_and(|ext| matches!(ext, "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "img"))
    {
        anyhow::bail!("invalid legacy image extension");
    }
    let root_metadata = tokio::fs::symlink_metadata(stash).await?;
    let metadata = tokio::fs::symlink_metadata(source).await?;
    if !root_metadata.is_dir()
        || root_metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.file_type().is_symlink()
    {
        anyhow::bail!("legacy sidecar is not a regular managed file");
    }
    let root = tokio::fs::canonicalize(stash).await?;
    let canonical = tokio::fs::canonicalize(source).await?;
    if canonical.parent() != Some(root.as_path())
        || crate::security::floor::any_protected(&[stash, source, canonical.as_path()]).await
    {
        anyhow::bail!("legacy sidecar escapes the permitted image store");
    }
    let (_, mb) = config.multimodal.effective_limits();
    let cap = mb * 1024 * 1024;
    if metadata.len() > cap as u64 {
        anyhow::bail!("legacy image exceeds configured size limit");
    }
    use tokio::io::AsyncReadExt;
    let file = tokio::fs::File::open(&canonical).await?;
    let mut bytes = Vec::new();
    file.take((cap + 1) as u64).read_to_end(&mut bytes).await?;
    if bytes.len() > cap {
        anyhow::bail!("legacy image exceeds configured size limit");
    }
    let mime = tinyagents_harness::multimodal::mime::image_mime_from_magic(&bytes)
        .ok_or_else(|| anyhow::anyhow!("legacy sidecar is not a recognized image"))?;
    let name = source
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("image");
    Ok(Some(
        store_original(config, name, mime, &bytes, scope).await?,
    ))
}

async fn store_original(
    config: &Config,
    name: &str,
    mime: &str,
    bytes: &[u8],
    scope: &AttachmentAccessScope,
) -> anyhow::Result<String> {
    use sha2::{Digest, Sha256};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut hash = Sha256::new();
    hash.update(mime.as_bytes());
    hash.update([0]);
    hash.update(bytes);
    let relative = PathBuf::from("uploads")
        .join("legacy-sidecars")
        .join(format!("{:x}", hash.finalize()))
        .join(filename(name));
    let root = action_root(config, scope);
    policy(config, &root)
        .validate_parent_path(&root.join(&relative).to_string_lossy())
        .await
        .map_err(anyhow::Error::msg)?;
    tokio::fs::create_dir_all(&root).await?;
    let root = tokio::fs::canonicalize(root).await?;
    let mut parent = root.clone();
    for component in relative
        .parent()
        .expect("legacy upload parent")
        .components()
    {
        parent.push(component);
        match tokio::fs::create_dir(&parent).await {
            Ok(()) => (),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(error) => return Err(error.into()),
        }
        let metadata = tokio::fs::symlink_metadata(&parent).await?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            anyhow::bail!("legacy upload directory is not a regular directory");
        }
    }
    let path = policy(config, &root)
        .validate_parent_path(&root.join(&relative).to_string_lossy())
        .await
        .map_err(anyhow::Error::msg)?;
    let temp = path.with_file_name(format!(".sidecar-{}.tmp", uuid::Uuid::new_v4()));
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
        return Err(error.into());
    }
    let linked = tokio::fs::hard_link(&temp, &path).await;
    let _ = tokio::fs::remove_file(&temp).await;
    match linked {
        Ok(()) => (),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let metadata = tokio::fs::symlink_metadata(&path).await?;
            if !metadata.is_file()
                || metadata.file_type().is_symlink()
                || metadata.len() != bytes.len() as u64
            {
                anyhow::bail!("legacy original is not the expected regular file");
            }
            let checked = resolve_path(config, &path.to_string_lossy(), scope).await?;
            let file = tokio::fs::File::open(checked).await?;
            let mut existing = Vec::new();
            file.take((bytes.len() + 1) as u64)
                .read_to_end(&mut existing)
                .await?;
            if existing != bytes {
                anyhow::bail!("legacy original has changed");
            }
        }
        Err(error) => return Err(error.into()),
    }
    Ok(relative.to_string_lossy().replace('\\', "/"))
}
