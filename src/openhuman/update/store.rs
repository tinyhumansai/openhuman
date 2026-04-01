use std::path::{Path, PathBuf};

fn executable_suffix() -> &'static str {
    #[cfg(windows)]
    {
        ".exe"
    }
    #[cfg(not(windows))]
    {
        ""
    }
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut path_os = path.as_os_str().to_os_string();
    path_os.push(suffix);
    PathBuf::from(path_os)
}

pub fn managed_binary_path() -> Result<PathBuf, String> {
    if let Ok(path) = std::env::var("OPENHUMAN_UPDATE_MANAGED_BIN") {
        let path = PathBuf::from(path);
        if path.as_os_str().is_empty() {
            return Err("OPENHUMAN_UPDATE_MANAGED_BIN is empty".to_string());
        }
        return Ok(path);
    }
    std::env::current_exe().map_err(|e| format!("failed to resolve current executable path: {e}"))
}

pub fn staged_binary_path(target_bin: &Path) -> PathBuf {
    with_suffix(target_bin, ".next")
}

fn backup_binary_path(target_bin: &Path) -> PathBuf {
    with_suffix(target_bin, ".bak")
}

pub fn has_staged_update(target_bin: &Path) -> bool {
    staged_binary_path(target_bin).exists()
}

pub fn apply_staged_update_for_path(target_bin: &Path) -> Result<bool, String> {
    let staged = staged_binary_path(target_bin);
    if !staged.exists() {
        return Ok(false);
    }

    log::debug!(
        "[update] applying staged update: {} -> {}",
        staged.display(),
        target_bin.display()
    );

    let backup = backup_binary_path(target_bin);
    if backup.exists() {
        let _ = std::fs::remove_file(&backup);
    }

    std::fs::rename(target_bin, &backup).map_err(|e| {
        format!(
            "failed to move current binary to backup ({} -> {}): {e}",
            target_bin.display(),
            backup.display()
        )
    })?;

    if let Err(error) = std::fs::rename(&staged, target_bin) {
        let _ = std::fs::rename(&backup, target_bin);
        return Err(format!(
            "failed to activate staged binary ({} -> {}): {error}",
            staged.display(),
            target_bin.display()
        ));
    }

    let _ = std::fs::remove_file(&backup);
    log::debug!("[update] staged update activated at {}", target_bin.display());
    Ok(true)
}

pub fn write_staged_binary(target_bin: &Path, bytes: &[u8]) -> Result<PathBuf, String> {
    let parent = target_bin.parent().ok_or_else(|| {
        format!(
            "managed binary path has no parent: {}",
            target_bin.display()
        )
    })?;
    std::fs::create_dir_all(parent).map_err(|e| {
        format!(
            "failed to create update directory {}: {e}",
            parent.display()
        )
    })?;

    let tmp = parent.join(format!(
        ".openhuman-update-{}{}",
        uuid::Uuid::new_v4(),
        executable_suffix()
    ));

    std::fs::write(&tmp, bytes)
        .map_err(|e| format!("failed to write update payload {}: {e}", tmp.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)).map_err(|e| {
            format!(
                "failed to mark staged binary executable {}: {e}",
                tmp.display()
            )
        })?;
    }

    let staged = staged_binary_path(target_bin);
    if staged.exists() {
        std::fs::remove_file(&staged).map_err(|e| {
            format!(
                "failed to remove previous staged binary {}: {e}",
                staged.display()
            )
        })?;
    }

    std::fs::rename(&tmp, &staged)
        .map_err(|e| format!("failed to stage update {}: {e}", staged.display()))?;

    log::debug!("[update] binary staged at {}", staged.display());
    Ok(staged)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staged_swap_roundtrip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let bin = dir.path().join("openhuman-core");
        std::fs::write(&bin, b"old").expect("write old");

        let staged = staged_binary_path(&bin);
        std::fs::write(&staged, b"new").expect("write new");

        let applied = apply_staged_update_for_path(&bin).expect("apply staged update");
        assert!(applied);
        assert_eq!(std::fs::read(&bin).expect("read activated"), b"new");
        assert!(!staged.exists());
    }
}
